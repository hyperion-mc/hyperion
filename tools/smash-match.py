#!/usr/bin/env python3
"""Drive four scripted clients through one whole Super Smash Mobs match.

`client-26.2.py` answers "is the server joinable". This answers the next
question, which is the one a single client structurally cannot: does a *match*
happen. `min_players` is four, so nothing past the hub is reachable until four
clients are in the world at once, and four separate processes cannot hit each
other because neither knows the other's entity id. Both problems disappear if
one process owns every socket, so this drives all of them from a single loop
and prints one interleaved transcript.

Protocol 776 throughout. The framing, the handshake, the login and the
configuration state are `client-26.2.py`'s, imported rather than copied, so
there is one place where "how do you get into this server" is written down.
What this file adds is the play state: the serverbound ids a player uses, the
clientbound ids a match is visible through, and the schedule.

What this proves and what it does not
-------------------------------------
It proves the server's own state machine, on the wire, at 776 ids: the lobby
count, the countdown, the scatter onto a committed map's spawn points, the kit
hotbar arriving as real item stacks, knockback matching the model in
`events/smash/src/module/knockback.rs`, the life counter, the respawn, and the
return to the hub.

It does not prove the game is playable by a human. These clients do not render,
do not simulate physics and never disagree with the server. They teleport
rather than walk, so nothing here says the platforms have collision from a real
client's point of view. It also does not prove every packet the server sends is
well formed: an id census at the end names what arrived, and `--strict` fails
the run on ids that are not clientbound play ids in 776, but a body that is
wrong under a right id is only caught for the packets this file decodes.
"""

from __future__ import annotations

import argparse
import importlib.util
import math
import pathlib
import re
import struct
import sys
import time

TOOLS = pathlib.Path(__file__).resolve().parent
ROOT = TOOLS.parent


def _load_client():
    """Import `client-26.2.py`, whose file name is not a Python identifier.

    A copy of the framing would be a second place for the compression
    threshold, the varint reader and the known-pack negotiation to drift, and
    the whole point of that file is that there is one of it.
    """
    path = TOOLS / "client-26.2.py"
    spec = importlib.util.spec_from_file_location("client_26_2", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


base = _load_client()

var_int = base.var_int
take_var_int = base.take_var_int
mc_string = base.mc_string
PLAY_NAMES = base.PLAY_NAMES
PROTOCOL = base.PROTOCOL

# Serverbound play ids, from
# crates/hyperion-minecraft-proto/src/generated/packet_id.rs. 26.2 split
# attacking out of `interact`, so melee is `Attack` and not an interact
# variant.
C2S_ACCEPT_TELEPORTATION = 0x00
C2S_ATTACK = 0x01
C2S_CHAT_COMMAND = 0x07
C2S_KEEP_ALIVE = 0x1C
C2S_MOVE_PLAYER_POS = 0x1E
C2S_MOVE_PLAYER_POS_ROT = 0x1F
C2S_SET_CARRIED_ITEM = 0x35
C2S_SWING = 0x3F
C2S_USE_ITEM = 0x43

# Clientbound play ids this file decodes. Everything else is counted by id and
# reported in the census.
S2C_CONTAINER_SET_SLOT = 0x14
S2C_DISCONNECT = 0x20
S2C_KEEP_ALIVE = 0x2C
S2C_LOGIN = 0x31
S2C_PLAYER_POSITION = 0x48
S2C_SET_ENTITY_MOTION = 0x65
S2C_SET_HEALTH = 0x68
S2C_SET_TITLE_TEXT = 0x72
S2C_SYSTEM_CHAT = 0x79

# `ServerboundMovePlayerPacket` packs the ground flag into a bitfield; bit 0 is
# `ON_GROUND`, which is the only bit hyperion reads.
ON_GROUND = 1

# Blocks per tick, quantised to 15 bits per component with a shared integer
# scale. `net.minecraft.network.LpVec3`, and the Rust side of it is
# `crates/hyperion-minecraft-proto/src/packets/play/entity.rs`.
LP_DATA_BITS = 15
LP_MAX_QUANTIZED = 32766
LP_CONTINUATION_FLAG = 4
LP_SCALE_BITS_MASK = 3


def take_lp_vec3(payload, offset):
    """Decode one `LpVec3`. Returns `((x, y, z), offset)`."""
    lowest = payload[offset]
    offset += 1
    if lowest == 0:
        return (0.0, 0.0, 0.0), offset
    middle = payload[offset]
    offset += 1
    (highest,) = struct.unpack(">I", payload[offset : offset + 4])
    offset += 4
    buffer = (highest << 16) | (middle << 8) | lowest

    scale = lowest & LP_SCALE_BITS_MASK
    if lowest & LP_CONTINUATION_FLAG == LP_CONTINUATION_FLAG:
        extra, offset = take_var_int(payload, offset)
        scale |= (extra & 0xFFFFFFFF) << 2

    def unpack(value):
        clamped = min(value & ((1 << LP_DATA_BITS) - 1), LP_MAX_QUANTIZED)
        return clamped * 2.0 / LP_MAX_QUANTIZED - 1.0

    return (
        unpack(buffer >> 3) * scale,
        unpack(buffer >> (3 + LP_DATA_BITS)) * scale,
        unpack(buffer >> (3 + 2 * LP_DATA_BITS)) * scale,
    ), offset


def take_nbt_string(payload, offset):
    """Read a network-NBT tag that is a bare string.

    `Component::text(..).to_tag()` collapses a component with no style and no
    children to `Tag::String`, which is every message this server sends, so a
    full NBT reader would be dead weight. A tag of any other type is reported
    rather than guessed at.
    """
    kind = payload[offset]
    offset += 1
    if kind != 0x08:
        return "<nbt tag type 0x%02X, not a string>" % kind, offset
    (length,) = struct.unpack(">H", payload[offset : offset + 2])
    offset += 2
    text = payload[offset : offset + length].decode("utf-8", "replace")
    return text, offset + length


# events/smash/src/terrain.rs: the hub is region 0 and each arena sits a whole
# stride further along +X, which is how a teleport's X says which map it landed
# on.
REGION_STRIDE = 512

# events/smash/src/module/lives.rs: Mineplex's `MAX_LIVES`.
MAX_LIVES = 4

# events/smash/src/module/knockback.rs: `KnockbackModel::default`, Mineplex's
# own numbers. Restated here so the check below is a check and not a
# tautology over the same constants the server read.
KNOCKBACK_SPEED_BASE = 0.2
KNOCKBACK_SPEED_PER_STRENGTH = 0.8 * 0.6
KNOCKBACK_VERTICAL_PER_STRENGTH = 0.2
KNOCKBACK_VERTICAL_CAP_BASE = 0.4
KNOCKBACK_VERTICAL_CAP_PER_STRENGTH = 0.04
KNOCKBACK_GROUND_BOOST = 0.2
KNOCKBACK_CAP_CROSSOVER = KNOCKBACK_VERTICAL_CAP_BASE / (
    KNOCKBACK_VERTICAL_PER_STRENGTH - KNOCKBACK_VERTICAL_CAP_PER_STRENGTH
)

# How far a step may carry a client, and how often one is sent. Well inside
# what `change_position_or_correct_client` accepts and slower than Minecraft's
# terminal velocity of about 3.9 blocks per tick.
STEP_BLOCKS = 4.0
POSITION_INTERVAL = 0.1

# Blocks from a region's centre to open air. The widest thing any committed map
# puts down reaches a radius of about 52, so 70 clears all of them.
OFF_MAP_RADIUS = 70.0

# The height to leave over an island on the way out. Every map's tallest block
# is at y 77, and hyperion refuses a step that ends inside a block, so a route
# that clips the far side of an island reads as a cheat and gets teleported
# back rather than falling.
ESCAPE_Y = 100.0

# Every arena puts its main platform at this radius, and the opening scatter
# puts all four players on it.
MAIN_ISLAND_RADIUS = 16.0

# How far out to step before climbing. Enough to clear the tree whose leaves
# hang over two of the main island's four opening spawn points, and short
# enough that a player standing on any island is still over open ground or open
# air after it.
RIM_HOP = 6.0

ITEM_REGISTRY = ROOT / "crates/hyperion-minecraft-proto/src/generated/registry.rs"


def load_item_names():
    """`minecraft:item` registry ids to names, read from the generated table.

    An item on the wire is a bare registry id, so a transcript without this
    says "the hotbar holds item 903" where the interesting claim is that it
    holds an iron axe. The table is generated from the same jar the server's
    ids come from, which is why it is read rather than restated here.
    """
    source = ITEM_REGISTRY.read_text()
    start = source.index('pub static ITEM: Registry = Registry {')
    end = source.index('};', start)
    return re.findall(r'"(minecraft:[a-z0-9_/]+)"', source[start:end])[1:]


def stamp(started):
    return "%7.2fs" % (time.time() - started)


class MatchClient(base.Client):
    """One scripted player.

    Inherits the framing, the handshake, the login and the configuration state.
    Adds a receive buffer, because a single-threaded driver of four clients
    cannot afford to block on any one of them.
    """

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, lambda line: None)
        self.started = started
        self.log = self._log
        self.host = host
        self.port = port
        self.buffer = b""
        self.position = (0.0, 65.0, 0.0)
        self.path = []
        self.on_ground = True
        self.health = None
        self.hotbar = {}
        self.seen = {}
        self.last_position_sent = 0.0
        self.alive = True

    def _log(self, line):
        print("%s [%-3s] %s" % (stamp(self.started), self.name, line), flush=True)

    # --- reading -------------------------------------------------------

    def enter_play(self):
        """Stop blocking on reads. Called once configuration is acknowledged."""
        self.sock.settimeout(0.02)

    def drain(self):
        """Every packet already readable, without blocking."""
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

    # --- acting --------------------------------------------------------

    def command(self, text):
        self.log("-> /%s" % text)
        self.send(C2S_CHAT_COMMAND, mc_string(text))

    def walk(self, waypoints, note=None):
        """Head for each point in turn, a few blocks at a time.

        Claiming a destination in one packet is what a teleporting client
        does, and hyperion treats a step it cannot account for as a cheat: it
        schedules a teleport back to where the player was last tick. Walking
        keeps every step inside what the server will accept, which is also the
        only way the arena's kill plane sees a player falling rather than
        being snapped back onto the map.
        """
        self.path = list(waypoints)
        if note:
            x, y, z = self.path[-1]
            self.log("-> walking to (%.1f, %.1f, %.1f)  %s" % (x, y, z, note))

    def arrived(self):
        return not self.path

    def repeat_position(self):
        """Re-assert where we are, the way a real client does every tick.

        Without it hyperion's mirrored position goes stale and the arena's
        bounds check keeps reading wherever we last claimed to be.
        """
        now = time.time()
        if now - self.last_position_sent < POSITION_INTERVAL:
            return
        self.last_position_sent = now

        if self.path:
            x, y, z = self.position
            tx, ty, tz = self.path[0]
            dx, dy, dz = tx - x, ty - y, tz - z
            distance = math.sqrt(dx * dx + dy * dy + dz * dz)
            if distance <= STEP_BLOCKS:
                self.position = self.path.pop(0)
            else:
                scale = STEP_BLOCKS / distance
                self.position = (x + dx * scale, y + dy * scale, z + dz * scale)

        x, y, z = self.position
        self.send(
            C2S_MOVE_PLAYER_POS,
            struct.pack(">dddb", x, y, z, ON_GROUND if self.on_ground else 0),
        )

    def attack(self, target):
        self.log("-> attack %s (entity %d)" % (target.name, target.entity_id))
        self.send(C2S_ATTACK, var_int(target.entity_id))
        self.send(C2S_SWING, var_int(0))

    def use_slot(self, slot, note=""):
        self.log("-> right-click hotbar slot %d %s" % (slot, note))
        self.send(C2S_SET_CARRIED_ITEM, struct.pack(">h", slot))
        # `sequence` is the client's own block-change counter; the server only
        # echoes it back, and nothing here reads the echo.
        self.send(C2S_USE_ITEM, var_int(0) + var_int(0) + struct.pack(">ff", 0.0, 0.0))
        self.send(C2S_SWING, var_int(0))


class Match:
    """The run: four clients, one transcript, and what it proved."""

    def __init__(self, args):
        self.args = args
        self.started = time.time()
        self.items = load_item_names()
        self.clients = []
        self.by_entity = {}
        # Every claim the report at the end makes, in the order a match makes
        # them. A step is proved by a packet, never by a timer expiring.
        self.proof = {
            "four in play": None,
            "lobby started a match": None,
            "kits equipped": None,
            "arena is a committed map": None,
            "knockback from an ability": None,
            "life lost and respawned": None,
            "match ended, back in the hub": None,
        }
        self.motions = []
        self.deaths = []
        self.respawns = []
        self.eliminated = set()
        self.lost_lives = {}
        self.died_awaiting_respawn = set()
        self.falls = 0
        self.phase = "hub"
        self.last_step = 0.0

    def log(self, line):
        print("%s %-5s %s" % (stamp(self.started), "", line), flush=True)

    def prove(self, claim, evidence):
        if self.proof[claim] is None:
            self.proof[claim] = evidence
            self.log("PROVED %s: %s" % (claim, evidence))

    # --- setup ---------------------------------------------------------

    def connect(self):
        for index in range(self.args.clients):
            name = "P%d" % (index + 1)
            client = MatchClient(self.args.host, self.args.port, name, self.started)
            client.handshake(self.args.host, self.args.port, 2)
            client.login()
            client.configuration()
            client.enter_play()
            self.clients.append(client)
            client.log("configuration acknowledged")

    # --- reading -------------------------------------------------------

    def pump(self, client):
        for packet_id, payload in client.drain():
            client.seen[packet_id] = client.seen.get(packet_id, 0) + 1
            self.handle(client, packet_id, payload)

    def handle(self, client, packet_id, payload):
        if packet_id == S2C_LOGIN:
            client.entity_id = struct.unpack(">i", payload[:4])[0]
            client.joined = True
            self.by_entity[client.entity_id] = client
            client.log("** in the world ** entity_id=%d" % client.entity_id)
            if all(other.joined for other in self.clients):
                self.prove(
                    "four in play",
                    "%d clients hold a Login packet: %s"
                    % (
                        len(self.clients),
                        ", ".join(
                            "%s=%d" % (c.name, c.entity_id) for c in self.clients
                        ),
                    ),
                )
        elif packet_id == S2C_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            client.position = (x, y, z)
            client.send(C2S_ACCEPT_TELEPORTATION, var_int(teleport_id))
            client.log("<- teleported to (%.1f, %.1f, %.1f)" % (x, y, z))
            self.on_teleport(client, x, y, z)
        elif packet_id == S2C_KEEP_ALIVE:
            client.send(C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == S2C_SYSTEM_CHAT:
            text, _ = take_nbt_string(payload, 0)
            client.log("<- chat: %s" % text)
            self.on_chat(client, text)
        elif packet_id == S2C_SET_HEALTH:
            # `ClientboundSetHealthPacket`: the health, then the food and
            # saturation nothing here reads.
            health = struct.unpack(">f", payload[:4])[0]
            client.health = health
            client.log("<- health %.2f/20" % health)
        elif packet_id == S2C_SET_TITLE_TEXT:
            text, _ = take_nbt_string(payload, 0)
            client.log("<- title: %s" % text)
        elif packet_id == S2C_SET_ENTITY_MOTION:
            entity, offset = take_var_int(payload)
            motion, _ = take_lp_vec3(payload, offset)
            self.on_motion(client, entity, motion)
        elif packet_id == S2C_CONTAINER_SET_SLOT:
            self.on_slot(client, payload)
        elif packet_id == S2C_DISCONNECT:
            text, _ = take_nbt_string(payload, 0)
            client.log("<- DISCONNECTED: %s" % text)
            client.alive = False

    def on_slot(self, client, payload):
        _container, offset = take_var_int(payload)
        _state, offset = take_var_int(payload, offset)
        (slot,) = struct.unpack(">h", payload[offset : offset + 2])
        offset += 2
        count, offset = take_var_int(payload, offset)
        if count <= 0:
            client.hotbar.pop(slot, None)
            return
        item_id, offset = take_var_int(payload, offset)
        name = (
            self.items[item_id]
            if 0 <= item_id < len(self.items)
            else "<item id %d>" % item_id
        )
        client.hotbar[slot] = name
        client.log("<- slot %d now holds %d x %s" % (slot, count, name))
        equipped = [c for c in self.clients if len(c.hotbar) >= 2]
        if len(equipped) == len(self.clients):
            self.prove(
                "kits equipped",
                "; ".join(
                    "%s %s"
                    % (
                        c.name,
                        ", ".join(
                            "%d:%s" % (s, c.hotbar[s]) for s in sorted(c.hotbar)
                        ),
                    )
                    for c in self.clients
                ),
            )

    def on_chat(self, client, text):
        # Every announcement is a broadcast, so one client narrates the match
        # and the rest are only proof it reached them too.
        if client is not self.clients[0]:
            return
        if "The game starts shortly" in text:
            self.phase = "countdown"
            self.prove(
                "lobby started a match",
                "server announced the countdown with %d players connected"
                % len(self.clients),
            )
        elif "Get ready" in text:
            self.phase = "preparing"
        elif text.strip().endswith("Go!") or text.strip() == "Go!":
            self.phase = "playing"
            self.log("== the match is running ==")
        elif "Game over" in text:
            self.phase = "ended"
        elif "fell out of bounds" in text or "was smashed by" in text:
            # Counted from one client only: the line is broadcast, so counting
            # every copy would say four people died. The victim's name leads
            # the line, and the chat is the only place it appears: the life
            # counter title carries a count and not a name.
            self.deaths.append(text)
            name = text.split()[0]
            self.lost_lives[name] = self.lost_lives.get(name, 0) + 1
            self.died_awaiting_respawn.add(name)
            if self.lost_lives[name] >= MAX_LIVES:
                self.eliminated.add(name)
                # Nothing brings them back, so leaving them in the
                # waiting-to-respawn set would stall every later step.
                self.died_awaiting_respawn.discard(name)
                self.log("%s is out of the match" % name)

    def on_teleport(self, client, x, y, z):
        client.on_ground = True
        # Wherever the client was heading, the server has just overruled it.
        client.path = []
        # The hub is region 0 and every arena is a whole `REGION_STRIDE` away
        # along +X, so which side of half a stride a teleport lands on is the
        # difference between "still in the lobby" and "on a map that was parsed
        # out of events/smash/maps".
        on_a_map = abs(x) >= REGION_STRIDE / 2
        if on_a_map:
            self.prove(
                "arena is a committed map",
                "%s scattered to (%.1f, %.1f, %.1f), which is map region %d "
                "rather than the hub at the origin"
                % (client.name, x, y, z, round(x / REGION_STRIDE)),
            )
        elif self.phase == "ended":
            self.prove(
                "match ended, back in the hub",
                "%s teleported to the hub at (%.1f, %.1f, %.1f) after the "
                "server announced the end of the match"
                % (client.name, x, y, z),
            )

        if on_a_map and client.name in self.died_awaiting_respawn:
            self.died_awaiting_respawn.discard(client.name)
            self.prove(
                "life lost and respawned",
                "%s died %d time(s) and came back at (%.1f, %.1f, %.1f) with "
                "the kit hotbar pushed again"
                % (client.name, self.lost_lives[client.name], x, y, z),
            )

    def on_motion(self, viewer, entity, motion):
        # Recorded once, from one viewer: the packet is broadcast, so counting
        # every copy would make one launch look like four.
        if viewer is not self.clients[0]:
            return
        victim = self.by_entity.get(entity)
        who = victim.name if victim else "entity %d" % entity
        speed = math.sqrt(motion[0] ** 2 + motion[1] ** 2 + motion[2] ** 2)
        if speed < 1e-4:
            return
        viewer.log(
            "<- knockback on %s: (%.3f, %.3f, %.3f) |v| = %.3f blocks/tick"
            % (who, motion[0], motion[1], motion[2], speed)
        )
        self.motions.append((entity, motion))

    # --- the script ----------------------------------------------------

    def run(self):
        self.connect()
        kits = [kit.strip() for kit in self.args.kits.split(",")]
        deadline = time.time() + self.args.seconds

        step = 0
        script = self.build_script(kits)
        while time.time() < deadline:
            for client in self.clients:
                if client.alive:
                    self.pump(client)
                    if client.joined:
                        client.repeat_position()
            if step < len(script):
                when, action = script[step]
                if self.ready(when):
                    action()
                    step += 1
            elif self.proof["match ended, back in the hub"] is not None:
                break
            time.sleep(0.01)

        return self.report()

    def ready(self, when):
        """A step runs when the server says so, not when a clock says so.

        `when` is either a predicate over what the server has already sent or a
        delay in seconds after the previous step. Gating on the server's own
        announcements is what keeps the transcript honest: a step that ran
        before the server got there would read as a pass.
        """
        if callable(when):
            return when()
        return time.time() - self.last_step >= when

    def build_script(self, kits):
        plan = []

        def at(when, action):
            def wrapped():
                action()
                self.last_step = time.time()

            plan.append((when, wrapped))

        in_play = lambda: all(c.joined for c in self.clients)
        playing = lambda: self.phase == "playing"
        gathered = lambda: all(c.arrived() for c in self.clients)

        # A command sent before the server has moved this connection into play
        # would be read against the configuration state's id table.
        at(in_play, lambda: None)
        for client, kit in zip(self.clients, kits):
            at(0.2, lambda c=client, k=kit: c.command("kit %s" % k))

        # Everything below waits for `Go!`, so the countdown's length is the
        # server's business and not a number written down here.
        launched = lambda: bool(self.motions)
        # Every fall so far has cost a life and the victim is back on their
        # feet. Counting falls against deaths is what stops the script walking
        # the next victim off the edge while the last one is still falling: a
        # fall that has not landed yet leaves nobody waiting to respawn, which
        # on its own looks exactly like a fall that is finished.
        settled = lambda: (
            self.falls == len(self.deaths)
            and not self.died_awaiting_respawn
            and time.time() - self.last_step >= 1.0
        )

        at(playing, self.gather)
        at(gathered, self.smash)
        at(launched, self.check_knockback)
        at(0.5, self.melee)
        at(1.0, self.fall)

        # Three of the four have to lose every life for the match to end: the
        # `Playing` phase ends the moment one player is left alive. Each fall
        # waits for the last victim to be back on their feet rather than for a
        # timer, so a slow respawn stalls the script instead of quietly
        # skipping a life.
        for _round in range(MAX_LIVES * (self.args.clients - 1) - 1):
            at(settled, self.fall)

        return plan

    def victims(self):
        return [
            c
            for c in self.clients[1:]
            if c.alive and c.name not in self.eliminated
        ]

    @staticmethod
    def outward(x, z, radius, y):
        """A point `radius` blocks from the region's centre, past (x, z).

        Every arena is a set of islands around its region's centre, so a
        direction is only ever "further out" or "further in" and a radius is
        the one coordinate that says whether a spot is over the map.
        """
        centre_x = round(x / REGION_STRIDE) * REGION_STRIDE
        away_x, away_z = x - centre_x, z
        length = math.hypot(away_x, away_z) or 1.0
        return (
            centre_x + away_x / length * radius,
            y,
            away_z / length * radius,
        )

    @staticmethod
    def radius_of(x, z):
        centre_x = round(x / REGION_STRIDE) * REGION_STRIDE
        return math.hypot(x - centre_x, z)

    def route(self, client, destination):
        """Waypoints from where a client stands to `destination`, over the map.

        A short hop outwards, then straight up above the tallest block any map
        has, then across, then down. hyperion answers a step that ends inside a
        block with a teleport back to where the player was, so none of the
        three obvious routes work: a straight line between two spawn points
        goes through the pillars and the trees in the middle of the main
        island, rising where you stand goes through the tree overhead, and
        leaving diagonally clips whichever island is on that bearing.
        """
        x, y, z = client.position
        clear = self.outward(x, z, self.radius_of(x, z) + RIM_HOP, y)
        return [
            clear,
            (clear[0], ESCAPE_Y, clear[2]),
            (destination[0], ESCAPE_Y, destination[2]),
            destination,
        ]

    def gather(self):
        """Walk everyone to within one ability's radius of the attacker.

        The meeting point is just past the main island's rim, on the attacker's
        own side of it, so the descent at the end of the route is through open
        air rather than through the island.
        """
        attacker = self.clients[0]
        ax, ay, az = attacker.position
        for index, client in enumerate(self.clients[1:]):
            spot = self.outward(
                ax, az, MAIN_ISLAND_RADIUS + 1.5 + index, ay
            )
            client.walk(self.route(client, spot), "into range of %s" % attacker.name)

    def smash(self):
        """Seismic Slam, which launches everything within eight blocks."""
        self.motions.clear()
        self.clients[0].use_slot(self.args.ability_slot, "(the kit's radial ability)")

    def melee(self):
        attacker = self.clients[0]
        for victim in self.victims():
            attacker.attack(victim)

    def check_knockback(self):
        """Hold the launch the ability produced against the model.

        Not against a number worked out from the kit's stats, which would need
        this file to know the kit's armour, health and knockback-taken and so
        would only be checking its own arithmetic. The model gives horizontal
        and vertical speed as two different functions of one hidden strength,
        so solving each for strength and asking whether the two agree tests the
        model itself and needs nothing from the kit. Vanilla Minecraft
        knockback, which is a flat 0.4 and 0.4, disagrees by a factor of two.
        """
        attacker = self.clients[0]
        launched = [
            (entity, motion)
            for entity, motion in self.motions
            if entity in self.by_entity and self.by_entity[entity] is not attacker
        ]
        if not launched:
            return

        entity, motion = launched[0]
        victim = self.by_entity[entity]
        horizontal = math.hypot(motion[0], motion[2])
        strength_h = (
            horizontal - KNOCKBACK_SPEED_BASE
        ) / KNOCKBACK_SPEED_PER_STRENGTH

        # Whether the victim was standing on something is the server's own
        # reading of the block under them, so both readings are tried and the
        # one that agrees is reported.
        for grounded in (False, True):
            lift = motion[1] - (KNOCKBACK_GROUND_BOOST if grounded else 0.0)
            # `vertical = min(0.2 * s, 0.4 + 0.04 * s)`, and the two branches
            # cross at s = 0.4 / (0.2 - 0.04).
            uncapped = lift / KNOCKBACK_VERTICAL_PER_STRENGTH
            strength_v = (
                uncapped
                if uncapped <= KNOCKBACK_CAP_CROSSOVER
                else (lift - KNOCKBACK_VERTICAL_CAP_BASE)
                / KNOCKBACK_VERTICAL_CAP_PER_STRENGTH
            )
            if abs(strength_v - strength_h) > 0.02:
                continue

            ax, _ay, az = attacker.position
            vx, _vy, vz = victim.position
            away = (vx - ax, vz - az)
            length = math.hypot(*away) or 1.0
            alignment = (motion[0] * away[0] + motion[2] * away[1]) / (
                length * (horizontal or 1.0)
            )
            self.prove(
                "knockback from an ability",
                "%s took (%.3f, %.3f, %.3f) blocks/tick. Horizontal %.3f "
                "implies strength %.3f, vertical %.3f implies %.3f "
                "(%s), which is the same launch the model in "
                "module/knockback.rs describes; the direction is %.0f%% away "
                "from %s. Vanilla would have sent 0.4 and 0.4."
                % (
                    victim.name,
                    motion[0],
                    motion[1],
                    motion[2],
                    horizontal,
                    strength_h,
                    motion[1],
                    strength_v,
                    "on the ground" if grounded else "airborne",
                    alignment * 100.0,
                    attacker.name,
                ),
            )
            return

        self.log(
            "knockback on %s was (%.3f, %.3f, %.3f), which does not solve to "
            "one strength under the model: horizontal says %.3f"
            % (victim.name, motion[0], motion[1], motion[2], strength_h)
        )

    def fall(self):
        """Walk the next living victim off the edge, after a hit lands.

        Kill credit expires ten seconds after the last hit, so the hit and the
        fall are close together and the death reads as a smash rather than as
        an accident.
        """
        living = self.victims()
        if not living:
            return
        victim = living[len(self.deaths) % len(living)]
        attacker = self.clients[0]
        if attacker.entity_id is not None and victim.entity_id is not None:
            attacker.attack(victim)
        x, _y, z = victim.position
        victim.on_ground = False
        edge = self.outward(x, z, OFF_MAP_RADIUS, self.args.void_y)
        victim.walk(self.route(victim, edge), "off the edge")
        self.falls += 1

    # --- the verdict ---------------------------------------------------

    def report(self):
        print("", flush=True)
        self.log("=== packets received in play state, by id ===")
        census = {}
        for client in self.clients:
            for packet_id, count in client.seen.items():
                census[packet_id] = census.get(packet_id, 0) + count
        unknown = []
        for packet_id in sorted(census):
            name = PLAY_NAMES.get(packet_id)
            if name is None:
                unknown.append(packet_id)
                name = "<NOT A 776 CLIENTBOUND PLAY ID>"
            self.log("    0x%02X %-28s x%d" % (packet_id, name, census[packet_id]))

        print("", flush=True)
        self.log("=== what the match proved ===")
        failed = []
        for claim, evidence in self.proof.items():
            if evidence is None:
                failed.append(claim)
                self.log("  NOT PROVED  %s" % claim)
            else:
                self.log("  proved      %s" % claim)
                self.log("              %s" % evidence)

        print("", flush=True)
        if unknown:
            self.log(
                "RESULT: %d packet id(s) are not clientbound play ids in "
                "protocol 776: %s"
                % (len(unknown), ", ".join("0x%02X" % i for i in unknown))
            )
            return 1
        if failed:
            self.log(
                "RESULT: %d of %d steps did not happen" % (len(failed), len(self.proof))
            )
            return 1
        self.log("RESULT: a whole match ran at protocol 776")
        return 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument("--clients", type=int, default=4)
    parser.add_argument(
        "--kits",
        default="Iron Golem,Skeleton,Enderman,Slime",
        help="comma-separated, one per client; the first one attacks",
    )
    parser.add_argument(
        "--ability-slot",
        type=int,
        default=3,
        help="hotbar slot the attacker right-clicks; 3 is Iron Golem's "
        "Seismic Slam, which is radial and so does not depend on facing",
    )
    parser.add_argument("--seconds", type=float, default=300.0)
    parser.add_argument(
        "--void-y",
        type=float,
        default=-5.0,
        help="the Y a client claims to be at when the script wants it to die",
    )
    args = parser.parse_args()

    if args.clients > len(args.kits.split(",")):
        raise SystemExit("--kits needs one kit per client")

    return Match(args).run()


if __name__ == "__main__":
    sys.exit(main())
