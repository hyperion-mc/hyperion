"""Extract a machine-readable Minecraft protocol description for hyperion.

Three independent sources are combined, deliberately:

  * ``generated/reports/packets.json`` from Mojang's own data generator is
    authoritative for *which* packets exist and what numeric id each one has,
    but it says nothing about wire layout.
  * ``generated/reports/registries.json`` is authoritative for registry
    contents, including the 111 data component types an ``ItemStack`` can
    carry.
  * The decompiled sources supply the layout. Since 26.1 the server jar ships
    unobfuscated, so the decompiler emits real class, field and method names
    and the layout can be read straight off the source.

Where two sources disagree the extractor fails loudly rather than guessing: a
silent mismatch here would produce a codec that looks right and corrupts the
stream at runtime.

The same rule governs layout recovery. Every statement of an ``encode`` body
and every argument of a ``StreamCodec.composite`` has to be accounted for by
the modelled vocabulary. Anything unmodelled makes the whole layout
``unresolved`` -- it is never dropped so that the rest can be reported as
complete. A packet reported complete is one whose byte sequence is known in
full.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# ---------------------------------------------------------------------------
# Wire types
#
# A wire type is a JSON object with a "kind". Composite kinds nest. Named
# struct layouts are hoisted into a shared table and referenced by key, which
# keeps the emitted document small and makes sharing visible.
# ---------------------------------------------------------------------------

Wire = dict[str, Any]


def prim(kind: str, **extra: Any) -> Wire:
    out: Wire = {"kind": kind}
    out.update({k: v for k, v in extra.items() if v is not None})
    return out


def unresolved(why: str) -> Wire:
    return {"kind": "unresolved", "why": why}


def is_resolved(wire: Wire, types: dict[str, Wire], seen: frozenset[str] = frozenset()) -> bool:
    """True when nothing anywhere in the type is unresolved.

    Recursive named types (a codec that mentions itself) count as resolved:
    the recursion is in the shape, not in our knowledge of it.
    """
    kind = wire["kind"]
    if kind == "unresolved":
        return False
    if kind == "named":
        ref = wire["ref"]
        if ref in seen:
            return True
        target = types.get(ref)
        if target is None:
            return False
        return is_resolved(target, types, seen | {ref})
    for key in ("of", "key", "value", "left", "right"):
        if key in wire and not is_resolved(wire[key], types, seen):
            return False
    for member in wire.get("fields", []):
        if not is_resolved(member["wire"], types, seen):
            return False
    for variant in wire.get("variants", []):
        if not is_resolved(variant["wire"], types, seen):
            return False
    return True


def unresolved_reasons(wire: Wire, types: dict[str, Wire], seen: frozenset[str] = frozenset()) -> list[str]:
    """Every distinct reason the type is not fully known, for the report."""
    kind = wire["kind"]
    if kind == "unresolved":
        return [wire["why"]]
    if kind == "named":
        ref = wire["ref"]
        if ref in seen:
            return []
        target = types.get(ref)
        if target is None:
            return [f"missing type {ref}"]
        return unresolved_reasons(target, types, seen | {ref})
    out: list[str] = []
    for key in ("of", "key", "value", "left", "right"):
        if key in wire:
            out.extend(unresolved_reasons(wire[key], types, seen))
    for member in wire.get("fields", []):
        out.extend(unresolved_reasons(member["wire"], types, seen))
    for variant in wire.get("variants", []):
        out.extend(unresolved_reasons(variant["wire"], types, seen))
    return out


# ---------------------------------------------------------------------------
# Java source text handling
# ---------------------------------------------------------------------------

# Every scan below runs on text whose string and character literals have been
# blanked and whose comments have been removed, so a brace or semicolon inside
# a literal can never be mistaken for structure. Literal *contents* are blanked
# in place rather than deleted, which keeps the interesting cases (the max
# lengths and resource names that are numeric or identifier-shaped) available
# through a separate lookup on the raw text where needed.
_STRING_LITERAL = re.compile(r'"(?:\\.|[^"\\])*"')
_CHAR_LITERAL = re.compile(r"'(?:\\.|[^'\\])'")


def strip_java(src: str) -> str:
    def blank(m: re.Match[str]) -> str:
        return m.group(0)[0] + " " * (len(m.group(0)) - 2) + m.group(0)[-1]

    src = _STRING_LITERAL.sub(blank, src)
    src = _CHAR_LITERAL.sub(blank, src)
    src = re.sub(r"/\*.*?\*/", lambda m: " " * len(m.group(0)), src, flags=re.S)
    src = re.sub(r"//[^\n]*", lambda m: " " * len(m.group(0)), src)
    return src


# Two-character operators that contain an angle bracket. Masking them keeps the
# depth scanner from treating a lambda arrow or a shift as a generic bracket,
# which would drive the depth negative and silently merge argument lists.
_ANGLE_OPS = ("->", "<=", ">=", "<<", ">>", "<>")


def _mask_angle_ops(text: str) -> str:
    for op in _ANGLE_OPS:
        text = text.replace(op, "\x00" * len(op))
    return text


def split_top_level(text: str) -> list[str]:
    """Split a Java argument list on commas not nested in ()/<>/[]/{}."""
    masked = _mask_angle_ops(text)
    parts: list[str] = []
    depth = 0
    start = 0
    for i, ch in enumerate(masked):
        if ch in "(<[{":
            depth += 1
        elif ch in ")>]}":
            depth -= 1
        elif ch == "," and depth == 0:
            parts.append(text[start:i].strip())
            start = i + 1
    tail = text[start:].strip()
    if tail:
        parts.append(tail)
    return [p for p in parts if p]


def match_bracket(text: str, open_index: int) -> int:
    """Index just past the bracket that closes the one at ``open_index``."""
    pairs = {"(": ")", "{": "}", "[": "]", "<": ">"}
    opener = text[open_index]
    closer = pairs[opener]
    masked = _mask_angle_ops(text)
    depth = 0
    for i in range(open_index, len(masked)):
        if masked[i] == opener:
            depth += 1
        elif masked[i] == closer:
            depth -= 1
            if depth == 0:
                return i + 1
    raise ValueError(f"unbalanced {opener} at {open_index}")


def statements(body: str) -> list[str]:
    """Split a class or method body into top-level statements.

    A statement ends at a semicolon at depth zero, or at the brace that closes
    a block back to depth zero. The second rule is what separates a method or a
    static initializer from whatever follows it: those carry no trailing
    semicolon, so splitting on semicolons alone glues a static block onto the
    method before it and the block's contents are never seen.
    """
    masked = _mask_angle_ops(body)
    out: list[str] = []
    depth = 0
    start = 0
    for i, ch in enumerate(masked):
        if ch in "({[":
            depth += 1
        elif ch in ")}]":
            depth -= 1
            if depth == 0 and ch == "}":
                out.append(body[start : i + 1].strip())
                start = i + 1
        elif ch == ";" and depth == 0:
            out.append(body[start:i].strip())
            start = i + 1
    tail = body[start:].strip()
    if tail:
        out.append(tail)
    return [s for s in out if s and s != ";"]


def split_owner(owner: str) -> tuple[str, str | None]:
    """``pkg.Outer$Nested`` -> ``("pkg.Outer", "Nested")``."""
    fqn, _, nested = owner.partition("$")
    return fqn, nested or None


def join_owner(fqn: str, nested: str | None) -> str:
    return f"{fqn}${nested}" if nested else fqn


def _enclosing_owner(owner: str) -> str | None:
    """The class one level out, or None at the top level."""
    fqn, nested = split_owner(owner)
    if nested is None:
        return None
    outer = nested.rpartition(".")[0]
    return join_owner(fqn, outer or None)


@dataclass
class JavaFile:
    fqn: str
    package: str
    simple: str
    # Structure-safe text: literals blanked, comments removed, both in place so
    # that ``text`` and ``raw`` stay the same length and share offsets.
    text: str
    raw: str
    imports: dict[str, str] = field(default_factory=dict)


class SourceIndex:
    """Fully-qualified index over the decompiled tree.

    Keying on the fully-qualified name rather than the simple one matters as
    soon as the decompile scope is wider than a single package: ``Action``,
    ``Input`` and ``Data`` all name several unrelated classes, and picking one
    by luck would attach the wrong layout to a packet.
    """

    def __init__(self, root: Path) -> None:
        self.files: dict[str, JavaFile] = {}
        self.by_simple: dict[str, list[str]] = {}
        for path in sorted(root.rglob("*.java")):
            if path.stem == "package-info":
                continue
            raw = path.read_text(encoding="utf-8", errors="replace")
            text = strip_java(raw)
            assert len(text) == len(raw), f"strip_java resized {path}"
            pm = re.search(r"^package\s+([\w.]+)\s*;", text, flags=re.M)
            package = pm.group(1) if pm else ""
            simple = path.stem
            fqn = f"{package}.{simple}" if package else simple
            jf = JavaFile(fqn=fqn, package=package, simple=simple, text=text, raw=raw)
            for im in re.finditer(r"^import\s+(?:static\s+)?([\w.]+)\s*;", text, flags=re.M):
                target = im.group(1)
                jf.imports[target.rsplit(".", 1)[-1]] = target
            self.files[fqn] = jf
            self.by_simple.setdefault(simple, []).append(fqn)

    def resolve(self, name: str, origin: str | None) -> str | None:
        """Resolve a possibly-nested type name to an owner string.

        An owner is ``package.Outer`` or ``package.Outer$Nested.Deeper``: the
        decompiler emits nested classes inside the file of their outermost
        enclosing class, so the file and the path within it both matter.
        """
        head, _, rest = name.partition(".")
        nested = rest or None
        origin_file = self.files.get(split_owner(origin)[0]) if origin else None

        if origin_file is not None:
            target = origin_file.imports.get(head)
            if target and target in self.files:
                return join_owner(target, nested)
            # A nested class of the origin itself is written unqualified.
            if re.search(rf"\b(?:class|interface|enum|record)\s+{re.escape(head)}\b", origin_file.text):
                if head == origin_file.simple:
                    return join_owner(origin_file.fqn, nested)
                return join_owner(origin_file.fqn, ".".join(filter(None, [head, nested])))
            same_package = f"{origin_file.package}.{head}"
            if same_package in self.files:
                return join_owner(same_package, nested)

        candidates = self.by_simple.get(head, [])
        if len(candidates) == 1:
            return join_owner(candidates[0], nested)
        return None

    def body_span(self, owner: str) -> tuple[JavaFile, int, int] | None:
        """Offsets of a class body, descending through nested classes.

        Offsets rather than a slice, because ``text`` and ``raw`` are the same
        length: a caller that needs a string literal's contents reads the same
        range out of ``raw``.
        """
        fqn, nested = split_owner(owner)
        jf = self.files.get(fqn)
        if jf is None:
            return None
        start, end = 0, len(jf.text)
        for name in [jf.simple] + (nested.split(".") if nested else []):
            found = None
            pattern = rf"\b(?:class|interface|enum|record)\s+{re.escape(name)}\b"
            for m in re.finditer(pattern, jf.text[start:end]):
                brace = jf.text.find("{", start + m.end())
                if brace == -1 or brace >= end:
                    continue
                found = (brace + 1, match_bracket(jf.text, brace) - 1)
                break
            if found is None:
                return None
            start, end = found
        return jf, start, end

    def body_of(self, owner: str) -> str | None:
        """Source text of a class body, descending through nested classes."""
        span = self.body_span(owner)
        return None if span is None else span[0].text[span[1] : span[2]]

    def raw_body_of(self, owner: str) -> str | None:
        """As body_of, but with string literals intact."""
        span = self.body_span(owner)
        return None if span is None else span[0].raw[span[1] : span[2]]


# ---------------------------------------------------------------------------
# The wire vocabulary
#
# These three tables are the only place a Java name is mapped to a byte layout
# by hand. Every one of them is asserted to still exist in the decompiled
# source before extraction starts, so a rename or removal in a future version
# breaks the build loudly instead of silently mis-modelling a field.
# ---------------------------------------------------------------------------

# The one ByteBufCodecs field whose layout is asserted rather than derived: its
# encoder is a loop, which is not a layout the expression reader can follow.
# Transcribed from ByteBufCodecs.GAME_PROFILE_PROPERTIES in the 26.2 source and
# only used if the derived value comes up short, so if a future version makes
# it readable the derived one silently wins.
#
# Every other ByteBufCodecs field is derived. Keeping the table this small is
# deliberate: the hand-written entry for RGB_COLOR said i32 when the source
# writes three bytes, and only preferring the derived value caught it.
CODEC_FIELDS: dict[str, Wire] = {
    "GAME_PROFILE_PROPERTIES": {
        "kind": "list",
        "max": 16,
        "of": {
            "kind": "struct",
            "name": "ProfileProperty",
            "fields": [
                {"name": "name", "wire": prim("string", max=64)},
                {"name": "value", "wire": prim("string", max=32767)},
                {"name": "signature", "wire": {"kind": "option", "of": prim("string", max=1024)}},
            ],
        },
    },
}

# FriendlyByteBuf instance writers whose byte layout is fixed and first-order.
# Higher-order writers (the ones taking an element encoder) and the ones with
# an argument-dependent layout are handled in read_encode_body instead.
BUF_WRITERS: dict[str, Wire] = {
    "writeBoolean": prim("bool"),
    "writeByte": prim("i8"),
    "writeShort": prim("i16"),
    "writeShortLE": prim("i16", endian="little"),
    "writeChar": prim("u16"),
    "writeMedium": prim("i24"),
    "writeInt": prim("i32"),
    "writeIntLE": prim("i32", endian="little"),
    "writeLong": prim("i64"),
    "writeLongLE": prim("i64", endian="little"),
    "writeFloat": prim("f32"),
    "writeDouble": prim("f64"),
    "writeVarInt": prim("varint"),
    "writeVarLong": prim("varlong"),
    "writeUtf": prim("string", max=32767),
    "writeUUID": prim("uuid", note="two big-endian longs, most significant first"),
    "writeBlockPos": prim("block_pos", note="one i64, x/z/y packed 26/26/12"),
    "writeChunkPos": prim("chunk_pos", note="one i64, x and z as two i32"),
    "writeIdentifier": prim("identifier", note="namespaced name written as a utf string"),
    "writeResourceKey": prim("identifier"),
    "writeGlobalPos": {
        "kind": "struct",
        "name": "GlobalPos",
        "fields": [
            {"name": "dimension", "wire": prim("identifier")},
            {"name": "pos", "wire": prim("block_pos")},
        ],
    },
    "writeContainerId": prim("varint"),
    "writeInstant": prim("i64", note="epoch milliseconds"),
    "writeByteArray": prim("byte_array", note="varint length, then that many bytes"),
    "writeVarIntArray": prim("varint_array"),
    "writeLongArray": prim("long_array", note="varint length, then that many i64"),
    "writeFixedSizeLongArray": prim("fixed_long_array", note="no length prefix"),
    "writeIntIdList": {"kind": "list", "of": prim("varint")},
    "writeBitSet": prim("long_array"),
    "writeNbt": prim("optional_nbt", note="TAG_End means absent"),
    "writeEnum": prim("varint", note="enum ordinal"),
    "writeBlockHitResult": prim("block_hit_result"),
    "writeVector3f": prim("vec3f"),
    "writeQuaternion": prim("quaternionf"),
    "writeById": prim("varint", note="index produced by a ToIntFunction"),
}

# Static helpers that write through a buffer passed as the first argument; a
# netty-level codec such as BlockPos.STREAM_CODEC reaches for these because it
# has a bare ByteBuf rather than a FriendlyByteBuf.
STATIC_WRITERS: dict[str, Wire] = {
    "VarInt.write": prim("varint"),
    "VarLong.write": prim("varlong"),
    "ByteBufCodecs.writeCount": prim("varint", note="element count, checked against a maximum"),
    "FriendlyByteBuf.writeBlockPos": BUF_WRITERS["writeBlockPos"],
    "FriendlyByteBuf.writeChunkPos": BUF_WRITERS["writeChunkPos"],
    "FriendlyByteBuf.writeByteArray": BUF_WRITERS["writeByteArray"],
    "FriendlyByteBuf.writeContainerId": BUF_WRITERS["writeContainerId"],
    "FriendlyByteBuf.writeLongArray": BUF_WRITERS["writeLongArray"],
    "FriendlyByteBuf.writeFixedSizeLongArray": BUF_WRITERS["writeFixedSizeLongArray"],
    "FriendlyByteBuf.writeNbt": BUF_WRITERS["writeNbt"],
    "FriendlyByteBuf.writeUUID": BUF_WRITERS["writeUUID"],
    "FriendlyByteBuf.writeVector3f": BUF_WRITERS["writeVector3f"],
    "FriendlyByteBuf.writeQuaternion": BUF_WRITERS["writeQuaternion"],
}

# Higher-order instance writers: name -> (element encoder argument indices,
# constructor for the resulting wire type).
BUF_COLLECTION_WRITERS: dict[str, str] = {
    "writeCollection": "list",
    "writeNullable": "option",
    "writeOptional": "option",
}

# Where each modelled name has to still be found, as an (owner, member) pair.
# Checked before extraction so that a Mojang rename is a build failure rather
# than a field that silently stops being modelled.
HAND_MODELLED: dict[str, Wire] = {
    f"net.minecraft.network.codec.ByteBufCodecs#{name}": wire for name, wire in CODEC_FIELDS.items()
}

VOCABULARY_ANCHORS: list[tuple[str, str]] = (
    [("ByteBufCodecs", name) for name in CODEC_FIELDS]
    + [("FriendlyByteBuf", name) for name in BUF_WRITERS]
    + [("FriendlyByteBuf", name) for name in BUF_COLLECTION_WRITERS]
    + [tuple(name.split(".", 1)) for name in STATIC_WRITERS]  # type: ignore[misc]
)

BYTE_BUF_CODECS_FQN = "net.minecraft.network.codec.ByteBufCodecs"

_ANCHOR_FILES = {
    "VarInt": "net.minecraft.network.VarInt",
    "VarLong": "net.minecraft.network.VarLong",
    "FriendlyByteBuf": "net.minecraft.network.FriendlyByteBuf",
    "ByteBufCodecs": "net.minecraft.network.codec.ByteBufCodecs",
}


def check_vocabulary(index: SourceIndex) -> list[str]:
    """Confirm every hand-modelled Java name still exists in the source.

    The tables above are the one place a byte layout is asserted rather than
    derived. If Mojang renames or drops a writer the derived layouts would
    silently stop covering it, so the pipeline refuses to run instead.
    """
    missing: list[str] = []
    for owner, member in VOCABULARY_ANCHORS:
        fqn = _ANCHOR_FILES[owner]
        jf = index.files.get(fqn)
        if jf is None:
            missing.append(f"{fqn} (source not decompiled)")
            continue
        if not re.search(rf"\b{re.escape(member)}\s*[(=]", jf.text):
            missing.append(f"{fqn}.{member}")
    return missing


JAVA_CONSTANTS: dict[str, int] = {
    "Short.MAX_VALUE": 32767,
    "Integer.MAX_VALUE": 2147483647,
    "Byte.MAX_VALUE": 127,
}


def parse_int_literal(text: str) -> int | None:
    text = text.strip()
    if text in JAVA_CONSTANTS:
        return JAVA_CONSTANTS[text]
    try:
        return int(text.rstrip("Ll"), 0)
    except ValueError:
        return None


# ---------------------------------------------------------------------------
# Codec expression evaluation
# ---------------------------------------------------------------------------

# How far back from a class body to look for the record header that declares
# its components. Generous enough for the longest packet record in 26.2.
RECORD_HEADER_WINDOW = 2000

_CAST = re.compile(r"^\(\s*[A-Za-z_][\w.]*(?:<[^()]*>)?\s*\)\s*(?=[A-Za-z_(])")
_NEW_ANON = re.compile(r"^new\s+StreamCodec\s*<")
_LAMBDA_HEAD = re.compile(r"^\(?\s*[\w\s,]*\)?\s*->")


def strip_wrappers(expr: str) -> str:
    """Remove redundant parentheses and leading casts."""
    expr = expr.strip()
    while True:
        if expr.startswith("(") and match_bracket(expr, 0) == len(expr):
            expr = expr[1:-1].strip()
            continue
        stripped = _CAST.sub("", expr)
        if stripped != expr:
            expr = stripped.strip()
            continue
        return expr


def split_chain(expr: str) -> list[str]:
    """Split ``a.b(c).d(e)`` into ``["a", "b(c)", "d(e)"]`` at depth zero.

    Method references (``Foo::bar``) and the dots inside a lambda body stay
    inside their segment because they are always nested in brackets by then.
    """
    masked = _mask_angle_ops(expr)
    parts: list[str] = []
    depth = 0
    start = 0
    for i, ch in enumerate(masked):
        if ch in "({[<":
            depth += 1
        elif ch in ")}]>":
            depth -= 1
        elif ch == "." and depth == 0:
            parts.append(expr[start:i].strip())
            start = i + 1
    parts.append(expr[start:].strip())
    return [p for p in parts if p]


def strip_getter(name: str) -> str:
    """``getEntityId`` -> ``entityId``, leaving anything else alone.

    Java's accessor prefix carries no information a field name needs, and
    dropping it here is what stops it reaching the generated Rust. Names like
    ``get3DDataValue`` keep the prefix because the remainder is not an
    identifier on its own.
    """
    m = re.fullmatch(r"get([A-Z]\w*)", name)
    return m.group(1)[0].lower() + m.group(1)[1:] if m else name


def _getter_name(getter: str) -> str:
    """Field label from a composite's getter, ``Foo::bar`` or ``f -> f.bar()``."""
    getter = getter.strip()
    m = re.search(r"::(\w+)\s*$", getter)
    if m:
        return strip_getter(m.group(1))
    # A lambda projects onto one member, with or without call parentheses.
    m = re.search(r"->\s*\w+\.(\w+)\s*(?:\(\s*\))?\s*$", getter)
    if m:
        return strip_getter(m.group(1))
    names = re.findall(r"\.(\w+)\s*\(\s*\)", getter)
    return strip_getter(names[-1]) if names else getter


def call_args(segment: str) -> tuple[str, str | None]:
    """``foo(a, b)`` -> ``("foo", "a, b")``; ``FOO`` -> ``("FOO", None)``."""
    open_paren = segment.find("(")
    if open_paren == -1:
        return segment.strip(), None
    if match_bracket(segment, open_paren) != len(segment):
        return segment.strip(), None
    return segment[:open_paren].strip(), segment[open_paren + 1 : -1]


@dataclass(frozen=True)
class Scope:
    """Where an expression is being read, and what its free names mean.

    ``owner`` is the class the expression was written in, which is how a simple
    type name gets resolved. ``env`` binds the parameters of an inlined factory
    to the argument expressions the caller passed, each with the scope it was
    written in: without it, inlining ``CustomPacketPayload.codec(Foo::write,
    Foo::new)`` leaves the parameter ``writer`` free and every one of the 130
    custom payload types comes back unresolved.
    """

    owner: str
    env: tuple[tuple[str, str, "Scope"], ...] = ()

    def lookup(self, name: str) -> tuple[str, "Scope"] | None:
        for bound, expr, scope in self.env:
            if bound == name:
                return expr, scope
        return None

    def bind(self, owner: str, params: list[str], args: list[str], caller: "Scope") -> "Scope":
        return Scope(owner, tuple((p, a, caller) for p, a in zip(params, args)))


class Resolver:
    """Evaluates a Java stream-codec expression down to a wire type.

    Every path that cannot be modelled returns ``unresolved`` carrying the text
    that defeated it. Nothing is ever dropped: an unresolved leaf propagates all
    the way up, so a packet is only reported complete when its whole byte
    sequence is known.
    """

    def __init__(self, index: SourceIndex) -> None:
        self.index = index
        self.types: dict[str, Wire] = {}
        # Keys where the asserted layout had to stand in for a derived one.
        # Reported so the asserted surface stays visible and prunable.
        self.asserted: set[str] = set()
        self._active: set[str] = set()
        self.registry_keys = self._scan_registry_keys()

    # -- registry key names -------------------------------------------------

    def _scan_registry_keys(self) -> dict[str, str]:
        """``Registries.BLOCK`` -> ``minecraft:block``.

        Read from the source rather than guessed from the constant name: most
        are the lower-cased constant, but a handful are not.
        """
        out: dict[str, str] = {}
        jf = self.index.files.get("net.minecraft.core.registries.Registries")
        if jf is None:
            return out
        for m in re.finditer(
            r"\b([A-Z0-9_]+)\s*=\s*(?:Registries\.)?create(?:Registry|)Key\s*\(\s*\"([a-z0-9_/.]+)\"",
            jf.raw,
        ):
            out[m.group(1)] = f"minecraft:{m.group(2)}"
        return out

    # -- member lookup ------------------------------------------------------

    def _class_statements(self, body: str) -> list[str]:
        """Top-level statements of a class body, including static blocks.

        A constant declared with its initializer and one assigned in a static
        block are the same thing to us; ``Difficulty.STREAM_CODEC`` is only
        found by looking in the block.
        """
        out: list[str] = []
        for stmt in statements(body):
            if re.match(r"^static\s*\{", stmt) or re.match(r"^\{", stmt):
                brace = stmt.find("{")
                out.extend(self._class_statements(stmt[brace + 1 : match_bracket(stmt, brace) - 1]))
            else:
                out.append(stmt)
        return out

    def member_initializer(self, owner: str, member: str) -> str | None:
        body = self.index.body_of(owner)
        if body is None:
            return None
        pattern = re.compile(rf"(?:^|[\s>\]])({re.escape(member)})\s*=\s*(?!=)")
        for stmt in self._class_statements(body):
            m = pattern.search(stmt)
            if m and "(" not in stmt[: m.start(1)]:
                return stmt[m.end() :].strip()
        return None

    def methods(self, owner: str, name: str) -> list[tuple[list[str], str]]:
        """Every overload of a method as (parameter names, body).

        Overloads are kept apart by arity rather than collapsed to the first
        match: ``CustomPacketPayload.codec`` has two, and picking the wrong one
        would attach the wrong layout to every custom payload.
        """
        body = self.index.body_of(owner)
        if body is None:
            return []
        out: list[tuple[list[str], str]] = []
        for m in re.finditer(rf"\b{re.escape(name)}\s*\(", body):
            close = match_bracket(body, m.end() - 1)
            rest = body[close:].lstrip()
            if not rest.startswith("{"):
                continue
            params: list[str] = []
            for param in split_top_level(body[m.end() : close - 1]):
                words = re.findall(r"\w+", param)
                if not words:
                    params = []
                    break
                params.append(words[-1])
            offset = close + (len(body[close:]) - len(rest))
            out.append((params, body[offset + 1 : match_bracket(body, offset) - 1]))
        return out

    def method_body(self, owner: str, name: str, arity: int | None = None) -> str | None:
        for params, body in self.methods(owner, name):
            if arity is None or len(params) == arity:
                return body
        return None

    # -- entry points -------------------------------------------------------

    def named(self, owner: str, member: str) -> Wire:
        """Resolve ``Class.MEMBER`` to a reference into the shared type table."""
        key = f"{owner}#{member}"
        if key in self.types or key in self._active:
            return {"kind": "named", "ref": key}
        self._active.add(key)
        try:
            init = self.member_initializer(owner, member)
            wire = unresolved(f"no initializer for {key}") if init is None else self.eval(init, Scope(owner))
        finally:
            self._active.discard(key)
        # The asserted layout wins only where the derived one came up short,
        # so the source stays the source of truth wherever it can be read.
        if not is_resolved(wire, self.types) and key in HAND_MODELLED:
            wire = dict(HAND_MODELLED[key])
            self.asserted.add(key)
        self.types[key] = wire
        return {"kind": "named", "ref": key}

    def eval(self, expr: str, scope: Scope) -> Wire:
        expr = strip_wrappers(expr)
        if not expr:
            return unresolved("empty expression")
        bound = scope.lookup(expr)
        if bound is not None:
            # A parameter of an inlined factory: evaluate the argument the
            # caller passed, in the caller's own scope.
            return self.eval(bound[0], bound[1])
        if _NEW_ANON.match(expr):
            return self._eval_anonymous(expr, scope)

        parts = split_chain(expr)
        head, head_args = call_args(parts[0])

        if head == "ByteBufCodecs" and len(parts) >= 2:
            base = self._eval_byte_buf_codecs(parts[1], scope)
            rest = parts[2:]
        elif head == "StreamCodec" and len(parts) >= 2:
            base = self._eval_stream_codec(parts[1], scope)
            rest = parts[2:]
        elif head_args is not None:
            base = self._eval_local_call(head, head_args, scope)
            rest = parts[1:]
        else:
            base, rest = self._eval_qualified(parts, scope)

        for segment in rest:
            base = self._apply_segment(base, segment, scope)
        return base

    # -- primaries ----------------------------------------------------------

    def _eval_qualified(self, parts: list[str], scope: Scope) -> tuple[Wire, list[str]]:
        """``Team.Visibility.STREAM_CODEC`` and friends.

        The longest leading run of type-shaped segments names the class; the
        segment after it names the member.
        """
        type_parts: list[str] = []
        for part in parts:
            name, args = call_args(part)
            if args is None and name[:1].isupper() and not name.isupper():
                type_parts.append(name)
                continue
            break
        if not type_parts or len(type_parts) == len(parts):
            # A constant of the class we are already reading, written without a
            # qualifier: `TRUSTED_STREAM_CODEC.apply(ByteBufCodecs::optional)`.
            head, head_args = call_args(parts[0])
            if head_args is None and re.fullmatch(r"[A-Za-z_]\w*", head):
                bound = scope.lookup(head)
                if bound is not None:
                    return self.eval(bound[0], bound[1]), parts[1:]
                # Walk out through the enclosing classes: a nested codec often
                # names a constant of the class that encloses it.
                owner: str | None = scope.owner
                while owner is not None:
                    if self.member_initializer(owner, head) is not None:
                        return self.named(owner, head), parts[1:]
                    owner = _enclosing_owner(owner)
            return unresolved(f"not a codec reference: {'.'.join(parts)}"), []

        member, member_args = call_args(parts[len(type_parts)])
        owner = self.index.resolve(".".join(type_parts), scope.owner)
        if owner is None:
            return unresolved(f"cannot resolve class {'.'.join(type_parts)}"), []
        rest = parts[len(type_parts) + 1 :]

        if member_args is None:
            return self.named(owner, member), rest

        inlined = self._inline_method(owner, member, split_top_level(member_args), scope)
        if inlined is None:
            return unresolved(f"unmodelled factory {'.'.join(type_parts)}.{member}(..)"), []
        return inlined, rest

    def _inline_method(
        self,
        owner: str,
        name: str,
        args: list[str] | None = None,
        caller: Scope | None = None,
    ) -> Wire | None:
        """Evaluate a one-line ``return <expr>;`` factory in place."""
        overloads = self.methods(owner, name)
        if args is not None:
            overloads = [o for o in overloads if len(o[0]) == len(args)]
        if len(overloads) != 1:
            return None
        params, body = overloads[0]
        stmts = statements(body)
        if len(stmts) != 1 or not stmts[0].startswith("return "):
            return None
        callee = Scope(owner)
        if args is not None and caller is not None:
            callee = callee.bind(owner, params, args, caller)
        return self.eval(stmts[0][len("return ") :], callee)

    def _eval_local_call(self, name: str, args: str, scope: Scope) -> Wire:
        """An unqualified call to a factory declared in the same class."""
        inlined = self._inline_method(scope.owner, name, split_top_level(args), scope)
        if inlined is None:
            return unresolved(f"unmodelled local factory {name}(..)")
        return inlined

    def _eval_stream_codec(self, segment: str, scope: Scope) -> Wire:
        name, args = call_args(segment)
        if args is None:
            return unresolved(f"unmodelled StreamCodec.{name}")
        argv = split_top_level(args)
        if name == "composite":
            return self._composite(argv, scope)
        if name == "unit":
            return prim("unit", note="carries no bytes")
        if name in ("of", "ofMember"):
            return self._from_encoder_reference(argv[0], scope) if argv else unresolved("StreamCodec.of()")
        if name == "recursive":
            # The factory takes the codec being defined and returns its body;
            # evaluating the body with the recursion already in flight is what
            # makes a self-referential codec terminate.
            return self._eval_lambda_return(argv[0], scope) if argv else unresolved("StreamCodec.recursive()")
        return unresolved(f"unmodelled StreamCodec.{name}(..)")

    def _composite(self, argv: list[str], scope: Scope) -> Wire:
        if len(argv) < 3 or len(argv) % 2 == 0:
            return unresolved(f"composite with {len(argv)} arguments")
        fields = []
        for codec_arg, getter in zip(argv[0:-1:2], argv[1:-1:2]):
            fields.append(
                {
                    "name": _getter_name(getter),
                    "wire": self.eval(codec_arg, scope),
                }
            )
        return {"kind": "struct", "fields": fields}

    def _follow(self, expr: str, scope: Scope) -> tuple[str, Scope]:
        """Chase a name bound by an inlined factory to its argument.

        Applied wherever an expression is consumed as something other than a
        codec value, since those paths bypass ``eval``'s own lookup.
        """
        expr = strip_wrappers(expr)
        seen = 0
        while (bound := scope.lookup(expr)) is not None and seen < 16:
            expr, scope = strip_wrappers(bound[0]), bound[1]
            seen += 1
        return expr, scope

    def _eval_lambda_return(self, expr: str, scope: Scope) -> Wire:
        """Body of a single-expression lambda, e.g. ``c -> <codec>``."""
        expr, scope = self._follow(expr, scope)
        m = _LAMBDA_HEAD.match(expr)
        if not m:
            return unresolved(f"not a lambda: {expr[:60]}")
        body = expr[m.end() :].strip()
        if body.startswith("{"):
            stmts = statements(body[1 : match_bracket(body, 0) - 1])
            if len(stmts) != 1 or not stmts[0].startswith("return "):
                return unresolved("multi-statement lambda")
            body = stmts[0][len("return ") :]
        return self.eval(body, scope)

    def _from_encoder_reference(self, expr: str, scope: Scope) -> Wire:
        """``StreamCodec.of(Foo::write, ..)`` -> read ``Foo.write``'s body."""
        expr, scope = self._follow(expr, scope)
        m = re.match(r"^([\w.]+)::(\w+)$", expr)
        if not m:
            return unresolved(f"unmodelled encoder {expr[:60]}")
        owner = self.index.resolve(m.group(1), scope.owner)
        if owner is None:
            return unresolved(f"cannot resolve encoder class {m.group(1)}")
        body = self.method_body(owner, m.group(2))
        if body is None:
            return unresolved(f"no body for {m.group(1)}.{m.group(2)}")
        buf = self.first_param(owner, m.group(2))
        if buf is None:
            return unresolved(f"no buffer parameter on {m.group(1)}.{m.group(2)}")
        return self.read_encode_body(body, Scope(owner), buf=buf)

    def _eval_anonymous(self, expr: str, scope: Scope) -> Wire:
        brace = expr.find("{")
        if brace == -1:
            return unresolved("anonymous StreamCodec without a body")
        body = expr[brace + 1 : match_bracket(expr, brace) - 1]
        m = re.search(r"\bvoid\s+encode\s*\(", body)
        if not m:
            return unresolved("anonymous StreamCodec without an encode method")
        close = match_bracket(body, m.end() - 1)
        rest = body[close:].lstrip()
        if not rest.startswith("{"):
            return unresolved("anonymous encode without a body")
        offset = close + (len(body[close:]) - len(rest))
        return self.read_encode_body(body[offset + 1 : match_bracket(body, offset) - 1], scope, enclosing=body)
    # -- ByteBufCodecs ------------------------------------------------------

    def _registry_name(self, expr: str) -> str | None:
        m = re.search(r"Registries\.([A-Z0-9_]+)", expr)
        if m:
            return self.registry_keys.get(m.group(1))
        return None

    def _eval_byte_buf_codecs(self, segment: str, scope: Scope) -> Wire:
        name, args = call_args(segment)
        if args is None:
            # Resolved from source like anything else. CODEC_FIELDS only backs
            # up the handful whose definition bottoms out somewhere this reader
            # cannot follow, and named() applies it.
            return self.named(BYTE_BUF_CODECS_FQN, name)
        argv = split_top_level(args)

        if name == "stringUtf8":
            return prim("string", max=parse_int_literal(argv[0]))
        if name == "byteArray":
            return prim("byte_array", max=parse_int_literal(argv[0]))
        if name == "lenientJson":
            # A length-limited UTF-8 string on the wire; the JSON parse only
            # shapes the in-memory value.
            return prim("string", max=parse_int_literal(argv[0]), note="parsed as JSON after decoding")
        if name in ("tagCodec", "compoundTagCodec"):
            return prim("nbt")
        if name == "optionalTagCodec":
            return prim("optional_nbt", note="TAG_End means absent")
        if name.startswith("fromCodec"):
            # Every fromCodec* variant funnels through tagCodec: the value is
            # serialised to NBT with a DFU codec and written as a network tag.
            return prim("nbt", note="DFU codec serialised to NBT")
        if name == "optional":
            return {"kind": "option", "of": self.eval(argv[0], scope)}
        if name == "collection":
            if len(argv) < 2:
                return unresolved("ByteBufCodecs.collection with no element codec")
            return {
                "kind": "list",
                "of": self.eval(argv[1], scope),
                **({"max": parse_int_literal(argv[2])} if len(argv) > 2 else {}),
            }
        if name == "map":
            if len(argv) < 3:
                return unresolved("ByteBufCodecs.map with no key/value codec")
            return {
                "kind": "map",
                "key": self.eval(argv[1], scope),
                "value": self.eval(argv[2], scope),
                **({"max": parse_int_literal(argv[3])} if len(argv) > 3 else {}),
            }
        if name == "either":
            return {
                "kind": "either",
                "left": self.eval(argv[0], scope),
                "right": self.eval(argv[1], scope),
                "note": "bool discriminant, true selects left",
            }
        if name == "idMapper":
            return prim("varint", note="index into an id map")
        if name in ("registry", "holderRegistry"):
            registry = self._registry_name(argv[0])
            if registry is None:
                return unresolved(f"unknown registry key in {argv[0]}")
            return prim("registry_id", registry=registry)
        if name == "holder":
            registry = self._registry_name(argv[0])
            if registry is None:
                return unresolved(f"unknown registry key in {argv[0]}")
            return {
                "kind": "holder",
                "registry": registry,
                "of": self.eval(argv[1], scope),
                "note": "varint 0 selects the inline value, otherwise id + 1",
            }
        if name == "holderSet":
            registry = self._registry_name(argv[0])
            if registry is None:
                return unresolved(f"unknown registry key in {argv[0]}")
            return prim("holder_set", registry=registry,
                        note="varint 0 then a tag identifier, otherwise count + 1 then that many holder ids")
        return unresolved(f"unmodelled ByteBufCodecs.{name}(..)")

    # -- chained operations -------------------------------------------------

    def _apply_segment(self, base: Wire, segment: str, scope: Scope) -> Wire:
        name, args = call_args(segment)
        if name in ("map", "mapStream", "cast", "validate", "orElse"):
            # None of these change the bytes; they only reshape the value or
            # add a check on the decoded side.
            return base
        if name == "apply" and args is not None:
            return self._apply_operation(base, args, scope)
        if name == "dispatch":
            return unresolved("dispatched codec: the layout depends on a runtime type")
        return unresolved(f"unmodelled codec operation .{name}")

    def _apply_operation(self, base: Wire, op: str, scope: Scope) -> Wire:
        op = strip_wrappers(op)
        if op == "ByteBufCodecs::optional":
            return {"kind": "option", "of": base}
        parts = split_chain(op)
        if parts[0] != "ByteBufCodecs" or len(parts) != 2:
            return unresolved(f"unmodelled codec operation apply({op[:60]})")
        name, args = call_args(parts[1])
        argv = split_top_level(args) if args else []
        if name == "list":
            return {"kind": "list", "of": base, **({"max": parse_int_literal(argv[0])} if argv else {})}
        if name == "collection":
            return {"kind": "list", "of": base, **({"max": parse_int_literal(argv[1])} if len(argv) > 1 else {})}
        if name in ("lengthPrefixed", "registryFriendlyLengthPrefixed"):
            return {"kind": "length_prefixed", "of": base,
                    **({"max": parse_int_literal(argv[0])} if argv else {}),
                    "note": "varint byte length, then the value"}
        if name == "fromCodec":
            # tagCodec.apply(fromCodec(NbtOps.INSTANCE, codec)): the bytes are
            # still whatever the base codec writes, here an NBT tag.
            return base
        return unresolved(f"unmodelled codec operation apply(ByteBufCodecs.{name})")
    # -- linear encode bodies -----------------------------------------------

    # Any of these makes the byte sequence depend on a runtime value, so the
    # body stops being a layout we can read off the source.
    _BRANCHING = re.compile(r"^(if|for|while|do|switch|return|try|throw|synchronized)\b")

    def read_encode_body(
        self,
        body: str,
        scope: Scope,
        buf: str = "output",
        enclosing: str | None = None,
        unwrap_single: bool = True,
    ) -> Wire:
        """Read a straight-line ``encode`` body as an ordered field list.

        Every statement must be accounted for. A statement that is not a
        recognised buffer write -- ``this.payload.write(output)`` in the login
        custom-query packets, say -- means part of the layout lives somewhere
        this reader cannot see, and returning the fields recovered so far would
        emit a codec that is silently short. Those bodies come back unresolved.
        """
        fields: list[dict[str, Any]] = []
        locals_: dict[str, str] = {}
        for stmt in statements(body):
            stmt = stmt.strip()
            if not stmt:
                continue
            if self._BRANCHING.match(stmt):
                return unresolved(f"branching encode body: {stmt[:60]}")
            got = self._read_statement(stmt, scope, buf, locals_, enclosing)
            if got is None:
                return unresolved(f"unmodelled statement: {' '.join(stmt.split())[:90]}")
            fields.extend(got)
        if not fields:
            return prim("unit", note="carries no bytes")
        if unwrap_single and len(fields) == 1:
            # A value codec that writes one thing is that thing; wrapping it in
            # a one-field struct would only add a layer for readers to peel.
            return fields[0]["wire"]
        return {"kind": "struct", "fields": fields}

    def _read_statement(
        self,
        stmt: str,
        scope: Scope,
        buf: str,
        locals_: dict[str, str],
        enclosing: str | None,
    ) -> list[dict[str, Any]] | None:
        # A local that never touches the buffer computes a value rather than
        # writing one; remembering it lets a later `codec.encode(buf, x)` find
        # the codec it names.
        decl = re.match(r"^(?:final\s+)?[\w.<>\[\],?\s]+?\s(\w+)\s*=\s*(.+)$", stmt, flags=re.S)
        if decl and not re.search(rf"\b{re.escape(buf)}\b", decl.group(2)):
            locals_[decl.group(1)] = decl.group(2).strip()
            return []

        parts = split_chain(stmt)
        if len(parts) < 2:
            return None
        name, args = call_args(parts[-1])
        if args is None:
            return None
        argv = split_top_level(args)
        receiver = ".".join(parts[:-1])

        # buf.writeX(..)
        if receiver == buf or receiver == f"this.{buf}":
            return self._read_buffer_write(name, argv, scope, buf, locals_)

        # Class.writeX(buf, ..) and VarInt.write(buf, id)
        if argv and strip_wrappers(argv[0]) == buf:
            static_key = f"{receiver}.{name}"
            wire = STATIC_WRITERS.get(static_key)
            if wire is not None:
                return [{"name": self._label(argv[1] if len(argv) > 1 else ""), "wire": dict(wire)}]
            if static_key == "FriendlyByteBuf.writeNullable" and len(argv) == 3:
                inner = self._element_encoder(argv[2], scope, buf)
                return [{"name": self._label(argv[1]), "wire": {"kind": "option", "of": inner}}]
            # <codec>.encode(buf, value)
            if name == "encode":
                codec = locals_.get(receiver, receiver)
                if enclosing is not None and receiver.startswith("this."):
                    field_init = self._enclosing_field(enclosing, receiver[len("this.") :])
                    if field_init is not None:
                        codec = field_init
                return [{"name": self._label(argv[1] if len(argv) > 1 else ""),
                         "wire": self.eval(codec, scope)}]

        # A helper that writes through a buffer handed to it, in any argument
        # position: `this.chunkData.write(output)`, `MessageSignature.Packed
        # .write(output, sig)`, `ClientboundSetEntityDataPacket.pack(items,
        # output)`. Reading the helper's own body is what keeps these from
        # being reported as layouts we cannot see.
        nested = self._read_nested_write(receiver, name, argv, scope, buf)
        if nested is not None:
            return nested
        return None

    def _read_nested_write(
        self,
        receiver: str,
        method: str,
        argv: list[str],
        scope: Scope,
        buf: str,
    ) -> list[dict[str, Any]] | None:
        positions = [i for i, a in enumerate(argv) if strip_wrappers(a) == buf]
        if len(positions) != 1:
            return None
        index = positions[0]

        if receiver == "this" or receiver.startswith("this."):
            owner = self._field_owner(scope.owner, receiver.removeprefix("this").lstrip("."))
        else:
            owner = self.index.resolve(receiver, scope.owner)
        if owner is None:
            return None

        overloads = [o for o in self.methods(owner, method) if len(o[0]) == len(argv)]
        if len(overloads) != 1:
            return None
        params, body = overloads[0]
        key = f"{owner}#{method}/{len(argv)}"
        if key in self._active:
            return None
        self._active.add(key)
        try:
            wire = self.read_encode_body(body, Scope(owner), buf=params[index])
        finally:
            self._active.discard(key)
        if not is_resolved(wire, self.types):
            return None
        return [{"name": self._label(receiver if receiver.startswith("this.") else argv[0]), "wire": wire}]

    def _field_owner(self, owner: str, name: str) -> str | None:
        """Class of ``this.<name>``, from the field or record component."""
        body = self.index.body_of(owner)
        if body is None:
            return owner if not name else None
        if not name:
            return owner
        declared = re.search(rf"(?:^|[;{{}}\s])([\w.]+(?:<[^;]*>)?)\s+{re.escape(name)}\s*[;=]", body)
        if declared is None:
            # A record component: the type sits in the header, not the body.
            span = self.index.body_span(owner)
            if span is None:
                return None
            jf, start, _ = span
            head = jf.text[max(0, start - RECORD_HEADER_WINDOW) : start]
            declared = re.search(rf"([\w.]+(?:<[^()]*>)?)\s+{re.escape(name)}\s*[,)]", head)
            if declared is None:
                return None
        return self.index.resolve(re.sub(r"<.*", "", declared.group(1)), owner)

    def _enclosing_field(self, enclosing: str, name: str) -> str | None:
        """Initializer of a field declared in an anonymous class body."""
        for stmt in self._class_statements(enclosing):
            m = re.search(rf"(?:^|[\s>\]])(?:this\.)?{re.escape(name)}\s*=\s*(?!=)", stmt)
            if m:
                return stmt[m.end() :].strip()
        return None

    def _read_buffer_write(
        self,
        name: str,
        argv: list[str],
        scope: Scope,
        buf: str,
        locals_: dict[str, str],
    ) -> list[dict[str, Any]] | None:
        label = self._label(argv[0] if argv else "")

        if name == "writeUtf":
            limit = parse_int_literal(argv[1]) if len(argv) > 1 else 32767
            return [{"name": label, "wire": prim("string", max=limit)}]
        if name == "writeFixedBitSet" and len(argv) == 2:
            bits = parse_int_literal(argv[1])
            if bits is None:
                return None
            return [{"name": label, "wire": prim("fixed_bitset", bits=bits)}]
        if name == "writeEnumSet" and len(argv) == 2:
            bits = self._enum_cardinality(argv[1], scope)
            if bits is None:
                return None
            return [{"name": label, "wire": prim("fixed_bitset", bits=bits,
                                                 note="one bit per enum constant, rounded up to whole bytes")}]
        if name in BUF_COLLECTION_WRITERS and len(argv) == 2:
            inner = self._element_encoder(argv[1], scope, buf)
            kind = BUF_COLLECTION_WRITERS[name]
            return [{"name": label, "wire": {"kind": kind, "of": inner}}]
        if name == "writeMap" and len(argv) == 3:
            return [{"name": label, "wire": {
                "kind": "map",
                "key": self._element_encoder(argv[1], scope, buf),
                "value": self._element_encoder(argv[2], scope, buf),
            }}]
        if name == "writeEither" and len(argv) == 3:
            return [{"name": label, "wire": {
                "kind": "either",
                "left": self._element_encoder(argv[1], scope, buf),
                "right": self._element_encoder(argv[2], scope, buf),
                "note": "bool discriminant, true selects left",
            }}]

        wire = BUF_WRITERS.get(name)
        if wire is None:
            return None
        return [{"name": label, "wire": dict(wire)}]

    def _enum_cardinality(self, expr: str, scope: Scope) -> int | None:
        """Number of constants in ``Foo.class``, needed for a fixed bitset."""
        m = re.match(r"^([\w.]+)\.class$", strip_wrappers(expr))
        if not m:
            return None
        target = self.index.resolve(m.group(1), scope.owner)
        if target is None:
            return None
        body = self.index.body_of(target)
        if body is None:
            return None
        header = body.split(";", 1)[0]
        constants = [c for c in split_top_level(header) if re.match(r"^[A-Z][A-Z0-9_]*\b", c.strip())]
        return len(constants) or None

    def _element_encoder(self, expr: str, scope: Scope, buf: str) -> Wire:
        """The element writer handed to writeCollection and friends."""
        expr, scope = self._follow(expr, scope)
        m = re.match(r"^([\w.]+)::(\w+)$", expr)
        if m:
            owner, method = m.group(1), m.group(2)
            if owner.split(".")[-1] in ("FriendlyByteBuf", "RegistryFriendlyByteBuf", "ByteBuf"):
                wire = BUF_WRITERS.get(method)
                return dict(wire) if wire else unresolved(f"unmodelled element writer ::{method}")
            static = STATIC_WRITERS.get(f"{owner}.{method}")
            if static is not None:
                return dict(static)
            target = self.index.resolve(owner, scope.owner)
            if target is None:
                return unresolved(f"cannot resolve element writer {expr}")
            body = self.method_body(target, method)
            if body is None:
                return unresolved(f"no body for element writer {expr}")
            return self.read_encode_body(body, Scope(target), buf=self.first_param(target, method) or buf)

        lam = _LAMBDA_HEAD.match(expr)
        if lam:
            head = expr[: lam.end()]
            names = re.findall(r"\w+", head)
            if not names:
                return unresolved("element writer lambda without parameters")
            body = expr[lam.end() :].strip()
            if body.startswith("{"):
                body = body[1 : match_bracket(body, 0) - 1]
            return self.read_encode_body(body, scope, buf=names[0])

        # A bare codec value: `SomeCodec.STREAM_CODEC::encode` already matched
        # above, so this is an expression evaluating to a StreamCodec.
        return self.eval(expr, scope)

    def first_param(self, owner: str, method: str) -> str | None:
        """Name of a method's first parameter, i.e. the buffer it writes to."""
        body = self.index.body_of(owner)
        if body is None:
            return None
        m = re.search(rf"\b{re.escape(method)}\s*\(\s*[\w.<>\[\],?\s]*?\s(\w+)\s*[,)]", body)
        return m.group(1) if m else None

    @staticmethod
    def _label(arg: str) -> str:
        """A readable field name for a written value.

        Only a label: the byte layout comes from the writer, never from this.
        The field the value came out of is preferred over whatever was called
        on it, so ``this.intention.id()`` is ``intention`` rather than ``id``.
        """
        arg = arg.strip()
        m = re.search(r"\bthis\.(\w+)", arg)
        if m:
            return m.group(1)
        m = re.search(r"->\s*\w+\.(\w+)\s*(?:\(\s*\))?\s*$", arg)
        if m:
            return strip_getter(m.group(1))
        matches = re.findall(r"\.(\w+)\s*\(\s*\)", arg)
        if matches:
            return strip_getter(matches[-1])
        matches = re.findall(r"\b(\w+)\b", arg)
        return strip_getter(matches[-1]) if matches else ""


# ---------------------------------------------------------------------------
# Packet table
# ---------------------------------------------------------------------------


@dataclass
class Packet:
    resource: str
    state: str
    direction: str
    protocol_id: int
    java_class: str | None = None
    layout_source: str = "none"
    wire: Wire = field(default_factory=lambda: unresolved("not attempted"))
    complete: bool = False
    reasons: list[str] = field(default_factory=list)


PACKET_TYPE_DECL = re.compile(
    r"PacketType<(?P<cls>[\w.$]+)>\s+(?P<const>[A-Z0-9_]+)\s*=\s*"
    r"\w+\.create(?P<flow>Clientbound|Serverbound)\(\s*\"(?P<id>[a-z0-9_/]+)\"",
)

# The bundle delimiter is registered with withBundlePacket rather than
# addPacket, but it still consumes protocol id 0 of play/clientbound, so both
# spellings have to count towards the ordinal. Leaving withBundlePacket out is
# an off-by-one across the whole channel.
ADD_PACKET = re.compile(r"\.(?:addPacket|withBundlePacket)\(\s*([\w.$]+)\s*,")

PROTOCOL_TEMPLATE = re.compile(
    r"(?P<var>[A-Z0-9_]+)_TEMPLATE\s*=\s*ProtocolInfoBuilder\.(?P<flow>\w+)Protocol\("
    r"ConnectionProtocol\.(?P<state>\w+)\s*,(?P<body>.*?);\n",
    re.S,
)

# packets.json spells the handshaking state differently from ConnectionProtocol.
STATE_ALIASES = {"handshaking": "handshake"}


def scan_packet_types(index: SourceIndex) -> dict[str, tuple[str, str, str]]:
    """const name -> (flow, resource id, owner), from the *PacketTypes classes.

    The class is resolved against the declaring file's imports rather than
    reduced to a simple name: seven of the movement packets are nested records
    (``ClientboundMoveEntityPacket.Pos``) whose simple name matches nothing.
    """
    table: dict[str, tuple[str, str, str]] = {}
    for jf in index.files.values():
        if not jf.simple.endswith("PacketTypes"):
            continue
        for m in PACKET_TYPE_DECL.finditer(jf.raw):
            owner = index.resolve(m.group("cls").replace("$", "."), jf.fqn)
            if owner is None:
                continue
            table[f"{jf.simple}.{m.group('const')}"] = (
                m.group("flow").lower(),
                f"minecraft:{m.group('id')}",
                owner,
            )
    return table


def scan_protocol_order(index: SourceIndex) -> dict[str, list[str]]:
    """channel -> ordered PacketTypes constants, i.e. the numeric id order.

    Recovering the order from the server's own protocol builder gives an
    independent check on the ids in packets.json rather than trusting one
    source. It has already caught a real off-by-one.
    """
    out: dict[str, list[str]] = {}
    for jf in index.files.values():
        if not jf.simple.endswith("Protocols"):
            continue
        for decl in PROTOCOL_TEMPLATE.finditer(jf.text):
            state = decl.group("state").lower()
            key = f"{STATE_ALIASES.get(state, state)}/{decl.group('flow').lower()}"
            out.setdefault(key, []).extend(m.group(1) for m in ADD_PACKET.finditer(decl.group("body")))
    return out


def scan_registered_codecs(index: SourceIndex) -> dict[str, tuple[str, str]]:
    """packet owner -> (codec expression, the file it was written in).

    The protocol builder is where the server installs a codec against a packet
    type, so it is the authority when the packet class has no codec of its own.
    The bundle delimiter is the case that needs it: it carries no payload and
    no encoder, and its zero-byte layout exists only as the
    ``StreamCodec.unit`` handed to ``withBundlePacket``.
    """
    out: dict[str, tuple[str, str]] = {}
    for jf in index.files.values():
        if not jf.simple.endswith("Protocols"):
            continue
        for m in re.finditer(r"\.withBundlePacket\s*\(", jf.text):
            argv = split_top_level(jf.text[m.end() : match_bracket(jf.text, m.end() - 1) - 1])
            if len(argv) != 3:
                continue
            cm = re.match(r"^new\s+([\w.]+)\s*\(", argv[2].strip())
            owner = index.resolve(cm.group(1), jf.fqn) if cm else None
            if owner is not None:
                out[owner] = ("StreamCodec.unit(delimiter)", jf.fqn)
    return out


def packet_codec_member(index: SourceIndex, owner: str) -> str | None:
    """Name of the class member holding the packet's stream codec."""
    body = index.body_of(owner)
    if body is None:
        return None
    if re.search(r"\bSTREAM_CODEC\s*=", body):
        return "STREAM_CODEC"
    m = re.search(r"\bStreamCodec\s*<[^;=]*>\s+([A-Z0-9_]+)\s*=", body)
    return m.group(1) if m else None


def recover_packet(
    pkt: Packet,
    index: SourceIndex,
    resolver: Resolver,
    registered: dict[str, tuple[str, str]],
) -> None:
    fqn = pkt.java_class
    if fqn is None or index.body_of(fqn) is None:
        pkt.wire = unresolved("decompiled source not found")
        pkt.reasons = [pkt.wire["why"]]
        return

    member = packet_codec_member(index, fqn)
    body = resolver.method_body(fqn, "write")
    buf = resolver.first_param(fqn, "write")
    if member is not None:
        pkt.layout_source = f"{fqn}#{member}"
        pkt.wire = resolver.named(fqn, member)
    elif body is not None and buf is not None:
        pkt.layout_source = f"{fqn}#write()"
        pkt.wire = resolver.read_encode_body(body, Scope(fqn), buf=buf, unwrap_single=False)
    elif fqn in registered:
        expression, declared_in = registered[fqn]
        pkt.layout_source = f"registered in {declared_in}"
        pkt.wire = resolver.eval(expression, Scope(declared_in))
    else:
        pkt.layout_source = "none"
        pkt.wire = unresolved("no stream codec and no write method")

    pkt.complete = is_resolved(pkt.wire, resolver.types)
    pkt.reasons = sorted(set(unresolved_reasons(pkt.wire, resolver.types)))


# ---------------------------------------------------------------------------
# Data components
#
# ItemStack carries a patch over this registry, so every component's layout is
# part of the item layout. The ids come from Mojang's registry report and the
# codecs from DataComponents' own registration calls; the two are cross-checked
# against each other the same way packet ids are.
# ---------------------------------------------------------------------------

DATA_COMPONENTS_FQN = "net.minecraft.core.component.DataComponents"
REGISTER_CALL = re.compile(r"\bregister\s*\(\s*\"([a-z0-9_/]+)\"\s*,")


def scan_data_components(index: SourceIndex, resolver: Resolver) -> tuple[list[dict[str, Any]], list[str]]:
    body = index.raw_body_of(DATA_COMPONENTS_FQN)
    if body is None:
        return [], [f"{DATA_COMPONENTS_FQN} was not decompiled"]

    out: list[dict[str, Any]] = []
    problems: list[str] = []
    for stmt in statements(body):
        m = REGISTER_CALL.search(stmt)
        if m is None:
            continue
        open_paren = stmt.index("(", m.start())
        argv = split_top_level(stmt[open_paren + 1 : match_bracket(stmt, open_paren) - 1])
        if len(argv) != 2:
            problems.append(f"minecraft:{m.group(1)}: register(..) with {len(argv)} arguments")
            continue
        builder = argv[1]

        network = _builder_argument(builder, "networkSynchronized")
        if network is not None:
            wire = resolver.eval(network, Scope(DATA_COMPONENTS_FQN))
            source = "networkSynchronized"
        else:
            persistent = _builder_argument(builder, "persistent")
            if persistent is None:
                problems.append(f"minecraft:{m.group(1)}: neither networkSynchronized nor persistent")
                continue
            # DataComponentType.Builder.build() falls back to
            # fromCodecWithRegistries when no stream codec was given, so the
            # value goes on the wire as network NBT of the persistent codec.
            wire = prim("nbt", note="persistent codec serialised to NBT")
            source = "persistent fallback"

        out.append(
            {
                "name": f"minecraft:{m.group(1)}",
                "constant": _register_target(stmt),
                "codecSource": source,
                "wire": wire,
                "complete": is_resolved(wire, resolver.types),
                "reasons": sorted(set(unresolved_reasons(wire, resolver.types))),
            }
        )
    return out, problems


def _builder_argument(builder: str, method: str) -> str | None:
    """Argument of ``.method(..)`` in a fluent builder lambda."""
    m = re.search(rf"\.{re.escape(method)}\s*\(", builder)
    if m is None:
        return None
    return builder[m.end() : match_bracket(builder, m.end() - 1) - 1].strip()


def _register_target(stmt: str) -> str | None:
    m = re.search(r"\b([A-Z0-9_]+)\s*=\s*[\w.]*register\s*\(", stmt)
    return m.group(1) if m else None

# ---------------------------------------------------------------------------


def cross_check_packet_ids(
    packets: list[Packet],
    order: dict[str, list[str]],
    const_to_resource: dict[str, str],
) -> list[str]:
    """Compare Mojang's ids with the registration order in the server source."""
    mismatches: list[str] = []
    by_key = {(p.state, p.direction, p.resource): p for p in packets}
    for key, consts in sorted(order.items()):
        state, flow = key.split("/")
        for index, const in enumerate(consts):
            resource = const_to_resource.get(const.split(".")[-1])
            if resource is None:
                continue
            pkt = by_key.get((state, flow, resource))
            if pkt is not None and pkt.protocol_id != index:
                mismatches.append(
                    f"{state}/{flow} {resource}: reports={pkt.protocol_id} source_order={index}"
                )
    return mismatches


def cross_check_components(
    components: list[dict[str, Any]],
    registry: dict[str, Any],
) -> list[str]:
    """Compare the registry report's component ids with registration order."""
    expected = sorted(registry["entries"], key=lambda name: registry["entries"][name]["protocol_id"])
    found = [c["name"] for c in components]
    if found == expected:
        return []
    problems = []
    for index, (want, got) in enumerate(zip(expected, found)):
        if want != got:
            problems.append(f"component id {index}: reports={want} source_order={got}")
    if len(expected) != len(found):
        problems.append(f"component count: reports={len(expected)} source={len(found)}")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--generated", required=True, type=Path, help="vanilla generated/ tree")
    ap.add_argument("--decompiled", required=True, type=Path, help="decompiled java tree")
    ap.add_argument("--version-json", required=True, type=Path, help="version.json from the jar")
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()

    reports = args.generated / "reports"
    packets_json = json.loads((reports / "packets.json").read_text())
    registries_json = json.loads((reports / "registries.json").read_text())
    version = json.loads(args.version_json.read_text())

    index = SourceIndex(args.decompiled)
    missing = check_vocabulary(index)
    if missing:
        print("modelled Java names that no longer exist:", file=sys.stderr)
        for name in missing:
            print(f"    {name}", file=sys.stderr)
        return 1

    resolver = Resolver(index)
    types = scan_packet_types(index)
    order = scan_protocol_order(index)
    registered = scan_registered_codecs(index)

    by_resource: dict[tuple[str, str], str] = {}
    const_to_resource: dict[str, str] = {}
    for const, (flow, resource, cls) in types.items():
        by_resource[(resource, flow)] = cls
        const_to_resource[const.split(".")[-1]] = resource

    packets: list[Packet] = []
    for state, dirs in sorted(packets_json.items()):
        for direction, entries in sorted(dirs.items()):
            for resource, meta in sorted(entries.items()):
                pkt = Packet(
                    resource=resource,
                    state=state,
                    direction=direction,
                    protocol_id=meta["protocol_id"],
                    java_class=by_resource.get((resource, direction)),
                )
                if pkt.java_class:
                    recover_packet(pkt, index, resolver, registered)
                else:
                    pkt.wire = unresolved("no PacketType declaration matched")
                    pkt.reasons = [pkt.wire["why"]]
                packets.append(pkt)

    components, component_problems = scan_data_components(index, resolver)
    component_problems += cross_check_components(
        components, registries_json["minecraft:data_component_type"]
    )
    id_mismatches = cross_check_packet_ids(packets, order, const_to_resource)

    complete = [p for p in packets if p.complete]
    partial = [p for p in packets if not p.complete and p.layout_source != "none"]
    unrecovered = [p for p in packets if not p.complete and p.layout_source == "none"]
    done_components = [c for c in components if c["complete"]]

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
            "unrecovered": len(unrecovered),
            "dataComponents": len(components),
            "dataComponentsMechanical": len(done_components),
            "namedTypes": len(resolver.types),
            "assertedTypes": sorted(resolver.asserted),
            "idCrossCheckMismatches": id_mismatches,
            "dataComponentProblems": component_problems,
        },
        # Every named codec the packet and component layouts refer to, resolved
        # once and shared. A reference is {"kind": "named", "ref": <key>}.
        "types": dict(sorted(resolver.types.items())),
        "packets": [
            {
                "resource": p.resource,
                "state": p.state,
                "direction": p.direction,
                "protocolId": p.protocol_id,
                "javaClass": p.java_class,
                "layoutSource": p.layout_source,
                "complete": p.complete,
                "wire": p.wire,
                **({"reasons": p.reasons} if p.reasons else {}),
            }
            for p in packets
        ],
        "dataComponents": components,
        "registries": {
            name: {
                "protocolId": body.get("protocol_id"),
                "entries": sorted(body.get("entries", {}), key=lambda k: body["entries"][k]["protocol_id"]),
            }
            for name, body in sorted(registries_json.items())
        },
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(doc, indent=2, sort_keys=False) + "\n")

    cov = doc["coverage"]
    print(f"protocol {doc['version']['protocolVersion']} ({doc['version']['id']})", file=sys.stderr)
    print(f"  packets:              {cov['packets']}", file=sys.stderr)
    print(f"  fully mechanical:     {cov['fullyMechanical']}", file=sys.stderr)
    print(f"  partial:              {cov['partial']}", file=sys.stderr)
    print(f"  unrecovered:          {cov['unrecovered']}", file=sys.stderr)
    print(f"  data components:      {cov['dataComponentsMechanical']}/{cov['dataComponents']}", file=sys.stderr)
    print(f"  named codec types:    {cov['namedTypes']}", file=sys.stderr)
    print(f"  registries:           {len(doc['registries'])}", file=sys.stderr)

    failed = False
    if id_mismatches:
        print(f"  ID CROSS-CHECK FAILED: {len(id_mismatches)}", file=sys.stderr)
        for line in id_mismatches[:10]:
            print(f"    {line}", file=sys.stderr)
        failed = True
    else:
        print("  packet id cross-check: ok", file=sys.stderr)
    if component_problems:
        print(f"  COMPONENT CROSS-CHECK FAILED: {len(component_problems)}", file=sys.stderr)
        for line in component_problems[:10]:
            print(f"    {line}", file=sys.stderr)
        failed = True
    else:
        print("  component cross-check: ok", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
