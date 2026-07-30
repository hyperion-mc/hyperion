#!/usr/bin/env python3
"""Drive a roster of scripted clients through one whole Super Smash Mobs match.

`client-26.2.py` answers "is the server joinable". This answers the next
question, which is the one a single client structurally cannot: does a *match*
happen. Nothing past the hub is reachable until the lobby has reached its
minimum, which is more clients than one process usually is, and separate
processes cannot hit each other because neither knows the other's entity id.
Both problems disappear if one process owns every socket, so this drives all of
them from a single loop and prints one interleaved transcript.

Protocol 776 throughout. The framing, the handshake, the login and the
configuration state are `client-26.2.py`'s, imported rather than copied, so
there is one place where "how do you get into this server" is written down.
What this file adds is the play state: the serverbound ids a player uses, the
clientbound ids a match is visible through, and the schedule.

It also drives every ability in the game, and it does not know what those are.
The server answers `/abilities` with its own registry -- one JSON object per
ability, built by walking the ability entities themselves -- and this file fires
each one and holds it to the effects that entry declares. Nothing here lists a
kit or an ability, so a kit added tomorrow is a kit this gate tests tomorrow,
and one whose ability does nothing fails here rather than passing quietly.

The sweep runs in the hub, on a roster small enough that the lobby will not
start under it: a committed match has a kill plane, locks abilities and refuses
to change a kit, and the sweep needs all three of those not to be true. How
small that is depends on the server's thresholds, so `--sweep-clients` says it
outright and nothing here computes it. This file does not know the lobby's
numbers and must not guess: a guess that is wrong reads as fifteen kits passing
when the last three never ran. It is checked against the running server
instead -- if the lobby leaves the hub while the sweep is in it, the run fails
there, naming the roster. The rest of the clients join once the sweep is done,
and the match runs after them.

What this proves and what it does not
-------------------------------------
It proves the server's own state machine, on the wire, at 776 ids: the lobby
count, the countdown, the scatter onto a committed map's spawn points, the kit
hotbar arriving as real item stacks, knockback matching the model in
`events/smash/src/module/knockback.rs`, the life counter, the respawn, and the
return to the hub. It proves each declared ability by its effect -- health
lost, a velocity packet, a teleport, a heal, a melee swing that got stronger --
and never by the fact that a right-click was sent.

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
import json
import math
import pathlib
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
C2S_PLAYER_ACTION = 0x29
C2S_SET_CARRIED_ITEM = 0x35
C2S_SWING = 0x3F
C2S_USE_ITEM = 0x43

# `ServerboundPlayerActionPacket$Action#RELEASE_USE_ITEM`. Letting go of a held
# item is a player action and not its own packet, which is why a client that
# only ever sends `use_item` can start a charge and never finish one.
ACTION_RELEASE_USE_ITEM = 5

# Clientbound play ids this file decodes. Everything else is counted by id and
# reported in the census.
S2C_ADD_ENTITY = 0x01
S2C_CONTAINER_SET_SLOT = 0x14
S2C_DISCONNECT = 0x20
S2C_KEEP_ALIVE = 0x2C
S2C_LEVEL_PARTICLES = 0x2F
S2C_LOGIN = 0x31
S2C_PLAYER_POSITION = 0x48
S2C_SET_ACTION_BAR_TEXT = 0x57
S2C_SET_ENTITY_MOTION = 0x65
S2C_SET_HEALTH = 0x68
S2C_SET_TITLE_TEXT = 0x72
S2C_SOUND = 0x75
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


def take_particles(payload):
    """`ClientboundLevelParticlesPacket`, as far as this file cares about it.

    Fixed-width up to the particle, then a var int type id and whatever that
    type's own codec writes. The body is not read: which particle arrived and
    how many of it are what a kit declares, and the body shapes are already
    held against Mojang's own encoder by `play_particles.rs`.

    Returns `(particle_id, position, count)`, position in blocks.
    """
    override_limiter = payload[0]
    always_show = payload[1]
    x, y, z, x_dist, y_dist, z_dist, max_speed, count = struct.unpack(
        ">dddffffi", payload[2:46]
    )
    particle_id, _ = take_var_int(payload, 46)
    del override_limiter, always_show, x_dist, y_dist, z_dist, max_speed
    return particle_id, (x, y, z), count


def take_sound(payload):
    """`ClientboundSoundPacket`, as far as this file cares about it.

    The sound event is a `Holder`: a var int that is either an index into the
    client's `minecraft:sound_event` registry, biased by one, or zero followed
    by the event inline. hyperion sends the registries by name and has no id
    table to look one up in, so it always writes the inline form -- which is
    also the only form this reads, because an id would tell a test nothing it
    could compare against a kit declaration.

    Returns `(id, source, position, volume, pitch)` with the position back in
    blocks; the wire carries it in eighths.
    """
    holder, offset = take_var_int(payload)
    if holder != 0:
        return None
    length, offset = take_var_int(payload, offset)
    sound_id = payload[offset : offset + length].decode("utf-8", "replace")
    offset += length
    # `fixedRange` is optional and hyperion leaves it out, but a present one
    # would shift everything after it.
    has_range = payload[offset]
    offset += 1
    if has_range:
        offset += 4
    source, offset = take_var_int(payload, offset)
    x, y, z, volume, pitch = struct.unpack(">iiiff", payload[offset : offset + 20])
    return sound_id, source, (x / 8.0, y / 8.0, z / 8.0), volume, pitch


# NBT payload sizes for the fixed-width tag types.
NBT_FIXED = {1: 1, 2: 2, 3: 4, 4: 8, 5: 4, 6: 8}


def _nbt_read(buf, offset, kind):
    """One NBT payload, as the little of it a chat component needs.

    Strings, lists and compounds come back as themselves; everything else comes
    back as `None` after being stepped over, because no readable part of a
    component is stored in a number.
    """
    if kind in NBT_FIXED:
        return None, offset + NBT_FIXED[kind]
    if kind == 8:
        (length,) = struct.unpack_from(">H", buf, offset)
        offset += 2
        return buf[offset : offset + length].decode("utf-8", "replace"), offset + length
    if kind == 9:
        element = buf[offset]
        (count,) = struct.unpack_from(">i", buf, offset + 1)
        offset += 5
        values = []
        for _ in range(count):
            value, offset = _nbt_read(buf, offset, element)
            values.append(value)
        return values, offset
    if kind == 10:
        fields = {}
        while True:
            entry = buf[offset]
            offset += 1
            if entry == 0:
                return fields, offset
            (length,) = struct.unpack_from(">H", buf, offset)
            offset += 2
            name = buf[offset : offset + length].decode("utf-8", "replace")
            offset += length
            fields[name], offset = _nbt_read(buf, offset, entry)
    if kind == 7:
        (count,) = struct.unpack_from(">i", buf, offset)
        return None, offset + 4 + count
    if kind == 11:
        (count,) = struct.unpack_from(">i", buf, offset)
        return None, offset + 4 + 4 * count
    if kind == 12:
        (count,) = struct.unpack_from(">i", buf, offset)
        return None, offset + 4 + 8 * count
    raise ValueError("unknown NBT tag type %d at offset %d" % (kind, offset))


def _nbt_plain(value):
    """The words in a decoded component, with the styling discarded.

    `Component::text` renders as `{text: "..."}` and picks up `extra` for
    children and `color` for a style, and this walks the first two and ignores
    the third. Matching the server's own `Component::plain`, which is what the
    Rust tests assert against.
    """
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "".join(_nbt_plain(item) for item in value)
    if isinstance(value, dict):
        out = _nbt_plain(value.get("text", ""))
        return out + _nbt_plain(value.get("extra", []))
    return ""


def take_nbt_string(payload, offset):
    """Read one network-NBT chat component as plain text.

    A component with no style and no children collapses to `Tag::String`, which
    is what every message here was until the component API landed. Anything
    coloured is a compound instead, so both shapes have to be read: a reader
    that handled only the bare string reported an action bar full of
    `<nbt tag type 0x0A>` and a gate that could no longer see what the server
    said.
    """
    kind = payload[offset]
    offset += 1
    if kind == 0x08:
        (length,) = struct.unpack(">H", payload[offset : offset + 2])
        offset += 2
        return payload[offset : offset + length].decode("utf-8", "replace"), offset + length
    if kind == 0x0A:
        # Network NBT: the root compound carries no name of its own.
        fields, offset = _nbt_read(payload, offset, 0x0A)
        return _nbt_plain(fields), offset
    return "<nbt tag type 0x%02X, neither a string nor a compound>" % kind, offset


# events/smash/src/terrain.rs: the hub is region 0 and each arena sits a whole
# stride further along +X, which is how a teleport's X says which map it landed
# on.
REGION_STRIDE = 512

# events/smash/src/module/lives.rs: Mineplex's `MAX_LIVES`.
MAX_LIVES = 4

# events/smash/src/module/sound.rs: `IMPACT`, the one sound every hit in the
# game makes. Named here rather than read off the manifest because it is not an
# ability's sound: it belongs to the hit, and its pitch and volume are what say
# how hard that hit was.
IMPACT_SOUND = "minecraft:entity.player.attack.strong"

# How far the sound may be from where the client last claimed the victim was, in
# blocks. Not zero: the server plays it at the position in its own mirror, which
# is a tick or two behind whatever the client last sent, and the wire quantises
# to an eighth of a block on top.
IMPACT_TOLERANCE = 3.0

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

# events/smash/src/command.rs. The markers `/abilities` puts on each line, so a
# reader can tell the registry apart from the rest of the chat channel.
MANIFEST_PREFIX = "smash-ability "
MANIFEST_END_PREFIX = "smash-abilities-end "

# events/smash/src/module/ability.rs: the refusal an ability on cooldown sends to
# the action bar. Restated here so the check below is a check.
COOLDOWN_REFUSAL = "That ability is recharging."

# Where the sweep stands everyone, in events/smash/maps/hub.map's coordinates.
#
# A clear line at x = 6: outside the raised centre (a disc of radius 4 at y 65,
# which a player standing at the origin is inside rather than on), inside the
# glass rim at radius 19, and clear of all twelve pillars, none of which is at
# that x. The attacker looks along +Z, which is yaw 0.
SWEEP_X = 6.0
SWEEP_Y = 65.0
SWEEP_Z = -11.0
SWEEP_YAW = 0.0

# How far past the hub's east wall (x=21) the boundary probe walks a client.
# Far enough to be unambiguously outside; the server should shove it back.
BEYOND_WALL_X = 25.0

# How far in front of the attacker the two victims stand.
#
# Neither is a whole number, and neither is a distance an ability centres its
# blast on. Knockback is horizontal and points away from the blast, so a victim
# standing exactly on a splash centre has no direction to be launched in and does
# not move: a round number would make several abilities look like they deal no
# knockback when it is the arrangement that has no answer.
NEAR_VICTIM = 3.5
FAR_VICTIM = 8.5

# How far the whole arrangement shifts along the aim axis between attempts.
#
# The three of them move together, so every distance and bearing an ability cares
# about is identical and only the absolute position differs. Wither Image needs
# exactly that: it drops a decoy on one press and swaps you to it on the next,
# and a swap back to the spot you never left is not a teleport anybody can see.
ATTEMPT_STRIDE = 4.0

# `PlayerInventory::HOTBAR_START_SLOT`: where the nine hotbar slots begin in the
# inventory numbering `ClientboundContainerSetSlot` uses.
HOTBAR_START_SLOT = 36

# The height the sweep travels at between marks.
#
# Above everything the hub puts down -- the pillars stop at 68 and their lanterns
# at 69 -- so a route that goes up, across and down never ends a step inside a
# block, which is the one move hyperion refuses outright. Going in a straight
# line at head height does not work: half the hub's spawn ring is on the far side
# of the raised centre.
#
# The route is walked at `STEP_BLOCKS` a step and not claimed in one packet:
# hyperion teleports a player back when a tick's movement exceeds about ten
# blocks against one movement packet, which is `sync_entity_state`'s speed check
# rather than the collision one, so it reports nothing at all and simply undoes
# the move.
HUB_CLEAR_Y = 72.0

# How many presses an ability gets before it has to have done what it declared.
SWEEP_ATTEMPTS = 3

# Seconds to wait for an ability's effects after the press. Two hundred server
# ticks, which is far longer than the slowest projectile in the game takes to
# cross the far victim.
SWEEP_WINDOW = 2.0

# Seconds to keep watching, with nothing cast, for an ability that declared it
# leaves something behind.
#
# Runs after `SWEEP_WINDOW`, so an effect has to still be ticking three and a
# half seconds after the press to be seen. Blaze's burn lasts four and Spider's
# poison six, both with room to spare; anything shorter than this would be
# indistinguishable here from an ability that left nothing at all, so a kit
# adding a briefer effect has to shorten the wait rather than trust it.
LINGER_WINDOW = 1.5

# Seconds to wait for a probe hit on the caster to show up on their health bar.
#
# Short on purpose. The only shield in the roster is one second long, and this
# probe is spent from inside that second: a generous timeout here would burn the
# window it is trying to measure and turn a working shield into a failure.
SHIELD_PROBE_WINDOW = 0.4

# The health the melee probe wants its victim standing on before it swings.
#
# `ClientboundSetHealth` is scaled to twenty whatever the kit's real maximum is,
# so this number means the same thing for every kit in the roster.
#
# Not a full bar: the abilities that declare `buffs_melee` are the ones spending
# the window hitting the same player the probe swings at, and demanding twenty
# would mean never getting a reading during a Mooshroom Madness. What it has to
# clear is one swing, so that health cannot hit the floor mid-reading and turn a
# hard hit into a soft one -- and the hardest swing in the roster is under ten
# on this scale, whatever the kit.
MELEE_PROBE_HEADROOM = 12.0

# How long the victim has to go without a health packet before the probe swings.
#
# Shorter than the fastest beat anything in the roster runs -- Cow's herd and
# Wolf's Frenzy are both every two seconds -- or no window ever opens.
MELEE_PROBE_QUIET = 0.3

# How long to keep looking for that window before giving up on a reading.
MELEE_PROBE_SETTLE = 1.5

# How long after the swing to collect health packets before deciding which of
# them was the swing. Every melee answer in the transcript came back inside
# 70 ms; wider than that only lets more of somebody else's damage in.
MELEE_PROBE_WINDOW = 0.25

# How many swings the probe may take looking for one it can attribute.
#
# Bounded from above and not just for time. Every try is a real melee hit, and
# Wolf's Ravage adds a stack to the attacker on every melee hit, so the probe's
# own retries make the next swing harder -- which is the game working, but it
# means a probe that retried indefinitely would saturate Ravage at its ceiling
# and then read a baseline the ultimate cannot beat, because Frenzy's bonus is
# exactly that same ceiling. Three tries leaves at most two stacks standing when
# the last one is measured, one short of the ceiling, so the ultimate is still
# visible above it.
MELEE_PROBE_TRIES = 3


def load_item_names():
    """`minecraft:item` registry ids to names, in network-id order.

    An item on the wire is a bare registry id, so a transcript without this
    says "the hotbar holds item 903" where the interesting claim is that it
    holds an iron axe. Read from `protocol.json`, which is the same file the
    server's own ids are generated from.
    """
    return base.registry_entries("minecraft:item")


def hub_lost(during, clients):
    """Why a gate that needed an unstarted lobby cannot go on, in one line.

    The hub is the only place a gate can change a kit, click a podium or read
    an untouched ring and be sure nothing is racing it. Several harnesses need
    to know they still have it, for the same reason and with the same two
    remedies, so the sentence is written once here rather than worded four
    ways. `hotbar-check.py`, `hud-check.py`, `identity-check.py` and
    `smash-selector.py` all load this module already.

    What this deliberately does not do is tell a gate how many clients are
    safe. That answer is `min_players` and `full_players` together, it belongs
    to the server, and a helper that returned it would be exactly the copy of
    `LobbyConfig::default` that this whole change exists to delete.
    """
    return (
        "the lobby left the hub %s. %d clients is enough for this server to "
        "start a countdown, so the hub is gone and nothing after this would be "
        "proved. Either the gate's roster is too many for this lobby, or the "
        "server needs a higher SMASH_MIN_PLAYERS/SMASH_FULL_PLAYERS."
        % (during, clients)
    )


def stamp(started):
    return "%7.2fs" % (time.time() - started)


def distance(one, other):
    return math.sqrt(sum((a - b) ** 2 for a, b in zip(one, other)))


class MatchClient(base.Client):
    """One scripted player.

    Inherits the framing, the handshake, the login and the configuration state.
    Adds a receive buffer, because a single-threaded driver of four clients
    cannot afford to block on any one of them.
    """

    def __init__(self, host, port, name, started, uuid=None):
        super().__init__(host, port, name, lambda line: None, uuid=uuid)
        self.started = started
        self.log = self._log
        self.host = host
        self.port = port
        self.buffer = b""
        self.position = (0.0, 65.0, 0.0)
        self.path = []
        self.on_ground = True
        # A scripted client that never sends a rotation looks along +Z forever,
        # which is what every ability that fires "where you look" would have
        # read. Carrying yaw and pitch and sending pos-rot rather than pos is
        # what lets the sweep aim.
        self.yaw = 0.0
        self.pitch = 0.0
        self.health = None
        # Every health this client has been sent, in order, and when the last
        # one arrived. The melee probe needs both: it reads the *first* packet
        # after its swing rather than where health ends up, and it waits for a
        # gap in this stream before swinging at all. See `measure_melee`.
        self.health_log = []
        self.health_at = 0.0
        self.spawns = []
        self.kit = None
        self.hotbar = {}
        self.seen = {}
        self.action_bar = []
        self.teleported_to = []
        # Every `ClientboundSoundPacket` this client has been sent, decoded.
        # Collected per client rather than once, because a positioned sound goes
        # out on the chunk channel and a player is not always in their own.
        self.sounds = []
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

        self.send_position()

    def send_position(self):
        x, y, z = self.position
        self.send(
            C2S_MOVE_PLAYER_POS_ROT,
            struct.pack(
                ">dddffb",
                x,
                y,
                z,
                self.yaw,
                self.pitch,
                ON_GROUND if self.on_ground else 0,
            ),
        )

    def aim(self, yaw, pitch=0.0):
        self.yaw = yaw
        self.pitch = pitch

    def look_at(self, other):
        """Face `other`, in Minecraft's yaw, which is clockwise from +Z."""
        dx = other.position[0] - self.position[0]
        dz = other.position[2] - self.position[2]
        self.aim(math.degrees(math.atan2(-dx, dz)))
        self.send_position()

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

    def release_slot(self, slot, note=""):
        """Let go of a held item, which is how a charge ability fires.

        `ServerboundPlayerActionPacket` with `RELEASE_USE_ITEM`: the block
        position and face are what a dig would have filled in and are ignored for
        this action.
        """
        self.log("-> release hotbar slot %d %s" % (slot, note))
        self.send(
            C2S_PLAYER_ACTION,
            var_int(ACTION_RELEASE_USE_ITEM)
            + struct.pack(">qb", 0, 0)
            + var_int(0),
        )


class Match:
    """The run: a roster of clients, one transcript, and what it proved."""

    def __init__(self, args):
        self.args = args
        self.started = time.time()
        self.items = load_item_names()
        self.clients = []
        self.by_entity = {}
        # Every claim the report at the end makes, in the order a match makes
        # them. A step is proved by a packet, never by a timer expiring.
        self.proof = {}
        if args.abilities:
            self.proof["the server published its ability registry"] = None
            self.proof["every declared ability did what it declared"] = None
            self.proof["every declared ability was heard"] = None
            self.proof["every declared ability was seen"] = None
            self.proof["a projectile was drawn"] = None
            self.proof["the hub shoves you back inside"] = None
            self.proof["cooldowns refused a second use"] = None
        self.proof.update({
            "four in play": None,
            "lobby started a match": None,
            "kits equipped": None,
            "arena is a committed map": None,
            "knockback from an ability": None,
            "a hit was heard where it landed": None,
            "life lost and respawned": None,
            "match ended, back in the hub": None,
        })
        # The registry, as the server describes it. Nothing in this file adds to
        # it or filters it.
        self.manifest = []
        self.manifest_expected = None
        # What the sweep found, one line per ability, for the report.
        self.sweep_results = []
        self.sweep_failures = []
        # How many times `measure_melee` gave up on the ability being exercised
        # right now. Counted so a `buffs_melee` failure can say which of the two
        # things went wrong. Reporting "declares buffs_melee and did not do it"
        # when the probe never managed a reading is how ENG-11399 got filed
        # against the harness on a night when the game was also broken.
        self.melee_unmeasured = 0
        # Failures found after the sweep has already reported, which is
        # everything the match phase notices. `sweep_failures` is folded
        # into a proof inside `sweep`, so anything appended to it later
        # is read by nobody.
        self.late_failures = []
        self.cooldown_results = []
        self.cooldown_failures = []
        # Abilities whose declared sound never arrived, and any sound that
        # arrived in a form a client could not resolve.
        self.sound_failures = []
        # Abilities that drew nothing a client could see.
        self.particle_failures = []
        # Velocity packets seen this window, keyed by entity id. Collected from
        # every client rather than one, because a player is not always in their
        # own broadcast channel and the caster's own launch has to be visible.
        self.window_motions = {}
        # Particle packets seen this window, from every client for the same
        # reason motions are: the caster is not always in their own broadcast
        # channel, and an effect drawn at the caster has to be visible.
        self.window_particles = []
        self.motions = []
        self.deaths = []
        self.respawns = []
        self.eliminated = set()
        self.lost_lives = {}
        self.died_awaiting_respawn = set()
        self.falls = 0
        self.phase = "hub"
        # Set by `hub_only` when the lobby starts under a sweep that needed it
        # not to. Fatal, because everything after it measures the wrong game.
        self.hub_lost = False
        self.last_step = 0.0

    def log(self, line):
        print("%s %-5s %s" % (stamp(self.started), "", line), flush=True)

    def prove(self, claim, evidence):
        if self.proof[claim] is None:
            self.proof[claim] = evidence
            self.log("PROVED %s: %s" % (claim, evidence))

    # --- setup ---------------------------------------------------------

    def connect(self, count):
        for _ in range(count):
            name = "P%d" % (len(self.clients) + 1)
            client = MatchClient(self.args.host, self.args.port, name, self.started)
            client.handshake(self.args.host, self.args.port, 2)
            client.login()
            client.configuration()
            client.enter_play()
            self.clients.append(client)
            client.log("configuration acknowledged")

    def pump_all(self):
        for client in self.clients:
            if client.alive:
                self.pump(client)
                if client.joined:
                    client.repeat_position()

    def wait_until(self, predicate, seconds):
        """Pump every socket until `predicate` holds. Returns whether it did."""
        deadline = time.time() + seconds
        while time.time() < deadline:
            self.pump_all()
            if predicate():
                return True
            time.sleep(0.01)
        return False

    def wait(self, seconds):
        self.wait_until(lambda: False, seconds)

    def hub_only(self, during):
        """Whether the lobby is still in the hub, where the sweep can work.

        The sweep changes kits, and a committed match refuses to: every `/kit`
        after the countdown commits is answered with a red line and ignored.
        So a sweep roster the lobby will start on does not fail the run, it
        silently stops proving anything. That is how this last broke -- three
        clients against thresholds that had moved under them produced twelve
        real ability results, three that were only `You cannot change kit once
        the game has started.`, and then a loop with no exit, because the match
        it was waiting to run had already run without it.

        Checking the phase costs nothing and turns all of that into one line.
        `self.phase` is driven by the server's own broadcasts, so this asks the
        server what state it is in rather than deciding from a player count.
        It therefore stays correct whatever the thresholds are, and it is not a
        second copy of them.
        """
        if self.phase == "hub":
            return True
        self.hub_lost = True
        # Logged here rather than left to the caller. The caller's job is to
        # stop, and the first version of this only recorded the line in
        # `sweep_failures`, which `sweep` prints at the end of a sweep it no
        # longer reaches -- so the run failed with the reason nowhere in the
        # transcript. A guard that stops the right run for a reason nobody can
        # read is most of a guard.
        self.log("SWEEP ABANDONED %s" % hub_lost(during, len(self.clients)))
        return False

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
            client.teleported_to.append((x, y, z))
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
            client.health_log.append(health)
            client.health_at = time.time()
            client.log("<- health %.2f/20" % health)
        elif packet_id == S2C_ADD_ENTITY:
            # `ClientboundAddEntityPacket`: entity id (varint), uuid (16 bytes),
            # then the entity type as a varint registry id. That is all this
            # needs -- it is counting that a projectile was *drawn*, not
            # decoding where it went. The type id is recorded so the census can
            # say what appeared.
            _entity_id, off = take_var_int(payload)
            off += 16
            type_id, _ = take_var_int(payload, off)
            client.spawns.append(type_id)
            client.log("<- add_entity type=%d" % type_id)
        elif packet_id == S2C_SET_TITLE_TEXT:
            text, _ = take_nbt_string(payload, 0)
            client.log("<- title: %s" % text)
        elif packet_id == S2C_SOUND:
            heard = take_sound(payload)
            if heard is None:
                self.sound_failures.append(
                    "a sound arrived as a registry id; hyperion sends no id table, "
                    "so a client would have nothing to resolve it against"
                )
                return
            client.sounds.append(heard)
            sound_id, _source, at, volume, pitch = heard
            client.log(
                "<- sound %s at (%.1f, %.1f, %.1f) vol %.2f pitch %.2f"
                % ((sound_id,) + at + (volume, pitch))
            )
        elif packet_id == S2C_LEVEL_PARTICLES:
            particle_id, at, count = take_particles(payload)
            self.window_particles.append((particle_id, at, count))
            client.log(
                "<- particles %d x%d at (%.1f, %.1f, %.1f)"
                % ((particle_id, count) + at)
            )
        elif packet_id == S2C_SET_ACTION_BAR_TEXT:
            text, _ = take_nbt_string(payload, 0)
            client.action_bar.append(text)
            client.log("<- action bar: %s" % text)
        elif packet_id == S2C_SET_ENTITY_MOTION:
            entity, offset = take_var_int(payload)
            motion, _ = take_lp_vec3(payload, offset)
            speed = math.sqrt(sum(component * component for component in motion))
            if speed > 1e-4:
                self.window_motions[entity] = motion
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
        # `ClientboundContainerSetSlot` numbers the whole inventory, and the
        # hotbar starts at 36. Everything else in this file talks about the nine
        # slots a player sees, which is also what the ability registry means by
        # a slot.
        slot -= HOTBAR_START_SLOT
        if not 0 <= slot < 9:
            return
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
        # Both of these are unicast to whoever asked, so they arrive before the
        # narrator check below and would otherwise be discarded.
        if text.startswith("Kit set to "):
            client.kit = text[len("Kit set to ") :].rstrip(".")
            return
        if text.startswith(MANIFEST_PREFIX):
            self.manifest.append(json.loads(text[len(MANIFEST_PREFIX) :]))
            return
        if text.startswith(MANIFEST_END_PREFIX):
            self.manifest_expected = int(text[len(MANIFEST_END_PREFIX) :])
            return

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

    # --- the ability sweep ---------------------------------------------

    def fetch_manifest(self):
        """Ask the server what every kit can do, and believe only the answer."""
        self.clients[0].command("abilities")
        done = lambda: (
            self.manifest_expected is not None
            and len(self.manifest) >= self.manifest_expected
        )
        if not self.wait_until(done, 30.0):
            self.sweep_failures.append(
                "the server answered /abilities with %d of %s lines"
                % (len(self.manifest), self.manifest_expected)
            )
            return False

        # An ability that declares nothing is one no gate can check, so the
        # roster is refused rather than swept: this is the property that makes
        # everything below mandatory instead of optional.
        silent = [
            "%s / %s" % (entry["kit"], entry["name"])
            for entry in self.manifest
            if not entry["proves"]
        ]
        if silent:
            self.sweep_failures.append(
                "these abilities declare no observable effect, so nothing can "
                "test them: %s" % ", ".join(silent)
            )
            return False

        kits = sorted({entry["kit"] for entry in self.manifest})
        self.prove(
            "the server published its ability registry",
            "%d abilities across %d kits (%s), every one of them declaring what "
            "a client would see" % (len(self.manifest), len(kits), ", ".join(kits)),
        )
        return True

    def sweep(self):
        """Fire every ability the registry names and check what it promised."""
        # Before the manifest rather than after, because the sweep is ten
        # minutes and a lobby that has already started makes every result in it
        # meaningless. The roster joined a moment ago, so a countdown it was
        # enough to trigger has had time to be announced.
        if not self.hub_only("before the sweep began"):
            return
        if not self.fetch_manifest():
            return

        by_kit = {}
        for entry in self.manifest:
            by_kit.setdefault(entry["kit"], []).append(entry)

        for done, (kit, abilities) in enumerate(by_kit.items()):
            # Again per kit: the roster is constant, but a player who is not
            # this harness can join the same server and push it over.
            if not self.hub_only("with %d of %d kits swept" % (done, len(by_kit))):
                return
            self.log("== %s ==" % kit)
            if not self.select_kit(kit):
                self.sweep_failures.append("%s: the kit never equipped" % kit)
                continue
            # Starting abilities first, then the Smash Crystal's, so the
            # crystal's twenty-second window is not spent on anything else.
            for entry in sorted(abilities, key=lambda e: (e["ultimate"], e["slot"])):
                self.exercise(entry)

        if self.sweep_failures:
            for line in self.sweep_failures:
                self.log("ABILITY FAILED %s" % line)
        else:
            self.prove(
                "every declared ability did what it declared",
                "%d abilities fired by a real client, each one checked against "
                "the effects its own registry entry names" % len(self.manifest),
            )
        if self.sound_failures:
            for line in self.sound_failures:
                self.log("SOUND FAILED %s" % line)
        else:
            self.prove(
                "every declared ability was heard",
                "%d abilities fired by a real client, each one answered by a "
                "ClientboundSoundPacket carrying the vanilla sound event its own "
                "registry entry names" % len(self.manifest),
            )
        if self.particle_failures:
            for line in self.particle_failures:
                self.log("PARTICLE FAILED %s" % line)
        else:
            self.prove(
                "every declared ability was seen",
                "%d abilities fired by a real client, each one answered by a "
                "ClientboundLevelParticlesPacket with a non-zero count"
                % len(self.manifest),
            )
        if self.cooldown_failures:
            for line in self.cooldown_failures:
                self.log("COOLDOWN FAILED %s" % line)
        elif self.cooldown_results:
            self.prove(
                "cooldowns refused a second use",
                "%d abilities answered a second right-click inside their "
                "cooldown with %r on the action bar"
                % (len(self.cooldown_results), COOLDOWN_REFUSAL),
            )

        self.prove_boundary()

    def prove_boundary(self):
        """Walk a client past the hub wall and prove it is shoved back.

        The load-bearing assertion, and the reason it is on the packet and not
        on where the player ends up: a bounds check that teleported a stray
        player back would also leave them in bounds, and read to a test that
        only checked position exactly like this one. What the operator ruled out
        is the teleport -- it rubber-bands and reads as lag -- so what is proved
        is the shove itself: a `ClientboundSetEntityMotion` pointing back
        inside. A control client standing in the middle proves the wall does not
        grab someone who never left.
        """
        if len(self.clients) < 2:
            return
        escapee, control = self.clients[0], self.clients[1]
        if escapee.entity_id is None or control.entity_id is None:
            return

        self.window_motions.clear()
        # Up over the glass wall, out past x=21, and hold there. The steps stay
        # inside hyperion's speed check; a client claiming a position is not
        # collision-checked against the wall, which is the whole point -- a real
        # double jump clears it too.
        escapee.walk(
            [
                (escapee.position[0], HUB_CLEAR_Y, escapee.position[2]),
                (BEYOND_WALL_X, HUB_CLEAR_Y, 0.0),
                (BEYOND_WALL_X, SWEEP_Y + 1.0, 0.0),
            ],
            note="past the hub's east wall",
        )
        self.wait_until(lambda: escapee.arrived(), 15.0)
        # Long enough for the bounds system to see the mirror out of bounds and
        # for the motion packet to come back.
        self.wait(1.0)

        push = self.window_motions.get(escapee.entity_id)
        control_push = self.window_motions.get(control.entity_id)

        if push is not None and push[0] < -1e-4:
            self.prove(
                "the hub shoves you back inside",
                "%s walked to x=%.0f past the hub wall and the server sent "
                "SetEntityMotion (%.3f, %.3f, %.3f), pointing back inside"
                % (escapee.name, BEYOND_WALL_X, push[0], push[1], push[2]),
            )
        else:
            self.sweep_failures.append(
                "a client walked past the hub wall and the server did not shove "
                "it back: motion=%r" % (push,)
            )
        if control_push is not None and abs(control_push[0]) > 1e-4:
            self.sweep_failures.append(
                "%s never left the hub and was shoved anyway: motion=%r"
                % (control.name, control_push)
            )

        # Back to a sweep spot, so nothing after this reads a client stranded
        # outside.
        escapee.walk(
            [
                (BEYOND_WALL_X, HUB_CLEAR_Y, 0.0),
                (SWEEP_X, HUB_CLEAR_Y, SWEEP_Z),
                (SWEEP_X, SWEEP_Y, SWEEP_Z),
            ]
        )
        self.wait_until(lambda: escapee.arrived(), 15.0)

    def spares(self, testing):
        """A mob for each victim that is not the one being tested.

        One player per mob is the selector's rule and `/kit` obeys it too, so
        the sweep cannot stand three clients on the kit under test the way it
        used to: the attacker takes it and the other two are refused. The
        attacker is the one whose abilities are being fired, so the attacker is
        the one who gets it, and the victims stand on whatever is left.

        Not any two. The kits named by `--kits` are reserved, because the match
        that runs after the sweep hands those out and a victim still holding
        one would refuse the client it is meant for. Everything else is taken
        in registry order, which makes the pairing the same on every run.
        """
        reserved = {kit.strip() for kit in self.args.kits.split(",")}
        reserved.add(testing)
        seen = []
        for entry in self.manifest:
            if entry["kit"] not in reserved and entry["kit"] not in seen:
                seen.append(entry["kit"])
        return seen[: len(self.clients) - 1]

    def plan_for(self, kit):
        """Who plays what while `kit`'s abilities are being fired."""
        plan = dict(zip((c.name for c in self.clients[1:]), self.spares(kit)))
        plan[self.clients[0].name] = kit
        if len(plan) < len(self.clients):
            self.sweep_failures.append(
                "the roster has too few mobs to give %d clients one each while "
                "testing %s" % (len(self.clients), kit)
            )
            return None
        return plan

    def unwanted(self, plan):
        """A mob nobody wants and nobody is standing on, to step aside onto."""
        held = {client.kit for client in self.clients}
        wanted = set(plan.values())
        for entry in self.manifest:
            if entry["kit"] not in wanted and entry["kit"] not in held:
                return entry["kit"]
        return None

    def equip(self, plan):
        """Put every client on the mob `plan` names, in a workable order.

        Ordering matters now that one player holds one mob: a client that has
        not yet moved off X refuses the client that wants X, and asking all of
        them at once means whoever asked second loses. So each pass asks only
        the clients whose mob is free or already theirs, waits, and goes round
        again.

        That is not enough on its own, and the first run of it proved so: when
        the sweep moves from the Sky Squid to the Creeper, the attacker is
        standing on the Sky Squid a victim now wants and the victim is standing
        on the Creeper the attacker now wants. Nobody can go first. One of them
        steps off onto a mob nobody in the plan wants, and the pass after that
        is ordinary.

        Everybody is asked even when they already hold the right mob, because
        re-picking is the game's own way of saying "start of a life" and is how
        the sweep gets its victims back to full health.
        """
        pending = list(self.clients)
        for _ in range(4 * len(self.clients)):
            if not pending:
                break
            owner = {
                client.kit: client.name
                for client in self.clients
                if client.kit is not None
            }
            ready = [
                client
                for client in pending
                if owner.get(plan[client.name], client.name) == client.name
            ]
            if ready:
                for client in ready:
                    client.kit = None
                    client.command("kit %s" % plan[client.name])
                self.wait_until(
                    lambda: all(client.kit == plan[client.name] for client in ready),
                    15.0,
                )
                pending = [
                    client for client in pending if client.kit != plan[client.name]
                ]
                continue

            stuck = pending[0]
            aside = self.unwanted(plan)
            if aside is None:
                break
            stuck.kit = None
            stuck.command("kit %s" % aside)
            self.wait_until(lambda: stuck.kit == aside, 15.0)
        return all(client.kit == plan[client.name] for client in self.clients)

    def select_kit(self, kit):
        """Put the attacker on `kit` and the victims on something else.

        Also restores everybody to full health, which is what makes it worth
        calling again between attempts.
        """
        plan = self.plan_for(kit)
        if plan is None or not self.equip(plan):
            return False
        # The hotbar is rebuilt a tick later, in PostUpdate, and an ability
        # cannot be used before the item backing it exists.
        wanted = {
            entry["slot"]
            for entry in self.manifest
            if entry["kit"] == kit and not entry["ultimate"]
        }
        attacker = self.clients[0]
        self.wait_until(lambda: wanted <= set(attacker.hotbar), 5.0)
        return True

    def stage(self, entry, attempt):
        """Put the three of them on their marks, at full health, the attacker
        looking down the line at both victims.

        Health first: a victim left on nothing by the last ability is a victim
        every splash skips, and an ability would then read as doing nothing when
        it was the arrangement that was spent. Picking the kit again is the
        game's own way of saying "start of a life", so it is what is used.

        Each victim re-picks the mob it is already standing on rather than the
        attacker's, which is the only thing one player per mob leaves open. It
        also makes the victims a constant across the whole sweep instead of
        changing with every kit, so an ability that reads as weak is weak and
        not measured against a tougher target than the last one.
        """
        plan = self.plan_for(entry["kit"])
        if plan is None:
            return
        for victim in self.clients[1:]:
            victim.kit = None
            victim.command("kit %s" % plan[victim.name])
        self.wait_until(
            lambda: all(
                victim.kit == plan[victim.name] for victim in self.clients[1:]
            ),
            5.0,
        )

        base = SWEEP_Z + attempt * ATTEMPT_STRIDE
        marks = [base, base + NEAR_VICTIM, base + FAR_VICTIM]
        for client, z in zip(self.clients, marks):
            client.aim(SWEEP_YAW)
            self.route_in_hub(client, (SWEEP_X, SWEEP_Y, z))
        if not self.wait_until(
            lambda: all(client.arrived() for client in self.clients), 15.0
        ):
            self.log("a client never reached its mark; the arrangement may be wrong")
        # Long enough for hyperion to mirror the last claim onto the components
        # the abilities read.
        self.wait(0.2)

    @staticmethod
    def route_in_hub(client, mark):
        """Walk `client` to `mark`, over the top of everything in the way."""
        if distance(client.position, mark) < 0.5:
            client.path = []
            return
        x, y, z = client.position
        client.walk(
            [
                (x, HUB_CLEAR_Y, z),
                (mark[0], HUB_CLEAR_Y, mark[2]),
                mark,
            ]
        )

    def arm(self, entry):
        """Pick up a Smash Crystal, if this ability needs one."""
        if not entry["ultimate"]:
            return True
        attacker = self.clients[0]
        if entry["slot"] in attacker.hotbar:
            return True
        attacker.command("crystal")
        if self.wait_until(lambda: entry["slot"] in attacker.hotbar, 6.0):
            return True
        self.sweep_failures.append(
            "%s / %s: the Smash Crystal never put anything in slot %d"
            % (entry["kit"], entry["name"], entry["slot"])
        )
        return False

    def wound_attacker(self):
        """Take the attacker off full health.

        `ClientboundSetHealth` is scaled to twenty, so an ability that heals to a
        raised maximum -- Mooshroom Madness is exactly that -- lands on the same
        number a full-health player was already showing. Somebody has to hit them
        first or the heal is invisible on the wire.
        """
        attacker, victim = self.clients[0], self.clients[1]
        before = attacker.health
        victim.attack(attacker)
        self.wait_until(
            lambda: attacker.health is not None
            and (before is None or attacker.health < before - 0.05),
            2.0,
        )

    def hit_caster(self):
        """One hit on the caster from the near victim. Returns whether it landed.

        The wire fact behind `shields_caster`, in both directions: inside the
        window this has to send no `ClientboundSetHealth` for the caster, and
        outside it, it has to send one. Only the pair proves anything -- a
        shield that never lifts and a server that cannot deal damage look
        identical from one probe.
        """
        attacker, victim = self.clients[0], self.clients[1]
        before = attacker.health
        victim.attack(attacker)
        return self.wait_until(
            lambda: attacker.health is not None
            and before is not None
            and attacker.health < before - 0.05,
            SHIELD_PROBE_WINDOW,
        )

    def probe_shield(self, entry, landed_before):
        """Whether the same hit landed before the press and was refused after it.

        Taken around the press rather than around the *window*, because the two
        shields in the roster are one second and nineteen: a check shaped as
        "wait it out and hit again" works for the first and cannot afford the
        second. Bracketing the press needs no knowledge of the duration at all.

        What this therefore does not check is that the window ever ends. That is
        the effect module's behaviour rather than any one kit's, and the Rust
        side proves it once in `tests/contract.rs`.

        False when the entry claims no shield. The probe costs the caster
        health, and running it on all fifty-one would perturb every other
        reading in the sweep -- `heals_caster` most of all, which is measured as
        the caster's health going *up*.
        """
        if "shields_caster" not in entry["proves"]:
            return False
        return landed_before and not self.hit_caster()

    def revive_near_victim(self):
        """Stand the near victim back up, and say whether it landed.

        Re-picking the kit they are already on, which is the game's own way of
        saying "start of a life" and the only heal this harness has; `stage`
        uses it for the same reason at the top of every attempt.

        Needed a second time here because the melee probe runs *after* the
        observation window, by which point the ability under test has had two
        seconds to work on the very player the probe swings at. The sweep used
        to swing at a corpse: Mooshroom Madness spends that window throwing cows
        and had killed the victim outright, and Target Laser spends it standing
        in the fallout of the two abilities tested before it, which had done the
        same. A swing that lands on nought health takes nothing off, and reads
        exactly like a swing that was never buffed.
        """
        victim = self.clients[1]
        mob = victim.kit
        if mob is None:
            return False
        victim.kit = None
        victim.command("kit %s" % mob)
        return self.wait_until(
            lambda: victim.kit == mob
            and victim.health is not None
            and victim.health >= MELEE_PROBE_HEADROOM,
            5.0,
        )

    def measure_melee(self):
        """What one melee swing at the near victim takes off, right now.

        `None` when no swing could be attributed, which the caller must not
        round into zero: a probe that answers 0.0 because it could not measure
        anything is a baseline every later swing beats, and `buffs_melee` would
        then pass for an ability that does nothing at all.

        This used to return "health the victim lost in the two seconds after the
        swing", which is not the swing. It is the swing plus everything else in
        flight, and during an ultimate it is mostly everything else. Wolf's
        Frenzy passed the sweep on exactly that reading: 5.60, of which 4.48 was
        the swing and 1.12 was a lunge that happened to arrive in the same pump.
        Cow's and Guardian's failed on the other end of it, swinging at a victim
        the ability under test had already killed.

        So a reading is only taken when it can be told apart from everything
        else: the victim is stood back up, given `MELEE_PROBE_QUIET` with
        nothing landing on them, hit once, and then *exactly one* health packet
        has to arrive inside `MELEE_PROBE_WINDOW`. Two packets means something
        else landed alongside the swing and neither of them can be called the
        swing -- which is not a guess to be resolved by taking the larger, it is
        a reading to be taken again. The sweep's own leftovers are what make
        that necessary: Zombie's Horde was still hitting the victim for 2.5 two
        kits later, and landed in the same tick as a probe swing.
        """
        attacker, victim = self.clients[0], self.clients[1]
        silent = 0

        for _ in range(MELEE_PROBE_TRIES):
            if (
                victim.health is None or victim.health < MELEE_PROBE_HEADROOM
            ) and not self.revive_near_victim():
                self.log("the melee probe could not stand %s back up" % victim.name)
                self.melee_unmeasured += 1
                return None
            if not self.wait_until(
                lambda: time.time() - victim.health_at >= MELEE_PROBE_QUIET,
                MELEE_PROBE_SETTLE,
            ):
                continue
            before = victim.health
            if before is None or before < MELEE_PROBE_HEADROOM:
                continue

            seen = len(victim.health_log)
            attacker.attack(victim)
            self.wait(MELEE_PROBE_WINDOW)
            landed = victim.health_log[seen:]
            if len(landed) == 1:
                return before - landed[0]
            if landed:
                self.log(
                    "%d health packets arrived with the melee probe's swing at %s, "
                    "so none of them is the swing; probing again"
                    % (len(landed), victim.name)
                )
            else:
                silent += 1

        if silent == MELEE_PROBE_TRIES:
            # Every swing was answered with nothing at all. That is a real zero
            # -- the swing takes nothing off -- and not a failed measurement.
            return 0.0
        self.log(
            "no melee swing at %s could be told apart from the damage around it"
            % victim.name
        )
        self.melee_unmeasured += 1
        return None

    def baseline(self, entry):
        # The melee probe is taken before the health snapshot, or the swing it
        # makes reads as the ability having hurt somebody.
        melee = self.measure_melee() if "buffs_melee" in entry["proves"] else None
        self.window_motions.clear()
        self.window_particles.clear()
        for client in self.clients:
            client.action_bar.clear()
            client.teleported_to.clear()
            client.sounds.clear()
            client.spawns.clear()
        return {
            "melee": melee,
            "health": {client.name: client.health for client in self.clients},
            "position": {client.name: client.position for client in self.clients},
        }

    def press(self, entry):
        attacker = self.clients[0]
        attacker.use_slot(entry["slot"], "(%s)" % entry["name"])
        if entry["charge_time"] is not None:
            self.wait(entry["charge_time"] + 0.2)
            attacker.release_slot(entry["slot"], "(%s, fully charged)" % entry["name"])

    def observe(self, entry, before, held=None):
        """Everything from `entry`'s declaration that actually reached a client.

        `held` is `probe_shield`'s reading, which has to be taken before this
        runs and cannot be taken here. See that method.
        """
        attacker = self.clients[0]
        victims = self.clients[1:]
        found = {}

        def look():
            for victim in victims:
                was = before["health"].get(victim.name)
                if was is not None and victim.health is not None and victim.health < was - 0.05:
                    found.setdefault(
                        "hurts_target",
                        "%s went from %.2f to %.2f health" % (victim.name, was, victim.health),
                    )
                motion = self.window_motions.get(victim.entity_id)
                if motion:
                    found.setdefault(
                        "launches_target",
                        "%s took (%.3f, %.3f, %.3f) blocks/tick" % ((victim.name,) + motion),
                    )
            motion = self.window_motions.get(attacker.entity_id)
            if motion:
                found.setdefault(
                    "launches_caster",
                    "the caster took (%.3f, %.3f, %.3f) blocks/tick" % motion,
                )
            for at in attacker.teleported_to:
                moved = distance(at, before["position"][attacker.name])
                if moved > 1.0:
                    found.setdefault(
                        "teleports_caster",
                        "the caster was moved %.1f blocks, to (%.1f, %.1f, %.1f)"
                        % ((moved,) + at),
                    )
            was = before["health"].get(attacker.name)
            if was is not None and attacker.health is not None and attacker.health > was + 0.05:
                found.setdefault(
                    "heals_caster",
                    "the caster went from %.2f to %.2f health" % (was, attacker.health),
                )
            return set(entry["proves"]) <= set(found)

        self.wait_until(look, SWEEP_WINDOW)

        if "buffs_melee" in entry["proves"]:
            after = self.measure_melee()
            # Both readings or neither. `measure_melee` answers `None` when it
            # could not take an honest reading, and treating that as zero would
            # give the after-probe a baseline it beats by standing still.
            if (
                before["melee"] is not None
                and after is not None
                and after > before["melee"] + 0.05
            ):
                found["buffs_melee"] = (
                    "a melee swing took %.2f health where the same swing took "
                    "%.2f before" % (after, before["melee"])
                )

        # The distinguishing word is *keeps*. Every ability in the game can take
        # health off somebody once, and `hurts_target` above is already that. So
        # nothing is cast, the health each victim is showing right now is
        # written down, and the question is whether a further
        # `ClientboundSetHealth` arrives for any of them.
        if "afflicts_target" in entry["proves"]:
            watched = {victim.name: victim.health for victim in victims}
            self.wait(LINGER_WINDOW)
            for victim in victims:
                was = watched.get(victim.name)
                if was is not None and victim.health is not None and victim.health < was - 0.05:
                    found.setdefault(
                        "afflicts_target",
                        "%s went on losing health with nothing cast: %.2f to %.2f"
                        % (victim.name, was, victim.health),
                    )

        # `held` is the pair of probes taken either side of the press, in
        # `exercise`. Both halves, because a shield that never lifts and a
        # server that cannot deal damage produce the same single reading -- and
        # bracketing the press rather than the window is what lets one shape
        # cover a one-second shield and a nineteen-second one.
        if held:
            found["shields_caster"] = (
                "the same hit landed before the cast and sent no health packet "
                "after it"
            )

        # Nothing is cast. The claim is that the ability is still acting,
        # because it left a mode on the caster rather than doing one thing, so
        # what is watched for is any of the packets an ability produces
        # arriving again on its own.
        if "sustains" in entry["proves"]:
            watched = {client.name: client.health for client in self.clients}
            self.window_motions.clear()
            self.wait(LINGER_WINDOW)
            for client in self.clients:
                was = watched.get(client.name)
                if was is not None and client.health is not None and client.health < was - 0.05:
                    found.setdefault(
                        "sustains",
                        "%s lost health with nothing pressed: %.2f to %.2f"
                        % (client.name, was, client.health),
                    )
            if "sustains" not in found and self.window_motions:
                found["sustains"] = "%d motion packets arrived with nothing pressed" % len(
                    self.window_motions
                )
        return found

    def window_sounds(self):
        """Every sound id any client has been sent since the last baseline."""
        return {heard[0] for client in self.clients for heard in client.sounds}

    def probe_cooldown(self, entry):
        """Right-click again straight away and expect to be told no."""
        if entry["cooldown"] < 1.0 or entry["refunds_on_hit"]:
            return
        # A few ticks after the press, not the same one. Releasing a held item
        # is a separate packet from using one, and a second use that arrives in
        # the same tick as the release is handled before it: the ability is
        # still charging rather than on cooldown, so the press restarts the
        # charge and there is nothing to refuse.
        self.wait(0.25)
        attacker = self.clients[0]
        attacker.action_bar.clear()
        attacker.use_slot(entry["slot"], "(again, expecting a refusal)")
        refused = self.wait_until(
            lambda: any(COOLDOWN_REFUSAL in line for line in attacker.action_bar), 0.8
        )
        label = "%s / %s" % (entry["kit"], entry["name"])
        if refused:
            self.cooldown_results.append(
                "%-42s refused inside its %.1fs cooldown" % (label, entry["cooldown"])
            )
        else:
            self.cooldown_failures.append(
                "%s has a %.1fs cooldown and let a second use through"
                % (label, entry["cooldown"])
            )

    def exercise(self, entry):
        label = "%s / %s" % (entry["kit"], entry["name"])
        outstanding = list(entry["proves"])
        self.melee_unmeasured = 0
        evidence = []
        heard = False
        seen = False

        for attempt in range(SWEEP_ATTEMPTS):
            if not outstanding and heard:
                break
            self.stage(entry, attempt)
            if not self.arm(entry):
                return
            if "heals_caster" in outstanding:
                self.wound_attacker()

            # Before the press, so the shield probes bracket it.
            landed_before = "shields_caster" in entry["proves"] and self.hit_caster()

            before = self.baseline(entry)
            self.press(entry)
            held = self.probe_shield(entry, landed_before)
            if attempt == 0:
                self.probe_cooldown(entry)
            for name, note in self.observe(entry, before, held).items():
                if name in outstanding:
                    outstanding.remove(name)
                    evidence.append("%s: %s" % (name, note))
            # After `observe`, which is what waits out the window the sound
            # arrives in.
            if entry["sound"] in self.window_sounds():
                if not heard:
                    evidence.append("heard: %s" % entry["sound"])
                heard = True
            # Every ability draws something. Which particle is the kit's
            # business and not this file's, so the claim is only that a real
            # client was sent one and that it carries a real count -- a
            # `level_particles` with a count of zero is a packet that spends
            # bandwidth and draws nothing.
            drawn = [seen_at for seen_at in self.window_particles if seen_at[2] > 0]
            if drawn:
                if not seen:
                    particle_id, at, count = drawn[0]
                    evidence.append(
                        "seen: particle %d x%d at (%.1f, %.1f, %.1f)"
                        % ((particle_id, count) + at)
                    )
                seen = True
            if outstanding or not heard or not seen:
                self.wait(entry["cooldown"] + 0.5)

        if outstanding:
            unreadable = ""
            if "buffs_melee" in outstanding and self.melee_unmeasured:
                unreadable = (
                    "; the melee probe gave up %d times because it could not "
                    "tell its own swing apart from the damage around the "
                    "victim, so this may be the probe and not the ability"
                    % self.melee_unmeasured
                )
            self.sweep_failures.append(
                "%s declares %s and did not do it%s"
                % (label, ", ".join(outstanding), unreadable)
            )
        if not heard:
            self.sound_failures.append(
                "%s declares the sound %s and firing it sent no such packet"
                % (label, entry["sound"])
            )
        if not seen:
            self.particle_failures.append(
                "%s fired and sent no level_particles a client could draw" % label
            )
        # A projectile ability now puts an entity in the world. Not declared
        # per ability -- which abilities fire one is a server-side detail this
        # file deliberately does not carry a list of -- so it is proved once,
        # in aggregate: over the whole sweep at least one `add_entity` for a
        # projectile arrives. During the sweep the only new entities are
        # projectiles, because every player spawned before it began.
        spawned = [type_id for client in self.clients for type_id in client.spawns]
        if spawned:
            self.prove(
                "a projectile was drawn",
                "%s fired and the server sent add_entity type=%d" % (label, spawned[0]),
            )
            evidence.append("spawned: entity type %d" % spawned[0])
        self.sweep_results.append(
            "%-42s %s" % (label, "; ".join(evidence) or "nothing reached a client")
        )

    # --- the script ----------------------------------------------------

    def run(self):
        # The sweep's roster, which the gate declares and `hub_only` checks
        # against the server. Nothing is subtracted from anything here: the
        # number that matters is the largest roster this lobby will not start
        # on, and that is the server's to know, not this file's. The rest of
        # the clients join once the sweep is done.
        self.connect(self.args.sweep_clients)
        if not self.wait_until(lambda: all(c.joined for c in self.clients), 60.0):
            self.log("clients never reached the world")
            return self.report()

        if self.args.abilities:
            self.sweep()
            # Straight to the report. Running the match now would be running it
            # in a countdown the sweep already lost to, and the transcript
            # would show a mostly-passing match under a failed run -- which is
            # exactly the reading that let this bug survive its first sighting.
            if self.hub_lost:
                return self.report()

        self.connect(self.args.clients - len(self.clients))
        kits = [kit.strip() for kit in self.args.kits.split(",")]
        deadline = time.time() + self.args.seconds

        step = 0
        script = self.build_script(kits)
        while time.time() < deadline:
            self.pump_all()
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
        # One call rather than one command per client, because one player per
        # mob makes the order matter: a client still holding the mob the next
        # one was assigned refuses it. `equip` sorts that out.
        at(0.2, lambda: self.equip(dict(zip((c.name for c in self.clients), kits))))

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
        at(gathered, self.take_aim)
        at(0.3, self.smash)
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

    def take_aim(self):
        """Turn to face the nearest victim.

        Its own step, a beat before the ability, because a rotation does not
        take effect in the tick it arrives: hyperion decodes packets after the
        phase that copies yaw onto the component abilities read, so an ability
        fired in the same tick as the turn fires along the old bearing. The
        original script hid this by using a radial ability; anything aimed does
        not have that luxury.
        """
        living = self.victims()
        if living:
            self.clients[0].look_at(living[0])

    def smash(self):
        """The attacker's own launching ability, chosen out of the registry."""
        self.motions.clear()
        attacker = self.clients[0]
        slot = self.launching_slot(attacker.kit)
        attacker.use_slot(slot, "(the knockback check's single-impulse ability)")

    def launching_slot(self, kit):
        """The slot the knockback check fires, and the registry's opinion of it.

        A fixed slot rather than the first launching ability the registry
        offers, because the check below solves one hidden strength out of two
        different functions of it and that only works on a single impulse. Iron
        Golem's Fissure puts three of its fourteen columns on one victim in one
        tick and hyperion sums them into one velocity packet, which solves to
        nothing at all. The sweep above is what covers every ability; this step
        is about the model.

        What the registry is for here is checking the choice rather than making
        it: a slot holding something that does not launch is a slot this check
        cannot use, and saying so beats a silent miss.

        A recorded failure and not a log line, because the miss it catches is
        the quiet one. When every kit's layout shifted one slot to the left, an
        `--ability-slot` left behind pointed at an empty key: the check pressed
        nothing, saw no motion, and reported a broken knockback model. An empty
        slot produced no message at all, because there was no registry entry to
        disagree with.
        """
        slot = self.args.ability_slot
        for_kit = sorted(
            (entry for entry in self.manifest if entry["kit"] == kit),
            key=lambda entry: entry["slot"],
        )
        # `--no-abilities` skips the sweep and leaves the manifest empty, in
        # which case there is nothing to check the choice against.
        if not for_kit:
            return slot
        declared = [entry for entry in for_kit if entry["slot"] == slot]
        if not declared:
            self.late_failures.append(
                "--ability-slot %d is empty on %s, so the knockback check "
                "presses a key holding nothing; the registry gives that kit %s"
                % (
                    slot,
                    kit,
                    ", ".join("%d:%s" % (e["slot"], e["name"]) for e in for_kit),
                )
            )
        elif "launches_target" not in declared[0]["proves"]:
            self.late_failures.append(
                "--ability-slot %d on %s is %s, which the registry says does "
                "not launch anybody" % (slot, kit, declared[0]["name"])
            )
        return slot

    def melee(self):
        attacker = self.clients[0]
        for victim in self.victims():
            attacker.attack(victim)

    def check_impact_sound(self, victim):
        """A hit is heard where it landed, at a pitch and volume that say how
        hard.

        The `knockback from an ability` proof above says the launch reached a
        client. This says the *feedback* did, which is the whole reason this
        change exists: the physics were already right and the player could not
        feel them. The two are checked at the same moment because they come from
        the same event.
        """
        vx, vy, vz = victim.position
        impacts = [
            heard
            for client in self.clients
            for heard in client.sounds
            if heard[0] == IMPACT_SOUND
        ]
        near = [
            heard
            for heard in impacts
            if distance(heard[2], (vx, vy, vz)) < IMPACT_TOLERANCE
        ]
        if not near:
            self.log(
                "no impact sound arrived near %s at (%.1f, %.1f, %.1f); %d were "
                "heard elsewhere" % (victim.name, vx, vy, vz, len(impacts))
            )
            return

        levels = sorted({(round(h[4], 3), round(h[3], 3)) for h in impacts})
        _sound_id, _source, at, volume, pitch = near[0]
        spread = ""
        if len(levels) > 1:
            softest, loudest = levels[-1], levels[0]
            spread = (
                ". Across the match the hits ranged from pitch %.2f at volume "
                "%.2f to pitch %.2f at volume %.2f, so a jab and a smash do not "
                "sound the same" % (softest[0], softest[1], loudest[0], loudest[1])
            )
        self.prove(
            "a hit was heard where it landed",
            "%s took an impact at (%.1f, %.1f, %.1f), %.2f blocks from where "
            "they stand, at pitch %.2f and volume %.2f%s"
            % (
                victim.name,
                at[0],
                at[1],
                at[2],
                distance(at, (vx, vy, vz)),
                pitch,
                volume,
                spread,
            ),
        )

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
            self.check_impact_sound(victim)
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
        if self.sweep_results:
            print("", flush=True)
            self.log("=== every ability the server declares, and what it did ===")
            for line in self.sweep_results:
                self.log("    %s" % line)
        if self.cooldown_results:
            print("", flush=True)
            self.log("=== cooldowns, checked by right-clicking twice ===")
            for line in self.cooldown_results:
                self.log("    %s" % line)

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
        if self.late_failures:
            for line in self.late_failures:
                self.log("FAILED %s" % line)
            self.log(
                "RESULT: %d check(s) were aimed at something the server's own "
                "registry does not have" % len(self.late_failures)
            )
            return 1
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
        "--sweep-clients",
        type=int,
        default=3,
        help="how many clients are in the world for the ability sweep. Must be "
        "a roster this server's lobby will not start a countdown on: the sweep "
        "changes kits and a committed match refuses to. The run fails naming "
        "this flag if the lobby starts anyway, so it is checked and not "
        "trusted. Minimum 2, because an ability that hurts or heals a target "
        "needs a target",
    )
    parser.add_argument(
        "--kits",
        default="Iron Golem,Skeleton,Enderman,Slime",
        help="comma-separated, one per client; the first one attacks",
    )
    parser.add_argument(
        "--ability-slot",
        type=int,
        default=2,
        help="hotbar slot the match's knockback check fires; 2 is Iron Golem's "
        "Seismic Slam, which lands one hit and so solves against the model",
    )
    parser.add_argument(
        "--no-abilities",
        dest="abilities",
        action="store_false",
        help="skip the registry sweep and run only the match",
    )
    parser.add_argument("--seconds", type=float, default=300.0)
    parser.add_argument(
        "--void-y",
        type=float,
        default=-5.0,
        help="the Y a client claims to be at when the script wants it to die",
    )
    args = parser.parse_args()

    # Two floors, both structural. The upper one is not about the lobby: the
    # lobby's limit is the server's to state and `hub_only` is what checks it.
    if args.sweep_clients < 2:
        raise SystemExit(
            "--sweep-clients is %d: the sweep proves `hurts_target` and calls "
            "`wound_attacker` for `heals_caster`, and both are claims about a "
            "body that is not the caster's" % args.sweep_clients
        )
    if args.sweep_clients > args.clients:
        raise SystemExit(
            "--sweep-clients is %d but --clients is %d, so the match after the "
            "sweep would have fewer players than the sweep did"
            % (args.sweep_clients, args.clients)
        )

    chosen = [kit.strip() for kit in args.kits.split(",")]
    if args.clients > len(chosen):
        raise SystemExit("--kits needs one kit per client")
    # One player per mob, so two clients cannot be given the same one: the
    # second would be refused and the match would run a client short of a kit.
    if len(set(chosen[: args.clients])) < args.clients:
        raise SystemExit("--kits must name a different mob for each client")

    return Match(args).run()


if __name__ == "__main__":
    sys.exit(main())
