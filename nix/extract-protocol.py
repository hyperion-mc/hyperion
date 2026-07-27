#!/usr/bin/env python3
"""Extract a machine-readable Minecraft protocol description for hyperion.

Two independent sources are combined, deliberately:

  * ``generated/reports/packets.json`` from Mojang's own data generator is
    authoritative for *which* packets exist and what numeric id each one has,
    but it says nothing about wire layout.
  * The decompiled ``net.minecraft.network`` sources supply the layout. Since
    26.1 the server jar ships unobfuscated, so the decompiler emits real class,
    field and method names and the layout can be read straight off the source.

Where the two disagree the extractor fails loudly rather than guessing: a
silent mismatch here would produce a codec that looks right and corrupts the
stream at runtime.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# ---------------------------------------------------------------------------
# Wire-type vocabulary
# ---------------------------------------------------------------------------

# FriendlyByteBuf.writeX -> canonical wire type. Only first-order writers are
# listed; higher-order ones (writeCollection and friends) take an element
# writer lambda and are handled separately.
BUF_WRITERS: dict[str, str] = {
    "writeBoolean": "bool",
    "writeByte": "i8",
    "writeShort": "i16",
    "writeInt": "i32",
    "writeLong": "i64",
    "writeFloat": "f32",
    "writeDouble": "f64",
    "writeVarInt": "varint",
    "writeVarLong": "varlong",
    "writeUtf": "string",
    "writeUUID": "uuid",
    "writeBlockPos": "block_pos",
    "writeChunkPos": "chunk_pos",
    "writeIdentifier": "identifier",
    "writeContainerId": "varint",
    "writeInstant": "i64",
    "writeByteArray": "byte_array",
    "writeVarIntArray": "varint_array",
    "writeLongArray": "long_array",
    "writeNbt": "nbt",
    "writeEnum": "enum_varint",
    "writeEnumSet": "enum_set",
    "writeBlockHitResult": "block_hit_result",
    "writeIntIdList": "int_id_list",
    "writeById": "registry_id",
}

# Higher-order writers: the payload shape depends on a lambda argument, so the
# element type is recorded as "unresolved" rather than invented.
BUF_HIGHER_ORDER: dict[str, str] = {
    "writeCollection": "list",
    "writeNullable": "option",
    "writeOptional": "option",
    "writeMap": "map",
}

# ByteBufCodecs static fields -> canonical wire type.
CODEC_FIELDS: dict[str, str] = {
    "BOOL": "bool",
    "BYTE": "i8",
    "SHORT": "i16",
    "UNSIGNED_SHORT": "u16",
    "INT": "i32",
    "LONG": "i64",
    "FLOAT": "f32",
    "DOUBLE": "f64",
    "VAR_INT": "varint",
    "VAR_LONG": "varlong",
    "STRING_UTF8": "string",
    "BYTE_ARRAY": "byte_array",
    "CONTAINER_ID": "varint",
    "OPTIONAL_VAR_INT": "optional_varint",
    "TRUSTED_COMPOUND_TAG": "nbt",
    "COMPOUND_TAG": "nbt",
    "GAME_PROFILE": "game_profile",
    "VECTOR3F": "vec3f",
    "QUATERNIONF": "quaternionf",
}

# Domain codecs that hyperion needs a hand-written implementation for. Recorded
# by name so the report can quantify exactly how much is *not* mechanical.
DOMAIN_CODEC_RE = re.compile(r"\b([A-Z][A-Za-z0-9_]*(?:\.[A-Z][A-Za-z0-9_]*)?)\.([A-Z_]*STREAM_CODEC[A-Z_]*)\b")


@dataclass
class Field:
    name: str
    wire: str
    java_type: str | None = None
    note: str | None = None


@dataclass
class Packet:
    resource: str
    state: str
    direction: str
    protocol_id: int
    java_class: str | None = None
    layout_source: str = "unknown"
    fields: list[Field] = field(default_factory=list)
    domain_codecs: list[str] = field(default_factory=list)
    complete: bool = False
    reason: str | None = None


# ---------------------------------------------------------------------------
# Java source scanning
# ---------------------------------------------------------------------------

def strip_comments(src: str) -> str:
    src = re.sub(r"/\*.*?\*/", "", src, flags=re.S)
    src = re.sub(r"//[^\n]*", "", src)
    return src


def load_sources(decompiled: Path) -> dict[str, str]:
    """Map simple class name -> decompiled source text."""
    out: dict[str, str] = {}
    for p in decompiled.rglob("*.java"):
        out[p.stem] = strip_comments(p.read_text(encoding="utf-8", errors="replace"))
    return out


PACKET_TYPE_DECL = re.compile(
    r"PacketType<(?P<cls>[\w.$]+)>\s+(?P<const>[A-Z0-9_]+)\s*=\s*"
    r"\w+\.create(?P<flow>Clientbound|Serverbound)\(\s*\"(?P<id>[a-z0-9_/]+)\"",
)


def scan_packet_types(sources: dict[str, str]) -> dict[str, tuple[str, str, str]]:
    """const name -> (flow, resource id, java class), from the *PacketTypes classes."""
    table: dict[str, tuple[str, str, str]] = {}
    for name, src in sources.items():
        if not name.endswith("PacketTypes"):
            continue
        for m in PACKET_TYPE_DECL.finditer(src):
            flow = m.group("flow").lower()
            table[f"{name}.{m.group('const')}"] = (
                flow,
                f"minecraft:{m.group('id')}",
                m.group("cls").split(".")[-1],
            )
    return table


# The bundle delimiter is registered with withBundlePacket rather than
# addPacket, but it still consumes protocol id 0 of play/clientbound, so both
# spellings have to count towards the ordinal.
ADD_PACKET = re.compile(r"\.(?:addPacket|withBundlePacket)\(\s*([\w.$]+)\s*,")


def scan_protocol_order(sources: dict[str, str]) -> dict[str, list[str]]:
    """*Protocols class -> ordered list of PacketTypes constant references.

    The registration order is the numeric protocol id order, which is how the
    ids in packets.json are assigned. Recovering it gives an independent check
    on the ids rather than trusting a single source.
    """
    out: dict[str, list[str]] = {}
    for name, src in sources.items():
        if not name.endswith("Protocols"):
            continue
        for decl in re.finditer(
            r"(?P<var>[A-Z0-9_]+)_TEMPLATE\s*=\s*ProtocolInfoBuilder\.(?P<flow>\w+)Protocol\("
            r"ConnectionProtocol\.(?P<state>\w+)\s*,(?P<body>.*?);\n",
            src,
            flags=re.S,
        ):
            key = f"{decl.group('state').lower()}/{decl.group('flow').lower()}"
            out.setdefault(key, []).extend(m.group(1) for m in ADD_PACKET.finditer(decl.group("body")))
    return out


# ---------------------------------------------------------------------------
# Layout recovery
# ---------------------------------------------------------------------------

RECORD_HEADER = re.compile(r"\brecord\s+(?P<cls>\w+)\s*\((?P<params>[^)]*)\)")
COMPOSITE = re.compile(r"StreamCodec\.composite\((?P<args>.*?)\);", re.S)
# Some packets (the ping ones, and anything whose codec is declared over
# StreamCodec<ByteBuf, ...>) write through a bare netty ByteBuf rather than
# FriendlyByteBuf. The primitive method names coincide, so both are accepted.
WRITE_METHOD = re.compile(
    r"\bvoid\s+write\((?:Registry)?(?:Friendly)?ByteBuf\s+(?P<buf>\w+)\)\s*\{(?P<body>.*?)\n    \}",
    re.S,
)
UNIT_CODEC = re.compile(r"StreamCodec\.unit\(")


def split_top_level(text: str) -> list[str]:
    """Split a Java argument list on commas that are not nested in ()/<>."""
    parts, depth, cur = [], 0, []
    for ch in text:
        if ch in "(<[":
            depth += 1
        elif ch in ")>]":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
    if "".join(cur).strip():
        parts.append("".join(cur).strip())
    return parts


def record_params(src: str, cls: str) -> list[tuple[str, str]]:
    for m in RECORD_HEADER.finditer(src):
        if m.group("cls") == cls:
            out = []
            for p in split_top_level(m.group("params")):
                p = re.sub(r"@\w+(\([^)]*\))?\s*", "", p).strip()
                if not p:
                    continue
                java_type, _, pname = p.rpartition(" ")
                out.append((pname.strip(), java_type.strip()))
            return out
    return []


# Domain codecs whose wire form was read out of the decompiled source by hand
# and is a plain primitive. Resolving them here is what keeps a packet like
# login_finished from being reported as needing work it does not need. Anything
# not on this list stays "domain:" so the coverage number never flatters itself.
VERIFIED_DOMAIN_CODECS: dict[str, str] = {
    # net/minecraft/core/UUIDUtil: delegates straight to FriendlyByteBuf.writeUUID,
    # i.e. two big-endian longs, most significant first.
    "UUIDUtil.STREAM_CODEC": "uuid",
}

JAVA_CONSTANTS: dict[str, int] = {
    "Short.MAX_VALUE": 32767,
    "Integer.MAX_VALUE": 2147483647,
}


def parse_int_literal(text: str) -> int | None:
    text = text.strip()
    if text in JAVA_CONSTANTS:
        return JAVA_CONSTANTS[text]
    try:
        return int(text, 0)
    except ValueError:
        return None


def classify_codec_arg(arg: str) -> tuple[str, str | None]:
    """Return (wire type, domain codec name if the layout is not primitive)."""
    m = re.search(r"ByteBufCodecs\.(?P<f>[A-Z_]+)\b", arg)
    if m and m.group("f") in CODEC_FIELDS:
        base = CODEC_FIELDS[m.group("f")]
        return (wrap_combinators(base, arg), None)
    m = re.search(r"ByteBufCodecs\.stringUtf8\(\s*([^)]+)\)", arg)
    if m:
        limit = parse_int_literal(m.group(1))
        return (wrap_combinators(f"string(max={limit})", arg), None)
    # lenientJson is a length-limited UTF-8 string on the wire; the DFU codec
    # threaded through .apply(fromCodec(..)) only shapes the in-memory value.
    m = re.search(r"ByteBufCodecs\.lenientJson\(\s*([^)]+)\)", arg)
    if m:
        limit = parse_int_literal(m.group(1))
        return (wrap_combinators(f"json_string(max={limit})", arg), None)
    dm = DOMAIN_CODEC_RE.search(arg)
    if dm:
        name = f"{dm.group(1)}.{dm.group(2)}"
        if name in VERIFIED_DOMAIN_CODECS:
            return (wrap_combinators(VERIFIED_DOMAIN_CODECS[name], arg), None)
        return (wrap_combinators(f"domain:{name}", arg), name)
    return ("unresolved", None)


def wrap_combinators(base: str, arg: str) -> str:
    if "ByteBufCodecs::optional" in arg or re.search(r"\boptional\b", arg):
        base = f"option<{base}>"
    if "ByteBufCodecs.list" in arg or ".apply(ByteBufCodecs::list" in arg:
        base = f"list<{base}>"
    return base


def layout_from_composite(src: str, cls: str) -> tuple[list[Field], list[str]] | None:
    m = COMPOSITE.search(src)
    if not m:
        return None
    args = split_top_level(m.group("args"))
    if len(args) < 3 or len(args) % 2 == 0:
        return None
    params = dict(record_params(src, cls))
    fields: list[Field] = []
    domain: list[str] = []
    # Pairs of (codec, getter), then a trailing constructor reference.
    for codec_arg, getter in zip(args[0:-1:2], args[1:-1:2]):
        gm = re.search(r"::(\w+)$", getter.strip())
        fname = gm.group(1) if gm else getter.strip()
        wire, dom = classify_codec_arg(codec_arg)
        if dom:
            domain.append(dom)
        fields.append(Field(name=fname, wire=wire, java_type=params.get(fname)))
    return fields, domain


def layout_from_write(src: str) -> tuple[list[Field], list[str]] | None:
    """Read the wire layout off a linear ``write`` method.

    Every statement in the body has to be accounted for. A statement that is
    not a recognised buffer call -- ``this.payload.write(output)`` in the login
    custom-query packets, say -- means part of the layout lives somewhere this
    parser cannot see, and returning the fields recovered so far would emit a
    codec that is silently short. Those packets are surfaced as unrecovered
    instead.
    """
    m = WRITE_METHOD.search(src)
    if not m:
        return None
    buf, body = m.group("buf"), m.group("body")
    if re.search(r"\b(if|for|while|switch|return)\b", body):
        return None

    fields: list[Field] = []
    call = re.compile(rf"{re.escape(buf)}\.(\w+)\(([^;]*?)\)\s*;", re.S)
    consumed_spans: list[tuple[int, int]] = []
    for cm in call.finditer(body):
        consumed_spans.append(cm.span())
        method, args = cm.group(1), cm.group(2)
        fname_m = re.search(r"this\.(\w+)", args)
        fname = fname_m.group(1) if fname_m else method
        if method in BUF_WRITERS:
            fields.append(Field(name=fname, wire=BUF_WRITERS[method]))
        elif method in BUF_HIGHER_ORDER:
            fields.append(
                Field(
                    name=fname,
                    wire=f"{BUF_HIGHER_ORDER[method]}<unresolved>",
                    note="element writer is a lambda; needs manual resolution",
                )
            )
        else:
            return None

    # Anything left after removing the recognised calls is an unmodelled write.
    leftover = body
    for start, end in reversed(consumed_spans):
        leftover = leftover[:start] + leftover[end:]
    if re.search(r"\S", re.sub(r"[{}\s]", "", leftover)):
        return None

    return (fields, []) if fields else None


def recover_layout(pkt: Packet, sources: dict[str, str]) -> None:
    src = sources.get(pkt.java_class or "")
    if src is None:
        pkt.reason = "decompiled source not found"
        return
    got = layout_from_composite(src, pkt.java_class or "")
    if got is not None:
        pkt.fields, pkt.domain_codecs = got
        pkt.layout_source = "composite"
    else:
        got = layout_from_write(src)
        if got is not None:
            pkt.fields, pkt.domain_codecs = got
            pkt.layout_source = "write_method"
        elif UNIT_CODEC.search(src):
            pkt.layout_source = "unit"
        else:
            pkt.reason = "no linear codec found (branching write, or custom StreamCodec)"
            return
    pkt.complete = all(
        "unresolved" not in f.wire and not f.wire.startswith("domain:") for f in pkt.fields
    )
    if not pkt.complete and pkt.reason is None:
        pkt.reason = "layout references a domain codec or an unresolved lambda"


# ---------------------------------------------------------------------------

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--generated", required=True, type=Path, help="vanilla generated/ tree")
    ap.add_argument("--decompiled", required=True, type=Path, help="decompiled java tree")
    ap.add_argument("--version-json", required=True, type=Path, help="version.json from the jar")
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()

    reports = args.generated / "reports"
    packets_json = json.loads((reports / "packets.json").read_text())
    version = json.loads(args.version_json.read_text())

    sources = load_sources(args.decompiled)
    types = scan_packet_types(sources)
    order = scan_protocol_order(sources)

    # resource id + flow -> java class
    by_resource: dict[tuple[str, str], str] = {}
    const_to_resource: dict[str, str] = {}
    for const, (flow, resource, cls) in types.items():
        by_resource[(resource, flow)] = cls
        const_to_resource[const.split(".")[-1]] = resource

    packets: list[Packet] = []
    for state, dirs in sorted(packets_json.items()):
        for direction, entries in sorted(dirs.items()):
            for resource, meta in sorted(entries.items()):
                p = Packet(
                    resource=resource,
                    state=state,
                    direction=direction,
                    protocol_id=meta["protocol_id"],
                    java_class=by_resource.get((resource, direction)),
                )
                if p.java_class:
                    recover_layout(p, sources)
                else:
                    p.reason = "no PacketType declaration matched"
                packets.append(p)

    # Cross-check ids against registration order recovered from *Protocols.
    mismatches: list[str] = []
    for key, consts in order.items():
        state, flow = key.split("/")
        state = {"handshaking": "handshake", "play": "play", "configuration": "configuration",
                 "status": "status", "login": "login"}.get(state, state)
        for idx, const in enumerate(consts):
            resource = const_to_resource.get(const.split(".")[-1])
            if resource is None:
                continue
            match = [p for p in packets if p.resource == resource and p.state == state and p.direction == flow]
            if match and match[0].protocol_id != idx:
                mismatches.append(f"{state}/{flow} {resource}: reports={match[0].protocol_id} source_order={idx}")

    complete = [p for p in packets if p.complete]
    partial = [p for p in packets if not p.complete and p.layout_source != "unknown"]
    unknown = [p for p in packets if p.layout_source == "unknown"]
    all_domain = sorted({d for p in packets for d in p.domain_codecs})

    doc = {
        "version": {
            "id": version["id"],
            "protocolVersion": version["protocol_version"],
            "worldVersion": version["world_version"],
            "releaseTime": version.get("build_time"),
        },
        "coverage": {
            "packets": len(packets),
            "fullyMechanical": len(complete),
            "partial": len(partial),
            "unrecovered": len(unknown),
            "distinctDomainCodecs": len(all_domain),
            "idCrossCheckMismatches": mismatches,
        },
        "domainCodecs": all_domain,
        "packets": [
            {
                "resource": p.resource,
                "state": p.state,
                "direction": p.direction,
                "protocolId": p.protocol_id,
                "javaClass": p.java_class,
                "layoutSource": p.layout_source,
                "complete": p.complete,
                "reason": p.reason,
                "fields": [
                    {k: v for k, v in
                     (("name", f.name), ("wire", f.wire), ("javaType", f.java_type), ("note", f.note))
                     if v is not None}
                    for f in p.fields
                ],
            }
            for p in packets
        ],
        "registries": {
            name: {
                "protocolId": body.get("protocol_id"),
                "entries": sorted(body.get("entries", {}), key=lambda k: body["entries"][k]["protocol_id"]),
            }
            for name, body in sorted(json.loads((reports / "registries.json").read_text()).items())
        },
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(doc, indent=2, sort_keys=False) + "\n")

    cov = doc["coverage"]
    print(f"protocol {doc['version']['protocolVersion']} ({doc['version']['id']})", file=sys.stderr)
    print(f"  packets:            {cov['packets']}", file=sys.stderr)
    print(f"  fully mechanical:   {cov['fullyMechanical']}", file=sys.stderr)
    print(f"  partial:            {cov['partial']}", file=sys.stderr)
    print(f"  unrecovered:        {cov['unrecovered']}", file=sys.stderr)
    print(f"  domain codecs:      {cov['distinctDomainCodecs']}", file=sys.stderr)
    print(f"  registries:         {len(doc['registries'])}", file=sys.stderr)
    if mismatches:
        print(f"  ID CROSS-CHECK FAILED: {len(mismatches)}", file=sys.stderr)
        for m in mismatches[:10]:
            print(f"    {m}", file=sys.stderr)
        return 1
    print("  id cross-check:     ok", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
