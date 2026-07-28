#!/usr/bin/env python3
"""A scripted Minecraft 26.2 client, for proving the game server is joinable.

Not a bot framework. It answers the one question the proxy cannot: does a
client reach *play* state, or does it authenticate and then sit on "Joining
world..." forever because the server never sent it the Login packet? On 763
that meant watching for GameJoinS2c; on 776 there are two more gates before it,
the configuration state and the known-pack negotiation, and either one can
silently swallow a client.

Offline mode, protocol 776, which is what crates/hyperion/src/net/mod.rs
declares.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import socket
import struct
import sys
import time
import zlib

PROTOCOL = 776
VERSION = "26.2"

# The extracted protocol description, which is the same file `build.rs` reads
# and the same one every generated table comes out of.
PROTOCOL_JSON = (
    pathlib.Path(__file__).resolve().parent.parent
    / "crates/hyperion-minecraft-proto/protocol.json"
)


def registry_entries(registry: str) -> list[str]:
    """A vanilla registry's entry names, in network-id order.

    Read from `protocol.json` rather than scraped out of the generated Rust.
    Two tools used to do the latter, each with its own regex and its own
    defensive slice to skip the registry's own name -- which sits in the same
    table and looks exactly like an entry, so getting it wrong shifted every id
    by one and turned `brigadier:string` into `brigadier:long`. Both broke the
    day the generated table became a directory, because a path is not an
    interface. This is: the ids here are the indices, with nothing to skip.
    """
    doc = json.loads(PROTOCOL_JSON.read_text())
    found = doc["registries"].get(registry)
    if found is None:
        raise SystemExit(
            "%s has no %s; it has %d registries, starting %s"
            % (PROTOCOL_JSON, registry, len(doc["registries"]), sorted(doc["registries"])[:3])
        )
    return found["entries"]


# Serverbound ids, from crates/hyperion-minecraft-proto/src/generated/packet_id.rs.
C2S_INTENTION = 0x00
C2S_STATUS_REQUEST = 0x00
C2S_PING_REQUEST = 0x01
C2S_HELLO = 0x00
C2S_LOGIN_ACKNOWLEDGED = 0x03
C2S_CONFIG_CLIENT_INFORMATION = 0x00
C2S_CONFIG_FINISH = 0x03
C2S_CONFIG_KEEP_ALIVE = 0x04
C2S_CONFIG_SELECT_KNOWN_PACKS = 0x07
C2S_PLAY_ACCEPT_TELEPORTATION = 0x00
C2S_PLAY_KEEP_ALIVE = 0x1C
C2S_PLAY_MOVE_PLAYER_POS = 0x1E

# Clientbound ids, same source.
S2C_STATUS_RESPONSE = 0x00
S2C_PONG_RESPONSE = 0x01
S2C_LOGIN_DISCONNECT = 0x00
S2C_LOGIN_FINISHED = 0x02
S2C_LOGIN_COMPRESSION = 0x03
S2C_CONFIG_CUSTOM_PAYLOAD = 0x01
S2C_CONFIG_DISCONNECT = 0x02
S2C_CONFIG_FINISH = 0x03
S2C_CONFIG_KEEP_ALIVE = 0x04
S2C_CONFIG_REGISTRY_DATA = 0x07
S2C_CONFIG_UPDATE_ENABLED_FEATURES = 0x0C
S2C_CONFIG_UPDATE_TAGS = 0x0D
S2C_CONFIG_SELECT_KNOWN_PACKS = 0x0E
S2C_PLAY_DISCONNECT = 0x20
S2C_PLAY_GAME_EVENT = 0x26
S2C_PLAY_KEEP_ALIVE = 0x2C
S2C_PLAY_LEVEL_CHUNK = 0x2D
S2C_PLAY_LOGIN = 0x31
S2C_PLAY_PLAYER_POSITION = 0x48
S2C_PLAY_SET_CHUNK_CACHE_CENTER = 0x5E
S2C_PLAY_SET_DEFAULT_SPAWN = 0x61

# Every clientbound play id in protocol 776, generated from
# crates/hyperion-minecraft-proto/src/generated/packet_id.rs. The whole point
# of listing all of them is that an id the server sends which is NOT in here is
# proof the server is still speaking some other protocol's numbering: valence's
# 1.20.1 table tops out well inside this range, so a stray 763 packet does not
# announce itself by being out of bounds. It announces itself by being a 776
# packet nobody would send at that moment -- 0x1E DebugSample, say, where 763
# meant UnloadChunk.
PLAY_NAMES = {
    0x00: "BundleDelimiter",
    0x01: "AddEntity",
    0x02: "Animate",
    0x03: "AwardStats",
    0x04: "BlockChangedAck",
    0x05: "BlockDestruction",
    0x06: "BlockEntityData",
    0x07: "BlockEvent",
    0x08: "BlockUpdate",
    0x09: "BossEvent",
    0x0A: "ChangeDifficulty",
    0x0B: "ChunkBatchFinished",
    0x0C: "ChunkBatchStart",
    0x0D: "ChunksBiomes",
    0x0E: "ClearTitles",
    0x0F: "CommandSuggestions",
    0x10: "Commands",
    0x11: "ContainerClose",
    0x12: "ContainerSetContent",
    0x13: "ContainerSetData",
    0x14: "ContainerSetSlot",
    0x15: "CookieRequest",
    0x16: "Cooldown",
    0x17: "CustomChatCompletions",
    0x18: "CustomPayload",
    0x19: "DamageEvent",
    0x1A: "DebugBlockValue",
    0x1B: "DebugChunkValue",
    0x1C: "DebugEntityValue",
    0x1D: "DebugEvent",
    0x1E: "DebugSample",
    0x1F: "DeleteChat",
    0x20: "Disconnect",
    0x21: "DisguisedChat",
    0x22: "EntityEvent",
    0x23: "EntityPositionSync",
    0x24: "Explode",
    0x25: "ForgetLevelChunk",
    0x26: "GameEvent",
    0x27: "GameRuleValues",
    0x28: "GameTestHighlightPos",
    0x29: "MountScreenOpen",
    0x2A: "HurtAnimation",
    0x2B: "InitializeBorder",
    0x2C: "KeepAlive",
    0x2D: "LevelChunkWithLight",
    0x2E: "LevelEvent",
    0x2F: "LevelParticles",
    0x30: "LightUpdate",
    0x31: "Login",
    0x32: "LowDiskSpaceWarning",
    0x33: "MapItemData",
    0x34: "MerchantOffers",
    0x35: "MoveEntityPos",
    0x36: "MoveEntityPosRot",
    0x37: "MoveMinecartAlongTrack",
    0x38: "MoveEntityRot",
    0x39: "MoveVehicle",
    0x3A: "OpenBook",
    0x3B: "OpenScreen",
    0x3C: "OpenSignEditor",
    0x3D: "Ping",
    0x3E: "PongResponse",
    0x3F: "PlaceGhostRecipe",
    0x40: "PlayerAbilities",
    0x41: "PlayerChat",
    0x42: "PlayerCombatEnd",
    0x43: "PlayerCombatEnter",
    0x44: "PlayerCombatKill",
    0x45: "PlayerInfoRemove",
    0x46: "PlayerInfoUpdate",
    0x47: "PlayerLookAt",
    0x48: "PlayerPosition",
    0x49: "PlayerRotation",
    0x4A: "RecipeBookAdd",
    0x4B: "RecipeBookRemove",
    0x4C: "RecipeBookSettings",
    0x4D: "RemoveEntities",
    0x4E: "RemoveMobEffect",
    0x4F: "ResetScore",
    0x50: "ResourcePackPop",
    0x51: "ResourcePackPush",
    0x52: "Respawn",
    0x53: "RotateHead",
    0x54: "SectionBlocksUpdate",
    0x55: "SelectAdvancementsTab",
    0x56: "ServerData",
    0x57: "SetActionBarText",
    0x58: "SetBorderCenter",
    0x59: "SetBorderLerpSize",
    0x5A: "SetBorderSize",
    0x5B: "SetBorderWarningDelay",
    0x5C: "SetBorderWarningDistance",
    0x5D: "SetCamera",
    0x5E: "SetChunkCacheCenter",
    0x5F: "SetChunkCacheRadius",
    0x60: "SetCursorItem",
    0x61: "SetDefaultSpawnPosition",
    0x62: "SetDisplayObjective",
    0x63: "SetEntityData",
    0x64: "SetEntityLink",
    0x65: "SetEntityMotion",
    0x66: "SetEquipment",
    0x67: "SetExperience",
    0x68: "SetHealth",
    0x69: "SetHeldSlot",
    0x6A: "SetObjective",
    0x6B: "SetPassengers",
    0x6C: "SetPlayerInventory",
    0x6D: "SetPlayerTeam",
    0x6E: "SetScore",
    0x6F: "SetSimulationDistance",
    0x70: "SetSubtitleText",
    0x71: "SetTime",
    0x72: "SetTitleText",
    0x73: "SetTitlesAnimation",
    0x74: "SoundEntity",
    0x75: "Sound",
    0x76: "StartConfiguration",
    0x77: "StopSound",
    0x78: "StoreCookie",
    0x79: "SystemChat",
    0x7A: "TabList",
    0x7B: "TagQuery",
    0x7C: "TakeItemEntity",
    0x7D: "TeleportEntity",
    0x7E: "TestInstanceBlockStatus",
    0x7F: "TickingState",
    0x80: "TickingStep",
    0x81: "Transfer",
    0x82: "UpdateAdvancements",
    0x83: "UpdateAttributes",
    0x84: "UpdateMobEffect",
    0x85: "UpdateRecipes",
    0x86: "UpdateTags",
    0x87: "ProjectilePower",
    0x88: "CustomReportDetails",
    0x89: "ServerLinks",
    0x8A: "Waypoint",
    0x8B: "ClearDialog",
    0x8C: "ShowDialog",
}


def var_int(value):
    out = bytearray()
    value &= 0xFFFFFFFF
    while True:
        byte = value & 0x7F
        value >>= 7
        out.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(out)


def take_var_int(payload, offset=0):
    result = 0
    for shift in range(0, 35, 7):
        byte = payload[offset]
        offset += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, offset
    raise ValueError("var int too long")


def mc_string(text):
    raw = text.encode()
    return var_int(len(raw)) + raw


def take_string(payload, offset=0):
    length, offset = take_var_int(payload, offset)
    return payload[offset : offset + length].decode(), offset + length


# Tags are the one piece of configuration data this client can check the
# *meaning* of rather than just the framing, and the check matters: a server
# that sends an empty update_tags looks perfectly healthy on the wire and
# disconnects every real client. See referenced_tags below.
REGISTRY_DATA = (
    pathlib.Path(__file__).resolve().parent.parent
    / "crates"
    / "hyperion-minecraft-proto"
    / "src"
    / "registry_data"
)

# A HolderSet written by RegistryOps is either an element name, a list, or the
# tag's own name with a '#' in front, and only the last is a reference. The
# colon is what tells one from the hex colours the biome elements carry, which
# also start with '#'.
TAG_REFERENCE = re.compile(r"\A#([a-z0-9_.-]+:[a-z0-9_./-]+)\Z")

# NBT payload sizes, by tag type, for the fixed-width ones.
NBT_FIXED = {1: 1, 2: 2, 3: 4, 4: 8, 5: 4, 6: 8}


def nbt_skip(buf, offset, kind, strings):
    """Advance past one NBT payload, collecting every string it holds."""
    if kind in NBT_FIXED:
        return offset + NBT_FIXED[kind]
    if kind == 7:  # byte array
        (count,) = struct.unpack_from(">i", buf, offset)
        return offset + 4 + count
    if kind == 8:  # string
        (length,) = struct.unpack_from(">H", buf, offset)
        strings.append(buf[offset + 2 : offset + 2 + length].decode("utf-8", "replace"))
        return offset + 2 + length
    if kind == 9:  # list
        element = buf[offset]
        (count,) = struct.unpack_from(">i", buf, offset + 1)
        offset += 5
        for _ in range(count):
            offset = nbt_skip(buf, offset, element, strings)
        return offset
    if kind == 10:  # compound
        while True:
            entry = buf[offset]
            offset += 1
            if entry == 0:
                return offset
            (length,) = struct.unpack_from(">H", buf, offset)
            offset += 2 + length
            offset = nbt_skip(buf, offset, entry, strings)
    if kind == 11:  # int array
        (count,) = struct.unpack_from(">i", buf, offset)
        return offset + 4 + 4 * count
    if kind == 12:  # long array
        (count,) = struct.unpack_from(">i", buf, offset)
        return offset + 4 + 8 * count
    raise ValueError("unknown NBT tag type %d at offset %d" % (kind, offset))


def referenced_tags():
    """Every tag the registry elements a client parses will look up.

    Derived from the committed registry contents rather than listed, because a
    list of the names that happen to matter today is exactly what stops being
    right at the next version bump. The files are the network NBT of every
    element, back to back, so they are walked rather than indexed.
    """
    if not REGISTRY_DATA.is_dir():
        raise SystemExit("no registry contents at %s" % REGISTRY_DATA)

    referenced = set()
    for path in sorted(REGISTRY_DATA.glob("*.nbt")):
        buf = path.read_bytes()
        offset = 0
        while offset < len(buf):
            strings = []
            kind = buf[offset]
            offset = nbt_skip(buf, offset + 1, kind, strings)
            for text in strings:
                match = TAG_REFERENCE.match(text)
                if match:
                    referenced.add(match.group(1))
    if not referenced:
        raise SystemExit(
            "found no tag references in %s, so this check is reading the "
            "payloads wrong rather than the data being clean" % REGISTRY_DATA
        )
    return referenced


def check_tags(sent, log):
    """Fail when a tag a registry element names is not one the server sent.

    What this proves: the client could parse every registry element it kept
    from its own packs, which is the gate `finish_configuration` runs and the
    one an empty update_tags fails.

    What it does not prove: that each tag landed in the right registry, or that
    its ids point at the right elements. The reference in the NBT is a bare
    name and carries no registry, so presence is all that can be matched here.
    `cargo test -p hyperion-minecraft-proto --test tag_data` checks the ids.
    """
    names = {tag for tags in sent.values() for tag in tags}
    log(
        "tag map: %d registries, %d tags (%s)"
        % (len(sent), len(names), ", ".join(sorted(sent)) or "none")
    )

    failed = False
    for registry in ("minecraft:item", "minecraft:block", "minecraft:entity_type"):
        if not sent.get(registry):
            log("RESULT: no tags for %s, which every client needs" % registry)
            failed = True

    missing = sorted(referenced_tags() - names)
    if missing:
        log(
            "RESULT: %d tag(s) the registry contents reference were never sent, "
            "so a real client fails registry loading and disconnects with "
            "Network Protocol Error: %s" % (len(missing), ", ".join(missing[:8]))
        )
        failed = True
    else:
        log("RESULT: every referenced tag is present in UpdateTags")
    return not failed


def printable(payload, limit=180):
    text = "".join(chr(b) if 32 <= b < 127 else "." for b in payload)
    return text[:limit]


class Client:
    def __init__(self, host, port, name, log):
        self.sock = socket.create_connection((host, port))
        self.sock.settimeout(20)
        self.name = name
        self.threshold = -1
        self.log = log
        self.entity_id = None
        self.joined = False
        # The profile id the server minted, as hex. Offline mode makes this the
        # only thing distinguishing two players who typed the same name, so a
        # test about duplicate names has to be able to read it.
        self.profile_id = None
        # {registry: {tag name: [network ids]}}, as the server sent it.
        self.tags = {}

    def read_exact(self, count):
        buf = b""
        while len(buf) < count:
            chunk = self.sock.recv(count - len(buf))
            if not chunk:
                raise ConnectionError("server closed the connection")
            buf += chunk
        return buf

    def read_var_int_stream(self):
        result = 0
        for shift in range(0, 35, 7):
            byte = self.read_exact(1)[0]
            result |= (byte & 0x7F) << shift
            if not byte & 0x80:
                return result
        raise ValueError("var int too long")

    def send(self, packet_id, payload=b""):
        body = var_int(packet_id) + payload
        if self.threshold >= 0:
            if len(body) >= self.threshold:
                body = var_int(len(body)) + zlib.compress(body)
            else:
                body = var_int(0) + body
        self.sock.sendall(var_int(len(body)) + body)

    def recv(self):
        length = self.read_var_int_stream()
        body = self.read_exact(length)
        if self.threshold >= 0:
            size, offset = take_var_int(body)
            body = zlib.decompress(body[offset:]) if size else body[offset:]
        packet_id, offset = take_var_int(body)
        return packet_id, body[offset:]

    def handshake(self, host, port, intent):
        self.log(
            "-> Intention protocol=%d intent=%s"
            % (PROTOCOL, "status" if intent == 1 else "login")
        )
        self.send(
            C2S_INTENTION,
            var_int(PROTOCOL) + mc_string(host) + struct.pack(">H", port) + var_int(intent),
        )

    def status(self):
        self.send(C2S_STATUS_REQUEST)
        packet_id, payload = self.recv()
        if packet_id != S2C_STATUS_RESPONSE:
            raise SystemExit("status: expected 0x00, got 0x%02X" % packet_id)
        text, _ = take_string(payload)
        self.log("<- StatusResponse %s" % text)

        self.send(C2S_PING_REQUEST, struct.pack(">q", 0x1234))
        packet_id, payload = self.recv()
        if packet_id != S2C_PONG_RESPONSE:
            raise SystemExit("status: expected pong 0x01, got 0x%02X" % packet_id)
        (echoed,) = struct.unpack(">q", payload[:8])
        self.log("<- PongResponse time=0x%X" % echoed)
        return text

    def login(self):
        self.log("-> Hello name=%s" % self.name)
        self.send(C2S_HELLO, mc_string(self.name) + b"\x00" * 16)

        while True:
            packet_id, payload = self.recv()
            if packet_id == S2C_LOGIN_COMPRESSION:
                self.threshold, _ = take_var_int(payload)
                self.log("<- LoginCompression threshold=%d" % self.threshold)
            elif packet_id == S2C_LOGIN_FINISHED:
                name, _ = take_string(payload, 16)
                self.profile_id = payload[:16].hex()
                self.log(
                    "<- LoginFinished profile=%s name=%s "
                    "(authenticated, NOT yet in the world)" % (payload[:16].hex(), name)
                )
                self.log("-> LoginAcknowledged")
                self.send(C2S_LOGIN_ACKNOWLEDGED)
                return
            elif packet_id == S2C_LOGIN_DISCONNECT:
                reason, _ = take_string(payload)
                raise SystemExit("login refused: %s" % reason)
            else:
                self.log("<- login packet 0x%02X" % packet_id)

    def configuration(self):
        """Run the configuration state until the server says it is finished."""
        registries = []
        while True:
            packet_id, payload = self.recv()
            if packet_id == S2C_CONFIG_SELECT_KNOWN_PACKS:
                count, offset = take_var_int(payload)
                packs = []
                for _ in range(count):
                    namespace, offset = take_string(payload, offset)
                    ident, offset = take_string(payload, offset)
                    version, offset = take_string(payload, offset)
                    packs.append((namespace, ident, version))
                self.log("<- SelectKnownPacks %s" % packs)
                # Echo them back verbatim. A vanilla client reports the packs it
                # shipped with, and this client claims the same ones so the
                # server may leave registry contents out.
                body = var_int(len(packs))
                for namespace, ident, version in packs:
                    body += mc_string(namespace) + mc_string(ident) + mc_string(version)
                self.log("-> SelectKnownPacks (accepting all)")
                self.send(C2S_CONFIG_SELECT_KNOWN_PACKS, body)
                self.log("-> ClientInformation view_distance=10 skin_parts=0x7F")
                self.send(
                    C2S_CONFIG_CLIENT_INFORMATION,
                    mc_string("en_us")
                    + bytes([10])
                    + var_int(0)
                    + b"\x01"
                    + bytes([0x7F])
                    + var_int(1)
                    + b"\x01\x01"
                    + var_int(0),
                )
            elif packet_id == S2C_CONFIG_REGISTRY_DATA:
                name, offset = take_string(payload)
                count, offset = take_var_int(payload, offset)
                registries.append((name, count))
            elif packet_id == S2C_CONFIG_UPDATE_TAGS:
                # Decoded in full, not counted: an empty tag map is a
                # well-formed packet and a broken join, so the number of
                # registries is the one thing this packet cannot be judged on.
                registries_count, offset = take_var_int(payload)
                for _ in range(registries_count):
                    registry, offset = take_string(payload, offset)
                    tag_count, offset = take_var_int(payload, offset)
                    entries = {}
                    for _ in range(tag_count):
                        name, offset = take_string(payload, offset)
                        id_count, offset = take_var_int(payload, offset)
                        ids = []
                        for _ in range(id_count):
                            value, offset = take_var_int(payload, offset)
                            ids.append(value)
                        entries[name] = ids
                    self.tags[registry] = entries
                if offset != len(payload):
                    raise SystemExit(
                        "UpdateTags has %d trailing byte(s)" % (len(payload) - offset)
                    )
                self.log(
                    "<- UpdateTags %d registries, %d tags, %d bytes"
                    % (
                        registries_count,
                        sum(len(t) for t in self.tags.values()),
                        len(payload),
                    )
                )
            elif packet_id == S2C_CONFIG_UPDATE_ENABLED_FEATURES:
                count, offset = take_var_int(payload)
                names = []
                for _ in range(count):
                    feature, offset = take_string(payload, offset)
                    names.append(feature)
                self.log("<- UpdateEnabledFeatures %s" % names)
            elif packet_id == S2C_CONFIG_CUSTOM_PAYLOAD:
                channel, offset = take_string(payload)
                self.log("<- CustomPayload %s %s" % (channel, printable(payload[offset:], 40)))
            elif packet_id == S2C_CONFIG_KEEP_ALIVE:
                self.send(C2S_CONFIG_KEEP_ALIVE, payload[:8])
            elif packet_id == S2C_CONFIG_FINISH:
                total = sum(count for _, count in registries)
                self.log(
                    "<- RegistryData x%d (%d elements): %s"
                    % (len(registries), total, ", ".join(n for n, _ in registries))
                )
                self.log("<- FinishConfiguration")
                self.log("-> FinishConfiguration (ack)")
                self.send(C2S_CONFIG_FINISH)
                return
            elif packet_id == S2C_CONFIG_DISCONNECT:
                raise SystemExit("configuration refused: %s" % printable(payload))
            else:
                self.log("<- configuration packet 0x%02X len=%d %s" % (packet_id, len(payload), payload[:48].hex()))


# Packets a server has no reason to send between accepting a login and the
# player standing in the world. Each is a 1.20.1 id that a port leaves behind:
# 0x24 was chunk data, 0x1E was chunk unload, 0x28 and 0x7E are test harness
# packets a production server never sends at all.
NEVER_ON_JOIN = frozenset(
    {
        0x1A,  # DebugBlockValue
        0x1B,  # DebugChunkValue
        0x1C,  # DebugEntityValue
        0x1D,  # DebugEvent
        0x1E,  # DebugSample
        0x24,  # Explode
        0x28,  # GameTestHighlightPos
        0x32,  # LowDiskSpaceWarning
        0x7E,  # TestInstanceBlockStatus
        0x80,  # TickingStep
    }
)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument("--name", default="Prober")
    parser.add_argument("--seconds", type=float, default=15.0)
    parser.add_argument(
        "--status-only",
        action="store_true",
        help="do the server list ping and stop, the way a client showing the "
        "multiplayer menu does",
    )
    args = parser.parse_args()

    def log(line):
        print("[%s] %s" % (args.name, line), flush=True)

    client = Client(args.host, args.port, args.name, log)

    if args.status_only:
        client.handshake(args.host, args.port, 1)
        client.status()
        log("RESULT: status ok")
        return 0

    client.handshake(args.host, args.port, 2)
    client.login()
    client.configuration()

    # Checked here rather than after the play loop because it is the one thing
    # this client can say about configuration that a real client would also
    # say: reaching play proves the server finished configuration, not that a
    # client could have.
    tags_ok = check_tags(client.tags, log)

    started = time.time()
    deadline = started + args.seconds
    seen = {}
    moved = False
    lost_after_move = False

    while time.time() < deadline:
        try:
            packet_id, payload = client.recv()
        except (socket.timeout, TimeoutError):
            break
        except ConnectionError as error:
            log("connection lost: %s" % error)
            lost_after_move = moved
            break

        seen[packet_id] = seen.get(packet_id, 0) + 1

        if packet_id == S2C_PLAY_LOGIN:
            client.entity_id = struct.unpack(">i", payload[:4])[0]
            client.joined = True
            log("<- Login entity_id=%d  ** REACHED PLAY STATE **" % client.entity_id)
        elif packet_id == S2C_PLAY_SET_CHUNK_CACHE_CENTER:
            x, offset = take_var_int(payload)
            z, _ = take_var_int(payload, offset)
            log("<- SetChunkCacheCenter (%d, %d)" % (x, z))
        elif packet_id == S2C_PLAY_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            log(
                "<- PlayerPosition (%.1f, %.1f, %.1f) teleport_id=%d"
                % (x, y, z, teleport_id)
            )
            client.send(C2S_PLAY_ACCEPT_TELEPORTATION, var_int(teleport_id))

            # Walk. A client that only reads is a client that never exercises
            # the movement handler, and that handler is where a player missing
            # a component the server never added aborts the whole process
            # (hyperion#987). One step is enough to reach it: the check is
            # that the server is still there afterwards.
            client.send(
                C2S_PLAY_MOVE_PLAYER_POS,
                struct.pack(">dddb", x + 0.2, y, z, 1),
            )
            moved = True
            log("-> MovePlayerPos (%.1f, %.1f, %.1f)" % (x + 0.2, y, z))
        elif packet_id == S2C_PLAY_SET_DEFAULT_SPAWN:
            log("<- SetDefaultSpawnPosition")
        elif packet_id == S2C_PLAY_KEEP_ALIVE:
            client.send(C2S_PLAY_KEEP_ALIVE, payload[:8])
        elif packet_id == S2C_PLAY_DISCONNECT:
            log("<- Disconnect %s" % printable(payload))
            break

    log("packets received in play state, by id:")
    unknown = []
    for packet_id in sorted(seen):
        name = PLAY_NAMES.get(packet_id)
        if name is None:
            unknown.append(packet_id)
            name = "<NOT A 776 CLIENTBOUND PLAY ID>"
        log("    0x%02X %-28s x%d" % (packet_id, name, seen[packet_id]))

    if unknown:
        log(
            "RESULT: %d packet id(s) are not clientbound play ids in protocol "
            "776: %s" % (len(unknown), ", ".join("0x%02X" % i for i in unknown))
        )
        return 1

    # Every id in 0x00..0x8x is a valid 776 packet, so "the id decoded" proves
    # nothing on its own: a 1.20.1 chunk packet lands on 0x24 and reads as a
    # perfectly well-formed Explode. What gives the mismatch away is meaning.
    # A server that has just accepted a connection has no reason to detonate
    # anything or to emit a profiler sample, so seeing one of these is proof
    # the numbering is from some other protocol version.
    implausible = sorted(i for i in seen if i in NEVER_ON_JOIN)
    if implausible:
        for packet_id in implausible:
            log(
                "RESULT: %d x 0x%02X %s during a join, which no server sends. "
                "The wire is carrying another protocol's numbering."
                % (seen[packet_id], packet_id, PLAY_NAMES[packet_id])
            )
        return 1

    if not client.joined:
        log("RESULT: never reached play state (client would sit on 'Joining world...')")
        return 1

    if not moved:
        log("RESULT: never sent a movement packet, so the movement handler is untested")
        return 1

    if lost_after_move:
        log(
            "RESULT: the server dropped the connection after one step. A crash "
            "here is what a real player hits the moment they walk."
        )
        return 1

    # Reaching play is necessary but not sufficient. A vanilla client shows the
    # world only once it has terrain and has been told loading is done, so
    # those are checked separately rather than folded into one pass/fail.
    chunks = seen.get(S2C_PLAY_LEVEL_CHUNK, 0)
    loaded = seen.get(S2C_PLAY_GAME_EVENT, 0)
    log("RESULT: reached play state (Login received)")
    log("RESULT: %d LevelChunkWithLight packet(s)" % chunks)
    log(
        "RESULT: GameEvent(LEVEL_CHUNKS_LOAD_START) %s"
        % ("sent" if loaded else "NEVER SENT - client stays on the loading screen")
    )
    return 0 if chunks and loaded and tags_ok else 1


if __name__ == "__main__":
    sys.exit(main())
