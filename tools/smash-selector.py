#!/usr/bin/env python3
"""Drive real clients through the kit selector: click a mob, be refused, leave.

`smash-match.py` asks whether a match happens. This asks the question that comes
before it: can a player walk up to the ring of podiums in the middle of the hub,
right-click a mob and be playing it, and does the game answer honestly when the
mob is already somebody else's.

Nine claims, and each one is checked against what a client was actually sent
rather than against a return value:

  1. a right-click on a podium picks that mob;
  2. a second player clicking the same podium is refused, told who has it, and
     is left with no kit;
  3. the wool under the mob goes green to red on the wire when it is taken, and
     back to green when the holder disconnects, which frees the mob;
  4. a selection made in the hub survives the countdown, and a click after the
     match commits is refused;
  5. every mob in the ring arrives wearing its kit's name, always visible, so a
     player does not have to click one to learn what it is;
  6. a click plays the mob's own declared sound, to the player who clicked;
  7. and to that player alone, so fifteen people browsing the ring is not a
     wall of noise;
  8. all fifteen mobs answer in different voices, swept by clicking every one of
     them, because the failure mode is one kit quietly having none;
  9. the OTHER player sees the selecting player's skin change on the wire, and
     the selecting player is not respawned to make that happen.

Claim 9 is the one that reaches furthest outside this file. A skin change used
to be published to the wearer as `PlayerInfoRemove`, `PlayerInfoUpdate` and a
`Respawn`, and the respawn is what left a real client on "Loading terrain..."
forever: `handleRespawn` puts `ReceivingLevelScreen` back up and only a second
`LEVEL_CHUNKS_LOAD_START` takes it down. So this gate asserts both directions.
No respawn is sent, and if one ever is, terrain and that game event must follow
it. A test that counted chunk packets would have passed the bug.

Claim 9 also needs a client that behaves like a real one in a way no other
scripted client here does: a vanilla client puts its own profile id in
`ServerboundHelloPacket`, and hyperion answers that by asking Mojang for a skin
which arrives after the join. Every other tool in this directory sends sixteen
zero bytes and takes a different branch, which is exactly why the hang shipped
through five green gates. `--real-profile` is the default here.

Nothing here knows where a podium is. The ring is generated from the roster at
boot, so the server is asked: `/podiums` answers with one JSON object per
podium, and this file clicks the coordinates it is given. A gate that
recomputed the ring would be a second copy of `selector::ring` and would go on
passing after the real one moved.

The framing, login and configuration handshake are `client-26.2.py`'s. The
chunk and block-update decoders are `smash-map-check.py`'s. Both are imported
rather than copied, so there is one place each of those is written down.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import pathlib
import struct
import sys
import time
import uuid

TOOLS = pathlib.Path(__file__).resolve().parent


def _load(filename, name):
    """Import a tool whose file name is not a Python identifier."""
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


world = _load("smash-map-check.py", "smash_map_check")
match = _load("smash-match.py", "smash_match")
monitor = _load("packet_monitor.py", "packet_monitor")
base = world.base

var_int = base.var_int
take_var_int = base.take_var_int
mc_string = base.mc_string
take_nbt_string = match.take_nbt_string
BlockNames = world.BlockNames
ROOT = TOOLS.parent
decode_chunk = world.decode_chunk
decode_section_blocks_update = world.decode_section_blocks_update

# Serverbound ids, from crates/hyperion-minecraft-proto/src/generated/packet_id.rs.
C2S_ACCEPT_TELEPORTATION = 0x00
C2S_CHAT_COMMAND = 0x07
C2S_KEEP_ALIVE = 0x1C
C2S_INTERACT = 0x1A
C2S_MOVE_PLAYER_POS_ROT = 0x1F
C2S_USE_ITEM_ON = 0x42

S2C_DISCONNECT = world.S2C_DISCONNECT
S2C_LEVEL_CHUNK_WITH_LIGHT = world.S2C_LEVEL_CHUNK_WITH_LIGHT
S2C_SECTION_BLOCKS_UPDATE = world.S2C_SECTION_BLOCKS_UPDATE
S2C_KEEP_ALIVE = world.S2C_KEEP_ALIVE
S2C_LOGIN = world.S2C_LOGIN
S2C_PLAYER_POSITION = world.S2C_PLAYER_POSITION
S2C_ADD_ENTITY = 0x01
S2C_SET_ACTION_BAR_TEXT = 0x57
S2C_SYSTEM_CHAT = 0x79
S2C_SET_ENTITY_DATA = 0x63
S2C_SOUND = match.S2C_SOUND
S2C_PLAYER_INFO_REMOVE = 0x45
S2C_PLAYER_INFO_UPDATE = 0x46
S2C_RESPAWN = 0x52
S2C_GAME_EVENT = 0x26

# `ClientboundGameEventPacket.LEVEL_CHUNKS_LOAD_START`, which is what takes
# `ReceivingLevelScreen` down. See the module docstring.
LEVEL_CHUNKS_LOAD_START = 13

# `PlayerInfoActions` bit names live in packet_monitor.

# `net.minecraft.world.entity.Entity`'s field indices, from
# crates/hyperion/src/simulation/metadata/entity.rs, and the serializer ids
# those two fields were declared with, from `EntityDataSerializer`. The client
# rejects a value whose serializer disagrees, so reading them back is a check on
# both halves.
DATA_CUSTOM_NAME = 2
DATA_CUSTOM_NAME_VISIBLE = 3
SERIALIZER_OPTIONAL_COMPONENT = 6
SERIALIZER_BOOLEAN = 8

# `ClientboundSetEntityDataPacket.EOF_MARKER`.
METADATA_EOF = 0xFF

# events/smash/src/command.rs.
PODIUM_PREFIX = "smash-podium "
PODIUM_END_PREFIX = "smash-podiums-end "

# events/smash/src/module/selector.rs. Restated rather than imported so that
# this file checks the colours against the design and not against itself.
FREE_BLOCK = "minecraft:lime_wool"
TAKEN_BLOCK = "minecraft:red_wool"

# The roster that fills the lobby is `--full-clients` and not a constant here.
# It was `FULL_PLAYERS = 8`, under a comment naming `LobbyConfig::default` as
# where it came from, which is what made it a copy: #1019 moved that default to
# 4 and this number stayed at 8, still claiming to be it.

# How many lines a complete run reports. More than the nine claims, because two
# of the checks answer several: the roster sweep proves the sound, its audience
# and its distinctness, and the skin check proves both what the other client
# sees and what the wearer was spared.
CLAIMS = 13

ON_GROUND = 1
# `smash-match.py`'s numbers: how far a step may carry a client and how often
# one is sent. Anything faster is a step hyperion cannot account for, and it
# answers those with a teleport back to where the player was last tick.
POSITION_INTERVAL = 0.1
STEP_BLOCKS = 4.0
# Over the top of the hub's furniture rather than through it, which is also
# well clear of the podiums the client is walking between.
HUB_CLEAR_Y = 72.0

# `Direction::Up`, the face a player clicking the top of a block hits.
FACE_UP = 1


def entity_names():
    """Registry id to entity name, in network-id order.

    The same trick `smash-map-check.py` uses for blocks, and for the same
    reason: the numbering is Mojang's and restating it here would be a copy
    that is wrong the next time the jar moves. Reading what the server encodes
    from is what makes "that is a creeper" a check rather than a hope.

    From `protocol.json` rather than by regex over the generated Rust. The
    regex version matched `Self::new("...", n)`, which stopped existing the day
    entity types became a plain enum, and would have failed here rather than
    where the change was.
    """
    return dict(enumerate(base.registry_entries("minecraft:entity_type")))


def stamp(started):
    return "%7.2fs" % (time.time() - started)


# The committed skins, which are the only thing a texture property on the wire
# can honestly be checked against: the server reads these files with
# `include_str!`, so comparing against them is comparing against the same bytes
# rather than against a description of them.
SKINS = ROOT / "events" / "smash" / "skins"


def skin_payload(kit):
    """The committed `(textures, signature)` pair for a kit, or `None`.

    Keyed on the kit name lowercased with spaces squashed, which is how
    `kit_skin!("iron_golem")` names its files. `None` rather than an exception
    so a roster that grows a kit with no skin yet fails with the gate's own
    wording.
    """
    stem = kit.lower().replace(" ", "_")
    value = SKINS / (stem + ".value")
    signature = SKINS / (stem + ".sig")
    if not value.exists() or not signature.exists():
        return None
    return value.read_text().strip(), signature.read_text().strip()


# One `PlayerInfoUpdate` decoder for the whole tools directory; the copy
# that used to live here drifted from identity-check's. Both import the
# one in `packet_monitor.py` now.
parse_player_info_update = monitor.parse_player_info_update


def decode_entity_metadata(payload):
    """The two `Entity`-level fields this gate reads, out of one SetEntityData.

    Only the two, and every other entry is skipped by its serializer rather
    than decoded: a reader that had to understand all forty serializers to see
    a name would be a second copy of the protocol, and the entries this gate
    cares about are always the same two. An entry whose serializer is not one
    of the ones handled below ends the read, because the run is
    self-delimiting only if every length is known.
    """
    entity_id, offset = take_var_int(payload)
    fields = {}
    while offset < len(payload):
        index = payload[offset]
        offset += 1
        if index == METADATA_EOF:
            break
        serializer, offset = take_var_int(payload, offset)
        if serializer == SERIALIZER_OPTIONAL_COMPONENT:
            present = payload[offset]
            offset += 1
            if present:
                text, offset = take_nbt_string(payload, offset)
            else:
                text = None
            fields[index] = text
        elif serializer == SERIALIZER_BOOLEAN:
            fields[index] = bool(payload[offset])
            offset += 1
        else:
            # Anything else is a field this gate does not read, and the run
            # cannot be walked past a length it does not know.
            break
    return entity_id, fields


def block_pos(at):
    """`BlockPos.asLong`: x in the top 26 bits, z in the next 26, y in the low 12."""
    x, y, z = at
    return ((x & 0x3FFFFFF) << 38) | ((z & 0x3FFFFFF) << 12) | (y & 0xFFF)


class Client(base.Client):
    """One scripted player, non-blocking, with the blocks it has been sent."""

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, lambda line: None)
        self.started = started
        self.log = self._log
        self.buffer = b""
        self.position = (0.0, 65.0, 0.0)
        self.path = []
        self.yaw = 0.0
        self.pitch = 0.0
        self.joined = False
        self.entity_id = None
        self.kit = None
        self.action_bar = []
        self.chat = []
        self.podiums = []
        self.podiums_expected = None
        self.blocks = {}
        # Entity id -> (x, y, z), from every `add_entity` this client was sent.
        # How the gate finds a podium's mob: the server says where a podium is
        # and the mob standing there is the entity at that block.
        self.entities = {}
        # Entity id -> the `Entity`-level metadata fields this gate reads,
        # accumulated because a client is told a name in whichever packet the
        # server chose, at spawn or later.
        self.metadata = {}
        # Profile id -> the tab list entry, so a client can be asked what it
        # believes another player looks like. Merged rather than replaced: a
        # `PlayerInfoUpdate` that carries only a gamemode says nothing about a
        # profile's properties, and overwriting would forget a texture the
        # client still has.
        self.roster = {}
        self.sounds = []
        # The ordering evidence for claim 9. `respawns` counts them; the two
        # lists record what arrived after the most recent one, which is the only
        # thing that distinguishes a repaired respawn from a client stuck on a
        # loading screen with a full world behind it.
        self.respawns = 0
        self.chunks_since_respawn = 0
        self.level_loads_since_respawn = 0
        self.last_position_sent = 0.0
        self.gone = False

    def _log(self, line):
        print("%s [%-3s] %s" % (stamp(self.started), self.name, line), flush=True)

    def login(self):
        """`Hello` carrying a profile id, the way a vanilla client does.

        Overridden rather than inherited because `client-26.2.py` sends sixteen
        zero bytes, which puts hyperion on the branch that hands the player an
        empty skin synchronously. A real id sends it to the Mojang session
        server instead and the answer lands after the join, which is the branch
        that used to publish a `Respawn` at every real client and is therefore
        the only branch worth testing here. See the module docstring.
        """
        self.claimed = uuid.uuid4()
        self.log("-> Hello name=%s profile=%s" % (self.name, self.claimed))
        self.send(base.C2S_HELLO, mc_string(self.name) + self.claimed.bytes)
        while True:
            packet_id, payload = self.recv()
            if packet_id == base.S2C_LOGIN_COMPRESSION:
                self.threshold, _ = take_var_int(payload)
            elif packet_id == base.S2C_LOGIN_FINISHED:
                self.profile_id = payload[:16].hex()
                self.log("<- LoginFinished profile=%s" % self.profile_id)
                self.send(base.C2S_LOGIN_ACKNOWLEDGED)
                return
            elif packet_id == base.S2C_LOGIN_DISCONNECT:
                reason, _ = base.take_string(payload)
                raise SystemExit("login refused: %s" % reason)

    def texture_of(self, profile_id):
        """The `textures` property this client believes `profile_id` wears."""
        return self.roster.get(profile_id, {}).get("properties", {}).get("textures")

    def name_over(self, entity_id):
        """The always-visible name this client sees over `entity_id`, if any.

        `None` when there is no name, and `None` when there is one the client
        has been told not to draw: a `custom_name` without
        `custom_name_visible` renders only when the crosshair is on the mob,
        which is the failure the operator reported and would otherwise pass a
        check that read the name alone.
        """
        fields = self.metadata.get(entity_id, {})
        if not fields.get(DATA_CUSTOM_NAME_VISIBLE):
            return None
        return fields.get(DATA_CUSTOM_NAME)

    def enter_play(self):
        self.sock.settimeout(0.02)

    def drain(self):
        try:
            chunk = self.sock.recv(1 << 20)
            if not chunk:
                raise ConnectionError("server closed the connection")
            self.buffer += chunk
        except (TimeoutError, BlockingIOError):
            pass

        out = []
        while True:
            length = 0
            shift = 0
            used = 0
            complete = False
            while used < len(self.buffer) and used < 5:
                byte = self.buffer[used]
                used += 1
                length |= (byte & 0x7F) << shift
                shift += 7
                if not byte & 0x80:
                    complete = True
                    break
            if not complete or len(self.buffer) < used + length:
                return out
            body = self.buffer[used : used + length]
            self.buffer = self.buffer[used + length :]
            out.append(self.decode_body(body))

    def decode_body(self, body):
        if self.threshold >= 0:
            import zlib

            size, offset = take_var_int(body)
            body = zlib.decompress(body[offset:]) if size else body[offset:]
        packet_id, offset = take_var_int(body)
        return packet_id, body[offset:]

    # --- acting ---------------------------------------------------------

    def command(self, text):
        self.log("-> /%s" % text)
        self.send(C2S_CHAT_COMMAND, mc_string(text))

    def right_click_mob(self, entity_id, note=""):
        """`minecraft:interact`, which since 26.2 means a right-click.

        Layout from `ServerboundInteractPacket#STREAM_CODEC`: the entity, the
        hand, where on it the ray landed as an `LpVec3`, and whether the player
        was sneaking. A zero byte is the whole of an `LpVec3` whose components
        are all below `ABS_MIN_VALUE`, which is what a hit on the entity's own
        origin encodes to.
        """
        self.log("-> right-click entity %d %s" % (entity_id, note))
        self.send(
            C2S_INTERACT,
            var_int(entity_id) + var_int(0) + bytes([0]) + bytes([0]),
        )

    def right_click(self, at, note=""):
        """`use_item_on` with an empty hand, which is what picking a kit is.

        The offsets put the hit in the middle of the block's top face. Nothing
        on the server reads them; they are filled in because a packet a real
        client would never send is a packet this gate has no business sending.
        """
        self.log("-> right-click %s %s" % (at, note))
        self.send(
            C2S_USE_ITEM_ON,
            var_int(0)
            + struct.pack(">q", block_pos(at))
            + var_int(FACE_UP)
            + struct.pack(">fff", 0.5, 1.0, 0.5)
            + bytes([0, 0])
            + var_int(0),
        )

    def walk(self, destination):
        x, _y, z = self.position
        self.path = [
            (x, HUB_CLEAR_Y, z),
            (destination[0], HUB_CLEAR_Y, destination[2]),
            destination,
        ]

    def arrived(self):
        return not self.path

    def look_at(self, at):
        dx = at[0] - self.position[0]
        dz = at[2] - self.position[2]
        self.yaw = math.degrees(math.atan2(-dx, dz))
        self.pitch = 0.0

    def repeat_position(self):
        now = time.time()
        if now - self.last_position_sent < POSITION_INTERVAL:
            return
        self.last_position_sent = now

        if self.path:
            x, y, z = self.position
            tx, ty, tz = self.path[0]
            dx, dy, dz = tx - x, ty - y, tz - z
            gap = math.sqrt(dx * dx + dy * dy + dz * dz)
            if gap <= STEP_BLOCKS:
                self.position = self.path.pop(0)
            else:
                scale = STEP_BLOCKS / gap
                self.position = (x + dx * scale, y + dy * scale, z + dz * scale)

        x, y, z = self.position
        self.send(
            C2S_MOVE_PLAYER_POS_ROT,
            struct.pack(">dddffb", x, y, z, self.yaw, self.pitch, ON_GROUND),
        )

    def leave(self):
        """Drop the connection the way a player who alt-F4s does."""
        self.log("-- disconnecting")
        self.gone = True
        try:
            self.sock.close()
        except OSError:
            pass

class Run:
    """The session: a handful of clients, one transcript, and a verdict."""

    def __init__(self, args):
        self.args = args
        self.started = time.time()
        self.clients = []
        self.names = BlockNames()
        self.kinds = entity_names()
        self.failures = []
        self.proved = []
        self.phase = "waiting"

    # --- plumbing -------------------------------------------------------

    def log(self, line):
        print("%s      %s" % (stamp(self.started), line), flush=True)

    def prove(self, claim, evidence):
        self.proved.append((claim, evidence))
        self.log("PROVED %-52s %s" % (claim, evidence))

    def fail(self, why):
        self.failures.append(why)
        self.log("FAILED %s" % why)

    def hub_only(self, during):
        """Whether the lobby is still unstarted, which the hub checks need.

        The wording is `smash-match.py`'s, which hit this first and owns the
        sentence. Three clients used to sit below `min_players` and now sit
        above it, so these checks run with a countdown underneath them rather
        than in a hub that will wait, and a claim made while the match commits
        is a claim about the wrong game.

        Reading the phase off the server's own broadcasts rather than
        recomputing a threshold is what keeps this right whatever the numbers
        become, and is what keeps it from being one more copy of them.
        """
        if self.phase == "waiting":
            return True
        self.fail(match.hub_lost(during, len(self.live())))
        return False

    def connect(self, count):
        for _ in range(count):
            name = "S%d" % (len(self.clients) + 1)
            client = Client(self.args.host, self.args.port, name, self.started)
            client.handshake(self.args.host, self.args.port, 2)
            client.login()
            client.configuration()
            client.enter_play()
            self.clients.append(client)
        return self.clients[-count:]

    def live(self):
        return [client for client in self.clients if not client.gone]

    def pump(self):
        for client in self.live():
            try:
                for packet_id, payload in client.drain():
                    self.handle(client, packet_id, payload)
                if client.joined:
                    client.repeat_position()
            except (ConnectionError, OSError) as error:
                client.log("connection ended: %s" % error)
                client.gone = True

    def wait_until(self, predicate, seconds, why=""):
        deadline = time.time() + seconds
        while time.time() < deadline:
            self.pump()
            if predicate():
                return True
            time.sleep(0.01)
        self.pump()
        if predicate():
            return True
        if why:
            self.fail("timed out after %.0fs waiting for %s" % (seconds, why))
        return False

    def settle(self, seconds):
        deadline = time.time() + seconds
        while time.time() < deadline:
            self.pump()
            time.sleep(0.01)

    def handle(self, client, packet_id, payload):
        if packet_id == S2C_LOGIN:
            client.entity_id = struct.unpack(">i", payload[:4])[0]
            client.joined = True
            client.log("** in the world ** entity_id=%d" % client.entity_id)
        elif packet_id == S2C_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            client.position = (x, y, z)
            client.path = []
            client.send(C2S_ACCEPT_TELEPORTATION, var_int(teleport_id))
        elif packet_id == S2C_KEEP_ALIVE:
            client.send(C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == S2C_SYSTEM_CHAT:
            text, _ = take_nbt_string(payload, 0)
            self.on_chat(client, text)
        elif packet_id == S2C_SET_ACTION_BAR_TEXT:
            text, _ = take_nbt_string(payload, 0)
            client.action_bar.append(text)
            client.log("<- action bar: %s" % text)
        elif packet_id == S2C_ADD_ENTITY:
            # `ClientboundAddEntityPacket`, from
            # crates/hyperion-minecraft-proto/src/packets/play/entity.rs. Only
            # the id and the position are read; the rest is skipped rather than
            # decoded because nothing here asks what a mob looks like.
            entity_id, offset = take_var_int(payload)
            offset += 16  # uuid
            kind, offset = take_var_int(payload, offset)
            x, y, z = struct.unpack_from(">ddd", payload, offset)
            client.entities[entity_id] = ((x, y, z), self.kinds.get(kind, "<type %d>" % kind))
        elif packet_id == S2C_SET_ENTITY_DATA:
            entity_id, fields = decode_entity_metadata(payload)
            client.metadata.setdefault(entity_id, {}).update(fields)
        elif packet_id == S2C_SOUND:
            heard = match.take_sound(payload)
            if heard is None:
                self.fail(
                    "a sound arrived as a registry id; hyperion sends no id "
                    "table, so a client would resolve it against the wrong "
                    "registry"
                )
            else:
                client.sounds.append(heard)
                client.log("<- sound %s" % heard[0])
        elif packet_id == S2C_PLAYER_INFO_UPDATE:
            try:
                _actions, entries = parse_player_info_update(payload)
            except ValueError as error:
                self.fail("could not read a PlayerInfoUpdate: %s" % error)
            else:
                for entry in entries:
                    known = client.roster.setdefault(entry["uuid"], {"properties": {}})
                    # Merged field by field rather than wholesale, so a packet
                    # that carries only a gamemode does not erase the texture
                    # the client was told about earlier.
                    for key, value in entry.items():
                        if key == "properties":
                            known["properties"].update(value)
                        else:
                            known[key] = value
        elif packet_id == S2C_RESPAWN:
            client.respawns += 1
            client.chunks_since_respawn = 0
            client.level_loads_since_respawn = 0
            client.log("<- RESPAWN (the client has just discarded its world)")
        elif packet_id == S2C_GAME_EVENT:
            if payload and payload[0] == LEVEL_CHUNKS_LOAD_START:
                client.level_loads_since_respawn += 1
        elif packet_id == S2C_LEVEL_CHUNK_WITH_LIGHT:
            client.chunks_since_respawn += 1
            _at, blocks = decode_chunk(payload, (60, 70))
            client.blocks.update(blocks)
        elif packet_id == S2C_SECTION_BLOCKS_UPDATE:
            _at, changes = decode_section_blocks_update(payload)
            for position, state in changes:
                client.blocks[position] = state
        elif packet_id == S2C_DISCONNECT:
            text, _ = take_nbt_string(payload, 0)
            client.log("<- DISCONNECTED: %s" % text)
            client.gone = True

    def on_chat(self, client, text):
        if text.startswith(PODIUM_PREFIX):
            client.podiums.append(json.loads(text[len(PODIUM_PREFIX) :]))
            return
        if text.startswith(PODIUM_END_PREFIX):
            client.podiums_expected = int(text[len(PODIUM_END_PREFIX) :])
            return
        client.log("<- chat: %s" % text)
        if text.startswith("Kit set to "):
            client.kit = text[len("Kit set to ") :].rstrip(".")
            return
        if "The game starts shortly" in text:
            self.phase = "countdown"
        elif "Get ready" in text:
            self.phase = "preparing"
        elif text.strip().endswith("Go!"):
            self.phase = "playing"
            self.log("== the match is running ==")
        elif "Game over" in text:
            self.phase = "ended"

    # --- reading the world ----------------------------------------------

    def ask_podiums(self, client):
        """What the server says the ring is, right now."""
        client.podiums = []
        client.podiums_expected = None
        client.command("podiums")
        got = self.wait_until(
            lambda: client.podiums_expected is not None
            and len(client.podiums) >= client.podiums_expected,
            15.0,
            "the server to answer /podiums",
        )
        if not got:
            return []
        return client.podiums

    @staticmethod
    def offer(podiums, kit):
        for entry in podiums:
            if entry["kit"] == kit:
                return entry
        return None

    @staticmethod
    def click_at(entry):
        return (entry["x"], entry["y"], entry["z"])

    @staticmethod
    def base_at(entry):
        return (entry["x"], entry["base_y"], entry["z"])

    def mob_of(self, client, entry):
        """The entity id of the mob standing on that podium, as this client
        has been told about it.

        Matched on position, because that is the only thing the two sides
        share: `/podiums` says which block the mob stands in and `add_entity`
        says where each entity is. The server puts a mob in the middle of its
        block, so the match is against the block the entity's position falls
        in rather than against the position itself.
        """
        wanted = self.click_at(entry)
        for entity_id, (at, _kind) in client.entities.items():
            if (
                math.floor(at[0]) == wanted[0]
                and math.floor(at[1]) == wanted[1]
                and math.floor(at[2]) == wanted[2]
            ):
                return entity_id
        return None

    def kind_of(self, client, entry):
        """What the client was told the podium's mob is."""
        entity_id = self.mob_of(client, entry)
        if entity_id is None:
            return None
        return client.entities[entity_id][1]

    def wool_seen_by(self, client, entry):
        """The block the client has actually been sent for that podium's wool."""
        state = client.blocks.get(self.base_at(entry))
        if state is None:
            return None
        return self.names.name(state)

    def approach(self, client, entry):
        """Walk to the podium, so the click comes from somewhere a player is."""
        x, y, z = self.click_at(entry)
        stand = (x + 1.5, float(y) - 1.0, z + 1.5)
        client.walk(stand)
        self.wait_until(client.arrived, 20.0)
        client.look_at((x, y, z))
        self.settle(0.2)

    def pick(self, client, entry):
        """Right-click the podium's mob, which is how a player picks a kit.

        The mob is the surface this whole feature is about, so the gate uses
        it, and a run where no mob ever arrived is a failure rather than a
        quiet fall back to the wool. The wool is clicked anyway afterwards so
        the rest of the script still has something to check, but the run has
        already been marked failed by then.
        """
        self.approach(client, entry)
        client.action_bar.clear()
        entity_id = self.mob_of(client, entry)
        if entity_id is None:
            self.fail(
                "%s was never sent a mob standing on the %s podium at %s, so "
                "there was nothing to right-click"
                % (client.name, entry["kit"], self.click_at(entry))
            )
            client.right_click(self.click_at(entry), "(%s, wool)" % entry["kit"])
            return
        client.right_click_mob(entity_id, "(the %s)" % entry["kit"])
    # --- the script -----------------------------------------------------

    def run(self):
        # Three because the checks below need three named roles -- a holder, a
        # watcher, and somebody to take a freed mob -- and not because of any
        # threshold. It used to say "one short of `min_players`", which was a
        # second copy of `LobbyConfig::default` and stopped being true when
        # #1019 moved it: three is now one *over* the minimum, and a countdown
        # starts under these checks rather than after them.
        #
        # The lobby has to stay unstarted for all of them even so, which is the
        # server's business and is checked with `hub_only` rather than computed
        # here. `selection_survives_the_countdown` at the end wants the exact
        # opposite and fills the lobby deliberately.
        one, two, three = self.connect(3)
        if not self.wait_until(
            lambda: all(client.joined for client in self.clients),
            60.0,
            "three clients to reach the world",
        ):
            return self.report()

        if not self.hub_only("before the hub checks began"):
            return self.report()

        self.the_ring_exists(one)
        self.every_mob_wears_its_name(one)
        # Before anything is claimed, because the sweep clicks all fifteen and
        # every later check wants a ring nobody has touched. `two` watches, so
        # the skin half is answered by a client that is not the one selecting.
        self.every_mob_answers_in_its_own_voice(one, two)
        self.a_skin_change_costs_nobody_their_world(one, two)
        # Again before anything is claimed. The reads above cost real seconds,
        # and a countdown that started during them makes every claim below a
        # claim about a lobby that is no longer taking them.
        if not self.hub_only("after reading the ring, before claiming a mob"):
            return self.report()
        taken = self.a_click_picks_the_mob(one)
        if taken is None:
            return self.report()
        self.the_wool_turns_red(one, two, taken)
        self.a_taken_mob_is_refused(two, taken, one.name)
        self.a_free_mob_is_not(two, taken)
        self.a_holder_who_leaves_frees_it(one, two, three, taken)
        self.selection_survives_the_countdown()
        return self.report()

    def every_mob_wears_its_name(self, client):
        """Claim 5: a player can read the ring without clicking any of it.

        Every podium, because the failure is one blank mob among fourteen
        labelled ones. The expected string is the server's own
        `Offer::label`, so this is a check that the name the client renders is
        the name the game decided, and not a check against fifteen kit names
        copied into Python.
        """
        podiums = self.ask_podiums(client)
        if not podiums:
            self.fail("the server named no podiums, so no mob could wear a name")
            return

        # Waited for rather than asserted: a mob's metadata rides on the
        # subscribe packets the proxy asks for when the client comes into range,
        # which is a tick or two after the chunk it stands in.
        self.wait_until(
            lambda: all(
                client.name_over(self.mob_of(client, entry)) is not None
                for entry in podiums
                if self.mob_of(client, entry) is not None
            )
            and all(self.mob_of(client, entry) is not None for entry in podiums),
            30.0,
        )

        wrong = []
        for entry in podiums:
            mob = self.mob_of(client, entry)
            if mob is None:
                wrong.append("%s has no mob at %s" % (entry["kit"], self.click_at(entry)))
                continue
            seen = client.name_over(mob)
            if seen is None:
                fields = client.metadata.get(mob, {})
                wrong.append(
                    "%s's mob wears no always-visible name; its metadata is %r"
                    % (entry["kit"], fields)
                )
            elif seen != entry["label"]:
                wrong.append(
                    "%s's mob is captioned %r and the server said %r"
                    % (entry["kit"], seen, entry["label"])
                )
        if wrong:
            self.fail("the ring is not readable without clicking it: %s" % "; ".join(wrong))
            return
        self.prove(
            "every mob wears its kit's name, always visible",
            "%d of %d, e.g. %r over the %s"
            % (
                len(podiums),
                len(podiums),
                podiums[0]["label"],
                podiums[0]["mob"],
            ),
        )

    def every_mob_answers_in_its_own_voice(self, chooser, bystander):
        """Claims 6, 7 and 8: the sweep over the whole roster.

        One client can hold every mob in turn, because `lobby::choose` only
        refuses a mob *somebody else* has, so fifteen clicks by one player is a
        legal sequence and is the cheapest way to hear all fifteen voices on a
        real wire.

        Each sound is held against the `select_sound` the server published for
        that podium rather than against a table here, so this cannot pass by
        agreeing with a copy of the roster that has gone stale.
        """
        podiums = self.ask_podiums(chooser)
        if not podiums:
            self.fail("the server named no podiums, so no mob could answer")
            return

        missing = [entry["kit"] for entry in podiums if not entry["select_sound"]]
        if missing:
            self.fail("these kits declare no selection sound: %r" % missing)
            return

        heard_for = {}
        overheard = []
        for entry in podiums:
            self.approach(chooser, entry)
            mob = self.mob_of(chooser, entry)
            if mob is None:
                self.fail("no mob stands on the %s podium" % entry["kit"])
                return
            chooser.sounds = []
            bystander.sounds = []
            chooser.right_click_mob(mob, "sweep %s" % entry["kit"])
            wanted = entry["select_sound"]
            self.wait_until(
                lambda wanted=wanted: any(sound[0] == wanted for sound in chooser.sounds),
                6.0,
            )
            ids = [sound[0] for sound in chooser.sounds]
            if wanted not in ids:
                self.fail(
                    "clicking the %s declares %s and played %r"
                    % (entry["kit"], wanted, ids)
                )
                return
            heard_for[entry["kit"]] = wanted
            if bystander.sounds:
                overheard.append(
                    "%s was audible to %s: %r"
                    % (entry["kit"], bystander.name, [s[0] for s in bystander.sounds])
                )

        self.prove(
            "a click plays the mob's own declared sound",
            "%d of %d kits answered, e.g. %s from the %s"
            % (
                len(heard_for),
                len(podiums),
                heard_for[podiums[0]["kit"]],
                podiums[0]["kit"],
            ),
        )

        if overheard:
            self.fail(
                "a selection was heard by somebody who did not make it, which a "
                "lobby of eight people browsing the ring would turn into a wall "
                "of noise: %s" % "; ".join(overheard)
            )
        else:
            self.prove(
                "and only the player who clicked hears it",
                "%s heard none of %d selections" % (bystander.name, len(podiums)),
            )

        duplicates = {}
        for kit, sound in heard_for.items():
            duplicates.setdefault(sound, []).append(kit)
        shared = {
            sound: kits for sound, kits in duplicates.items() if len(kits) > 1
        }
        if shared:
            self.fail("these kits answer a click alike: %r" % shared)
        else:
            self.prove(
                "all fifteen mobs answer in different voices",
                "%d distinct sounds over %d kits" % (len(duplicates), len(heard_for)),
            )

    def a_skin_change_costs_nobody_their_world(self, chooser, watcher):
        """Claim 9: the other client sees the new skin, and nobody respawns.

        The skin is the point of the feature and the respawn is the bug that
        shipped with it. Both are read off `watcher` and `chooser` rather than
        inferred: a texture property in somebody else's tab list entry is the
        only evidence that a change reached the client that renders the model,
        and a respawn count is the only evidence that it did not cost that
        client its terrain.
        """
        # Any free mob except the first podium, which the checks after this one
        # take for themselves. Leaving `chooser` already holding that mob would
        # make the next check a no-op: it clicks and waits for the kit to
        # change, which it never does, and reads an action bar that was never
        # written. That is a fault in the order these checks run in and not in
        # the game, but it reads exactly like one.
        podiums = self.ask_podiums(chooser)
        free = [
            entry
            for entry in podiums
            if entry["held_by"] is None and entry["kit"] != podiums[0]["kit"]
        ]
        if not free:
            self.fail("no mob is free, so no skin could change")
            return
        entry = free[0]

        before = watcher.texture_of(chooser.profile_id)
        chooser_respawns = chooser.respawns

        self.approach(chooser, entry)
        mob = self.mob_of(chooser, entry)
        if mob is None:
            self.fail("no mob stands on the %s podium" % entry["kit"])
            return
        chooser.right_click_mob(mob, "dress as %s" % entry["kit"])

        self.wait_until(
            lambda: watcher.texture_of(chooser.profile_id) not in (None, before),
            20.0,
            "%s to be told what %s now looks like" % (watcher.name, chooser.name),
        )
        after = watcher.texture_of(chooser.profile_id)
        if after is None or after == before:
            self.fail(
                "%s picked the %s and %s was never told: the texture it holds is "
                "still %r" % (chooser.name, entry["kit"], watcher.name, before)
            )
            return

        value, signature = after
        expected = skin_payload(entry["kit"])
        if expected is None:
            self.fail("no committed skin for the %s, so nothing could be checked" % entry["kit"])
            return
        if value != expected[0]:
            self.fail(
                "%s sees a texture that is not the one committed for the %s"
                % (watcher.name, entry["kit"])
            )
            return
        if signature != expected[1]:
            self.fail(
                "the texture %s sees carries %s, and without the committed Mojang "
                "signature the client would show it to nobody but its wearer"
                % (watcher.name, "no signature" if signature is None else "a different signature")
            )
            return
        self.prove(
            "the other player sees the new skin on the wire",
            "%s now holds the committed %s payload for %s, signed"
            % (watcher.name, entry["kit"], chooser.name),
        )

        # The ordering claim. Both halves, because a respawn is legitimate as
        # long as it is repaired, and the failure that shipped was a respawn
        # nobody repaired.
        self.settle(1.5)
        if chooser.respawns == chooser_respawns:
            self.prove(
                "and it did not cost the wearer its world",
                "%s was sent no Respawn while its skin changed" % chooser.name,
            )
        elif chooser.level_loads_since_respawn and chooser.chunks_since_respawn:
            self.prove(
                "and the respawn it took gave the world back",
                "%d chunk columns and %d LEVEL_CHUNKS_LOAD_START after the respawn"
                % (chooser.chunks_since_respawn, chooser.level_loads_since_respawn),
            )
        else:
            self.fail(
                "a skin change respawned %s and left it there: %d chunk columns "
                "and %d LEVEL_CHUNKS_LOAD_START arrived afterwards, so a real "
                "client sits on ReceivingLevelScreen forever"
                % (
                    chooser.name,
                    chooser.chunks_since_respawn,
                    chooser.level_loads_since_respawn,
                )
            )

    def the_ring_exists(self, client):
        podiums = self.ask_podiums(client)
        if not podiums:
            self.fail("the server named no podiums at all")
            return
        free = [entry for entry in podiums if entry["held_by"] is None]
        if len(free) != len(podiums):
            self.fail("somebody already holds a mob in an empty lobby: %r" % podiums)
            return
        wools = {entry["wool"] for entry in podiums}
        if wools != {FREE_BLOCK}:
            self.fail("a free podium is not %s: %r" % (FREE_BLOCK, wools))
            return

        # The blocks the server said are there against the blocks it sent.
        # Everything below reads the manifest; this is the one check that the
        # manifest describes the world a player is standing in.
        #
        # Waited for rather than asserted: the hub arrives as an empty chunk
        # and then seventy-odd thousand block changes layered on top of it, so
        # a podium is briefly air on a client that has only just joined.
        self.wait_until(
            lambda: all(
                self.wool_seen_by(client, entry) == FREE_BLOCK for entry in podiums
            ),
            30.0,
        )
        mismatched = []
        for entry in podiums:
            seen = self.wool_seen_by(client, entry)
            if seen != FREE_BLOCK:
                mismatched.append(
                    "%s at %s is %s" % (entry["kit"], self.base_at(entry), seen)
                )
        if mismatched:
            self.fail(
                "the podiums the server described are not the blocks it sent: %s"
                % "; ".join(mismatched)
            )
            return

        # And a real mob standing on each of them, which is the whole feature:
        # a player picks a kit by right-clicking the mob, so a ring of wool
        # with nothing on it is a ring nobody can use.
        self.wait_until(
            lambda: all(self.mob_of(client, entry) is not None for entry in podiums),
            30.0,
        )
        empty = [
            entry["kit"] for entry in podiums if self.mob_of(client, entry) is None
        ]
        if empty:
            self.fail("these podiums have no mob standing on them: %s" % ", ".join(empty))
            return

        # And the mob the kit says it is. Fifteen armour stands would satisfy
        # every check above and would not be the feature.
        wrong = [
            "%s stands on a %s" % (entry["kit"], self.kind_of(client, entry))
            for entry in podiums
            if self.kind_of(client, entry) != entry["mob"]
        ]
        if wrong:
            self.fail("the wrong mob is standing on these podiums: %s" % "; ".join(wrong))
            return

        self.prove(
            "the hub has a podium per mob, with that mob standing on it",
            "%d podiums, every one free, every one standing on %s in the chunks "
            "the client was sent, and each one carrying the entity its kit names "
            "(%s)"
            % (
                len(podiums),
                FREE_BLOCK,
                ", ".join(sorted(entry["mob"] for entry in podiums)),
            ),
        )

    def a_click_picks_the_mob(self, client):
        podiums = self.ask_podiums(client)
        if not podiums:
            return None
        entry = podiums[0]
        self.pick(client, entry)

        if not self.wait_until(
            lambda: client.kit == entry["kit"],
            10.0,
            "%s to be playing the %s" % (client.name, entry["kit"]),
        ):
            return None
        # And then for the announcement, separately. `client.kit` is read off
        # the chat line, which `lobby::choose` sends one packet ahead of the
        # action bar, and `drain` takes a single `recv` per pump: a burst split
        # across two reads satisfies the wait above while the action bar is
        # still on its way. Run 30500640846 recorded the failure at
        # 00:21:41.7509285 and logged the line it was waiting for at
        # 00:21:41.7511018. Waiting on the packet the claim is about, rather
        # than on the one that happens to precede it, is the whole fix: the
        # bytes had not reached the client, so no amount of draining harder
        # would have found them.
        self.wait_until(
            lambda: any(entry["kit"] in line for line in client.action_bar), 10.0
        )
        told = [line for line in client.action_bar if entry["kit"] in line]
        if not told:
            self.fail(
                "%s picked the %s and the action bar never said so: %r"
                % (client.name, entry["kit"], client.action_bar)
            )
            return entry
        self.prove(
            "a right-click on a podium picks that mob",
            "%s right-clicked %s and the server answered %r"
            % (client.name, self.click_at(entry), told),
        )
        return entry

    def the_wool_turns_red(self, holder, watcher, entry):
        base = self.base_at(entry)
        if not self.wait_until(
            lambda: self.wool_seen_by(watcher, entry) == TAKEN_BLOCK,
            10.0,
            "the wool under the %s to go red for %s" % (entry["kit"], watcher.name),
        ):
            self.log(
                "%s sees %s at %s"
                % (watcher.name, self.wool_seen_by(watcher, entry), base)
            )
            return
        # Only that one. A repaint that reddened the whole ring would pass a
        # check that looked at one block.
        podiums = self.ask_podiums(watcher)
        red = [
            other["kit"]
            for other in podiums
            if self.wool_seen_by(watcher, other) == TAKEN_BLOCK
        ]
        if red != [entry["kit"]]:
            self.fail("one mob was taken and these podiums are red: %r" % red)
            return
        self.prove(
            "a taken mob is red without anybody reading anything",
            "%s took the %s and %s was sent %s at %s, the only red podium in "
            "the ring" % (holder.name, entry["kit"], watcher.name, TAKEN_BLOCK, base),
        )

    def a_taken_mob_is_refused(self, client, entry, holder):
        self.pick(client, entry)
        self.settle(1.0)

        if client.kit is not None:
            self.fail(
                "%s clicked a mob %s already had and ended up playing %s"
                % (client.name, holder, client.kit)
            )
            return
        told = [
            line
            for line in client.action_bar
            if entry["kit"] in line and holder in line
        ]
        if not told:
            self.fail(
                "%s was refused the %s and never told who has it: %r"
                % (client.name, entry["kit"], client.action_bar)
            )
            return
        self.prove(
            "a mob somebody else has is refused, by name",
            "%s clicked the %s and was told %r" % (client.name, entry["kit"], told[0]),
        )

    def a_free_mob_is_not(self, client, taken):
        podiums = self.ask_podiums(client)
        free = [entry for entry in podiums if entry["held_by"] is None]
        if not free:
            self.fail("every mob is taken with one player holding one")
            return
        entry = free[0]
        self.pick(client, entry)
        if not self.wait_until(
            lambda: client.kit == entry["kit"],
            10.0,
            "%s to be playing the %s" % (client.name, entry["kit"]),
        ):
            return
        self.prove(
            "the refusal is about that mob and not about clicking",
            "%s was refused the %s and then took the %s from the next podium "
            "along" % (client.name, taken["kit"], entry["kit"]),
        )
    def a_holder_who_leaves_frees_it(self, holder, watcher, spare, entry):
        """The claim is the holder's `(Playing, kit)` edge and nothing else.

        Nothing in the server frees a mob on disconnect, because there is
        nothing to free: destroying the player destroys the edge that was the
        claim. This is the check that says so from outside, where a forgotten
        cleanup handler would be visible as a mob nobody can ever take again.
        """
        holder.leave()

        if not self.wait_until(
            lambda: self.wool_seen_by(watcher, entry) == FREE_BLOCK,
            20.0,
            "the %s to go back to %s after %s left" % (entry["kit"], FREE_BLOCK, holder.name),
        ):
            return
        podiums = self.ask_podiums(watcher)
        again = self.offer(podiums, entry["kit"])
        if again is None or again["held_by"] is not None:
            self.fail(
                "%s left and the %s is still held by %r"
                % (holder.name, entry["kit"], again and again["held_by"])
            )
            return

        self.pick(spare, entry)
        if not self.wait_until(
            lambda: spare.kit == entry["kit"],
            10.0,
            "%s to take the mob %s left behind" % (spare.name, holder.name),
        ):
            return
        self.prove(
            "a holder who disconnects frees their mob at once",
            "%s left, the podium went back to %s, and %s took the %s"
            % (holder.name, FREE_BLOCK, spare.name, entry["kit"]),
        )

    def selection_survives_the_countdown(self):
        """Fill the lobby, let it commit, and see whether the picks held.

        `full_players` rather than `min_players` because the countdown at a
        full lobby is the shortest one the config will run, and a gate should
        not spend a minute of wall clock proving something a ten-second one
        proves.
        """
        joining = self.args.full_clients - len(self.live())
        if joining > 0:
            self.connect(joining)
        if not self.wait_until(
            lambda: all(client.joined for client in self.live()),
            60.0,
            "a full lobby",
        ):
            return

        # Everybody who has not picked yet takes a different free mob.
        #
        # Assigned from one reading of the manifest and then walked and clicked
        # all at once, rather than a client at a time. Both halves of that
        # matter. Asking each client in turn what is free races the one before
        # it, whose claim may not have landed yet, and two clients then go for
        # the same mob. And doing it in series costs a second and a half of
        # walking each, which is most of the ten-second countdown a full lobby
        # runs: the first version of this had the eighth player still choosing
        # when the match committed, and was refused by the game, correctly.
        waiting = [client for client in self.live() if client.kit is None]
        podiums = self.ask_podiums(self.live()[0])
        free = [entry for entry in podiums if entry["held_by"] is None]
        if len(free) < len(waiting):
            self.fail(
                "a lobby of %d has only %d free mobs left"
                % (len(self.live()), len(free))
            )
            return

        assigned = list(zip(waiting, free))
        wanted = {client.name: entry["kit"] for client, entry in assigned}
        for client, entry in assigned:
            x, y, z = self.click_at(entry)
            client.walk((x + 1.5, float(y) - 1.0, z + 1.5))
        self.wait_until(
            lambda: all(client.arrived() for client, _ in assigned), 20.0
        )
        for client, entry in assigned:
            client.look_at(self.click_at(entry))
        self.settle(0.2)

        for client, entry in assigned:
            client.action_bar.clear()
            entity_id = self.mob_of(client, entry)
            if entity_id is None:
                self.fail(
                    "%s was never sent the %s mob, so there was nothing to click"
                    % (client.name, entry["kit"])
                )
                return
            client.right_click_mob(entity_id, "(the %s)" % entry["kit"])

        if not self.wait_until(
            lambda: all(client.kit is not None for client in self.live()),
            20.0,
            "every client to be playing something",
        ):
            missing = [
                "%s (wanted the %s)" % (c.name, wanted.get(c.name, "?"))
                for c in self.live()
                if c.kit is None
            ]
            self.log("still without a mob: %s" % ", ".join(missing))
            return

        before = {client.name: client.kit for client in self.live()}
        distinct = set(before.values())
        if len(distinct) != len(before):
            self.fail("a full lobby ended up sharing mobs: %r" % before)
            return
        self.prove(
            "a full lobby holds one mob each",
            "; ".join("%s=%s" % pair for pair in sorted(before.items())),
        )

        if not self.wait_until(
            lambda: self.phase == "playing",
            180.0,
            "the countdown to finish and the match to start",
        ):
            return

        after = {client.name: client.kit for client in self.live()}
        changed = {
            name: (before[name], after.get(name))
            for name in before
            if before.get(name) != after.get(name)
        }
        if changed:
            self.fail("the countdown changed somebody's mob: %r" % changed)
            return
        self.prove(
            "a selection made in the hub survives the countdown",
            "%d clients started the match on the mob they clicked for in the "
            "lobby" % len(after),
        )

        # And once it has committed, the podiums stop answering. This is the
        # rule that makes the claim last the whole match without anything
        # having to say so.
        client = self.live()[0]
        podiums = self.ask_podiums(client)
        other = next(
            (entry for entry in podiums if entry["kit"] != client.kit), None
        )
        if other is None:
            self.fail("the roster has one kit in it, so this proves nothing")
            return
        held = client.kit
        client.action_bar.clear()
        # From the arena, several hundred blocks from the hub. hyperion does
        # not check that a clicked block is within reach, and this check is
        # about the phase rule rather than about reach, so the click is sent
        # from wherever the scatter put the player.
        client.right_click(self.click_at(other), "(%s, mid-match)" % other["kit"])
        self.settle(1.0)
        if client.kit != held:
            self.fail(
                "%s clicked a podium mid-match and changed from %s to %s"
                % (client.name, held, client.kit)
            )
            return
        told = [line for line in client.action_bar if "cannot change kit" in line]
        if not told:
            self.fail(
                "a mid-match click was ignored rather than refused: %r"
                % client.action_bar
            )
            return
        self.prove(
            "a click after the match commits is refused",
            "%s clicked the %s while playing the %s and was told %r"
            % (client.name, other["kit"], held, told[0]),
        )

    # --- the verdict ----------------------------------------------------

    def report(self):
        for client in self.live():
            client.leave()
        print("")
        print("=" * 78)
        for claim, evidence in self.proved:
            print("PASS  %-52s %s" % (claim, evidence))
        for why in self.failures:
            print("FAIL  %s" % why)
        print("=" * 78)
        if self.failures:
            print("RESULT: %d checks failed" % len(self.failures))
            return 1
        # A count, because a check that returned early without failing is a
        # check that proved nothing and said nothing, and the run would
        # otherwise read as a pass.
        if len(self.proved) < CLAIMS:
            print(
                "RESULT: only %d of %d checks reported, so something stopped "
                "early" % (len(self.proved), CLAIMS)
            )
            return 1
        print("RESULT: the kit selector works, on the wire")
        return 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument(
        "--full-clients",
        type=int,
        default=8,
        help="how many clients fill the lobby for the last check, which wants "
        "a committed match and the shortest countdown the server will run. "
        "Must be at or above the server's `full_players`. Declared by the gate "
        "rather than read from a default here, because it is the server's "
        "number: it was `FULL_PLAYERS = 8` copied out of `LobbyConfig::default` "
        "until #1019 moved that default to 4 and the copy stayed at 8",
    )
    args = parser.parse_args()
    return Run(args).run()


if __name__ == "__main__":
    sys.exit(main())
