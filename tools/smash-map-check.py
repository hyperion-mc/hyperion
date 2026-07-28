#!/usr/bin/env python3
"""Check a Super Smash Mobs map against the blocks a real client is sent.

`smash-match.py` proves a match happens. It says so itself: its clients
"teleport rather than walk, so nothing here says the platforms have collision
from a real client's point of view". It decides which map a player landed on
from the x coordinate of a teleport, which is arithmetic over a region stride
and not a block. Nothing in the repository has ever read a block off the wire.

This does. It joins on protocol 776, decodes the world into block state ids,
turns those into block names, and compares them against the map files under
`events/smash/maps/`. Then it walks a player off the edge and descends a block
at a time to find the height the server kills at, and checks that height
against the `kill_y` the map file declares.

Reading the world takes two packets, not one, and this is the part that is easy
to get wrong: `level_chunk_with_light` carries nothing but air. `terrain.rs`
builds on `Blocks::empty`, so every column was encoded before a single block
was stamped into it, and the cached encoding is never rebuilt. What a joining
player gets is that empty chunk followed by `section_blocks_update` for every
change the column has seen since it loaded. A checker that decodes only the
chunk packet reports a world with no floor in it, which is how this one spent
its first three runs.

Three claims, in the order the run makes them:

  * **the map file's blocks are in the world** -- every block a map places, in
    every chunk this client was sent, arrives with the name the file gives it,
    and the gaps between the islands arrive as air.
  * **a player stands on them** -- the block under a spawn point is solid on
    the wire, and a client that claims to be standing there is not corrected.
  * **the kill plane is where the map says** -- descending through open air
    below an island, the server kills within a block of `kill_y` and not
    before.

The map format is re-implemented here rather than imported, for the same
reason the knockback constants are restated in `smash-match.py`: a checker that
asks the server's own parser what a file means cannot catch the parser being
wrong. This one reads the bytes and the text and has to agree with neither.
"""

from __future__ import annotations

import argparse
import bisect
import importlib.util
import math
import pathlib
import re
import struct
import sys
import time

TOOLS = pathlib.Path(__file__).resolve().parent
ROOT = TOOLS.parent
MAPS = ROOT / "events/smash/maps"


def _load_client():
    """Import `client-26.2.py`, whose file name is not a Python identifier."""
    path = TOOLS / "client-26.2.py"
    spec = importlib.util.spec_from_file_location("client_26_2", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


base = _load_client()
var_int = base.var_int
take_var_int = base.take_var_int
PROTOCOL = base.PROTOCOL

# Serverbound play ids, from
# crates/hyperion-minecraft-proto/src/generated/packet_id.rs.
C2S_ACCEPT_TELEPORTATION = 0x00
C2S_KEEP_ALIVE = 0x1C
C2S_MOVE_PLAYER_POS = 0x1E

# Clientbound play ids this file decodes.
S2C_DISCONNECT = 0x20
S2C_LEVEL_CHUNK_WITH_LIGHT = 0x2D
S2C_SECTION_BLOCKS_UPDATE = 0x54
S2C_KEEP_ALIVE = 0x2C
S2C_LOGIN = 0x31
S2C_PLAYER_POSITION = 0x48
S2C_SET_HEALTH = 0x68

# `ServerboundMovePlayerPacket` packs the ground flag into a bitfield; bit 0 is
# `ON_GROUND`, the only bit hyperion reads.
ON_GROUND = 1

# crates/hyperion/src/simulation/blocks/chunk.rs and crates/hyperion/src/lib.rs:
# the overworld spans 384 blocks from y -64, which is 24 sections of 16.
WORLD_MIN_Y = -64
SECTION_COUNT = 24

# events/smash/src/terrain.rs: the hub is region 0 and each arena sits a whole
# stride further along +X.
REGION_STRIDE = 512

POSITION_INTERVAL = 0.05


# --- the map format, restated -----------------------------------------------


class MapSpec:
    def __init__(self, name, kill_y, spawns, brushes):
        self.name = name
        self.kill_y = kill_y
        self.spawns = spawns
        self.brushes = brushes
        self._blocks = None

    @property
    def blocks(self):
        """Every block the file places, brushes applied in order.

        Later brushes overwrite earlier ones because `set_block` does, which is
        how the hub carves the inside out of its glass ring with a cylinder of
        air. A rasteriser that unioned instead would expect glass across the
        whole floor.
        """
        if self._blocks is None:
            out = {}
            for brush in self.brushes:
                for at, block in brush:
                    out[at] = block
            self._blocks = out
        return self._blocks


def _box(min_xyz, max_xyz, block):
    lo = [min(a, b) for a, b in zip(min_xyz, max_xyz)]
    hi = [max(a, b) for a, b in zip(min_xyz, max_xyz)]
    for x in range(lo[0], hi[0] + 1):
        for y in range(lo[1], hi[1] + 1):
            for z in range(lo[2], hi[2] + 1):
                yield (x, y, z), block


def _cylinder(centre, radius, height, block):
    squared = radius * radius
    for dx in range(-radius, radius + 1):
        for dz in range(-radius, radius + 1):
            if dx * dx + dz * dz > squared:
                continue
            for dy in range(height):
                yield (centre[0] + dx, centre[1] + dy, centre[2] + dz), block


def _sphere(centre, radius, block):
    squared = radius * radius
    for dx in range(-radius, radius + 1):
        for dy in range(-radius, radius + 1):
            for dz in range(-radius, radius + 1):
                if dx * dx + dy * dy + dz * dz > squared:
                    continue
                yield (centre[0] + dx, centre[1] + dy, centre[2] + dz), block


def _cone(centre, radius, depth, block):
    for level in range(depth):
        # Rust: `radius - (radius * level) / depth.max(1)`, truncating.
        shrunk = radius - (radius * level) // max(depth, 1)
        squared = shrunk * shrunk
        for dx in range(-shrunk, shrunk + 1):
            for dz in range(-shrunk, shrunk + 1):
                if dx * dx + dz * dz > squared:
                    continue
                yield (centre[0] + dx, centre[1] - level, centre[2] + dz), block


def parse_map(text):
    """events/smash/src/map.rs, in Python."""
    name = None
    kill_y = None
    spawns = []
    brushes = []
    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        words = line.split()
        directive, rest = words[0], words[1:]
        if directive == "name":
            name = line[len(directive) :].strip()
        elif directive == "author":
            pass
        elif directive == "kill_y":
            kill_y = float(rest[0])
        elif directive == "spawn":
            spawns.append(tuple(float(v) for v in rest[:3]))
        elif directive == "crystal":
            pass
        elif directive == "box":
            nums = [int(v) for v in rest[:6]]
            brushes.append(list(_box(nums[:3], nums[3:6], rest[6])))
        elif directive == "cylinder":
            nums = [int(v) for v in rest[:5]]
            brushes.append(list(_cylinder(nums[:3], nums[3], nums[4], rest[5])))
        elif directive == "sphere":
            nums = [int(v) for v in rest[:4]]
            brushes.append(list(_sphere(nums[:3], nums[3], rest[4])))
        elif directive == "cone":
            nums = [int(v) for v in rest[:5]]
            brushes.append(list(_cone(nums[:3], nums[3], nums[4], rest[5])))
        else:
            raise SystemExit("line %d: unknown directive %r" % (number, directive))
    if name is None or kill_y is None or not spawns:
        raise SystemExit("map is missing a name, a kill_y or a spawn")
    return MapSpec(name, kill_y, spawns, brushes)


# events/smash/src/map.rs: `HUB` then `ARENAS`, which is the order
# `terrain.rs` assigns regions in.
MAP_ORDER = ["hub", "skylands", "mushroom_islands", "glacier", "desert"]


def load_maps():
    """The hub and every arena, keyed by the region they are stamped into."""
    return {
        index: parse_map((MAPS / ("%s.map" % stem)).read_text())
        for index, stem in enumerate(MAP_ORDER)
    }


# --- block state ids --------------------------------------------------------

BLOCK_STATES = ROOT / "crates/hyperion-minecraft-proto/src/block_state.rs"


def load_block_runs():
    """Every block's run of state ids, from the generated table.

    A block state on the wire is one number out of 32366, and the numbering is
    arbitrary. The table is generated from the same Mojang data the server's
    ids come from, which is why it is read rather than restated.
    """
    source = BLOCK_STATES.read_text()
    pattern = re.compile(
        r'name: "([^"]+)",\s*base_id: (\d+),\s*default_id: (\d+),\s*state_count: (\d+),'
    )
    runs = []
    for name, base_id, _default, count in pattern.findall(source):
        base_id = int(base_id)
        runs.append((base_id, base_id + int(count), name))
    if not runs:
        raise SystemExit("no blocks found in %s" % BLOCK_STATES)
    runs.sort()
    return runs


class BlockNames:
    def __init__(self):
        self.runs = load_block_runs()
        self.starts = [run[0] for run in self.runs]

    def name(self, state_id):
        index = bisect.bisect_right(self.starts, state_id) - 1
        if index < 0:
            return "<state %d>" % state_id
        start, end, name = self.runs[index]
        if start <= state_id < end:
            return name
        return "<state %d>" % state_id


# --- chunk decoding ---------------------------------------------------------
#
# Transcribed from crates/hyperion-minecraft-proto/src/world/chunk.rs and
# .../world/palette.rs. Two things about 26.2 that most write-ups get wrong and
# that a decoder silently desynchronises on:
#
#   * a section carries `nonEmptyBlockCount` *and* `fluidCount`, two shorts;
#   * a paletted container's storage has no length prefix. The reader works the
#     long count out from the bit width and the entry count.


def storage_len(bits, count):
    """`SimpleBitStorage`: values never straddle a long, so this is not
    `count * bits / 64`."""
    if bits == 0:
        return 0
    per_long = 64 // bits
    return -(-count // per_long)


class Reader:
    def __init__(self, buf, offset=0):
        self.buf = buf
        self.offset = offset

    def var_int(self):
        value, used = take_var_int(self.buf, self.offset)
        self.offset = used
        return value

    def u8(self):
        value = self.buf[self.offset]
        self.offset += 1
        return value

    def var_long(self):
        out = 0
        for index in range(10):
            byte = self.u8()
            out |= (byte & 0x7F) << (index * 7)
            if not byte & 0x80:
                return out
        raise SystemExit("var long too long")

    def i16(self):
        value = struct.unpack_from(">h", self.buf, self.offset)[0]
        self.offset += 2
        return value

    def i32(self):
        value = struct.unpack_from(">i", self.buf, self.offset)[0]
        self.offset += 4
        return value

    def u64s(self, count):
        if count == 0:
            return ()
        values = struct.unpack_from(">%dQ" % count, self.buf, self.offset)
        self.offset += count * 8
        return values


def decode_container(reader, entry_count, global_above):
    """One paletted container: a palette and its bit-packed indices.

    `global_above` is the bit width past which the palette is dropped and the
    storage holds registry ids directly: 8 for block states, 3 for biomes.
    Which palette a container uses is not on the wire; the reader picks it from
    the bit width using the same table the writer did.
    """
    bits = reader.u8()
    if bits == 0:
        palette = [reader.var_int()]
    elif bits > global_above:
        palette = None
    else:
        palette = [reader.var_int() for _ in range(reader.var_int())]
    storage = reader.u64s(storage_len(bits, entry_count))
    return bits, palette, storage


def container_value(bits, palette, storage, index):
    if bits == 0:
        return palette[0]
    per_long = 64 // bits
    word = storage[index // per_long]
    shift = (index % per_long) * bits
    raw = (word >> shift) & ((1 << bits) - 1)
    return raw if palette is None else palette[raw]


def decode_chunk(payload, keep_y):
    """Block state ids from one `level_chunk_with_light`.

    `keep_y` is a `(low, high)` band; blocks outside it are decoded and thrown
    away. Every section has to be read whether or not it is wanted, because the
    blob is a concatenation with no per-section offset, but a smash map occupies
    forty of the overworld's three hundred and eighty-four levels and keeping
    the rest would be most of a million entries per chunk for nothing.
    """
    reader = Reader(payload)
    chunk_x = reader.i32()
    chunk_z = reader.i32()

    for _ in range(reader.var_int()):
        reader.var_int()
        reader.u64s(reader.var_int())

    blob_len = reader.var_int()
    blob = Reader(payload, reader.offset)
    end = reader.offset + blob_len

    low, high = keep_y
    blocks = {}
    for section in range(SECTION_COUNT):
        blob.i16()
        blob.i16()
        bits, palette, storage = decode_container(blob, 4096, 8)
        decode_container(blob, 64, 3)

        base_y = WORLD_MIN_Y + section * 16
        if base_y + 15 < low or base_y > high:
            continue
        for y in range(16):
            world_y = base_y + y
            if not low <= world_y <= high:
                continue
            for z in range(16):
                # `Strategy.getIndex`: ((y << 4 | z) << 4) | x.
                row = ((y << 4) | z) << 4
                for x in range(16):
                    state = container_value(bits, palette, storage, row | x)
                    if state == 0:
                        continue
                    blocks[(chunk_x * 16 + x, world_y, chunk_z * 16 + z)] = state

    if blob.offset != end:
        raise SystemExit(
            "section blob desynchronised: read %d of %d bytes"
            % (blob.offset - (end - blob_len), blob_len)
        )
    return (chunk_x, chunk_z), blocks


# `SectionPos.asLong`: x in the top 22 bits, z in the next 22, y in the low 20.
SECTION_HORIZONTAL_BITS = 22
SECTION_Y_BITS = 64 - 2 * SECTION_HORIZONTAL_BITS


def _sign_extend(value, bits):
    value &= (1 << bits) - 1
    if value >> (bits - 1):
        value -= 1 << bits
    return value


def decode_section_blocks_update(payload):
    """`section_blocks_update`, which is how a map's blocks actually arrive.

    This is the packet the whole check turned on and the reason a decoder that
    reads only `level_chunk_with_light` sees an empty world. `terrain.rs` starts
    from `Blocks::empty` and stamps the maps in afterwards, so every column was
    already encoded as air by the time a block was placed in it. The cached
    encoding is never rebuilt; instead `sync_chunks.rs` sends a joining player
    the empty base chunk and then `original_delta_packets`, every change the
    column has seen since it loaded. So the platform arrives as seventy-six
    thousand block changes layered on a world of air, and a client that ignores
    them stands in a void.
    """
    reader = Reader(payload)
    packed = struct.unpack_from(">q", payload, 0)[0]
    reader.offset = 8
    section_x = _sign_extend(packed >> (SECTION_Y_BITS + SECTION_HORIZONTAL_BITS), SECTION_HORIZONTAL_BITS)
    section_z = _sign_extend(packed >> SECTION_Y_BITS, SECTION_HORIZONTAL_BITS)
    section_y = _sign_extend(packed, SECTION_Y_BITS)

    changes = []
    for _ in range(reader.var_int()):
        bits = reader.var_long()
        # `Block.getId(state) << 12 | position`, and the position inside the
        # section packs as x << 8 | z << 4 | y. Note the order: the chunk
        # section container indexes y << 8 | z << 4 | x, the other way round.
        position = bits & 0xFFF
        state = bits >> 12
        x = (position >> 8) & 0xF
        z = (position >> 4) & 0xF
        y = position & 0xF
        changes.append(
            ((section_x * 16 + x, section_y * 16 + y, section_z * 16 + z), state)
        )
    return (section_x, section_z), changes


# --- the client -------------------------------------------------------------


def stamp(started):
    return "%7.2fs" % (time.time() - started)


class MapClient(base.Client):
    """One scripted player, with the chunks it was sent."""

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, lambda line: None)
        self.started = started
        self.log = self._log
        self.buffer = b""
        self.position = (0.0, 65.0, 0.0)
        self.path = []
        self.on_ground = True
        self.health = None
        self.alive = True
        self.chunks = {}
        self.blocks = {}
        self.teleports = []
        self.corrections = 0
        self.last_position_sent = 0.0

    def _log(self, line):
        print("%s [%-3s] %s" % (stamp(self.started), self.name, line), flush=True)

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

    def step_towards(self):
        """Re-assert where we are, the way a real client does every tick.

        Without it hyperion's mirrored position goes stale and the arena's
        bounds check keeps reading wherever we last claimed to be. Steps are
        small because hyperion treats a step it cannot account for as a cheat
        and teleports the player back, which is also the only way a descent
        reads as falling rather than as a correction.
        """
        now = time.time()
        if now - self.last_position_sent < POSITION_INTERVAL:
            return
        self.last_position_sent = now

        if self.path:
            x, y, z = self.position
            tx, ty, tz = self.path[0]
            dx, dy, dz = tx - x, ty - y, tz - z
            distance = (dx * dx + dy * dy + dz * dz) ** 0.5
            if distance <= 3.0:
                self.position = self.path.pop(0)
            else:
                scale = 3.0 / distance
                self.position = (x + dx * scale, y + dy * scale, z + dz * scale)

        x, y, z = self.position
        self.send(
            C2S_MOVE_PLAYER_POS,
            struct.pack(">dddb", x, y, z, ON_GROUND if self.on_ground else 0),
        )


# --- walking off the edge ---------------------------------------------------

# The height to leave over an island on the way out. Every map's tallest block
# is at y 77, and hyperion refuses a step that ends inside a block, so a route
# that clips the far side of an island reads as a cheat and gets teleported
# back rather than falling.
ESCAPE_Y = 100.0

# Blocks from a region's centre to open air. The widest thing any committed map
# puts down reaches a radius of about 52, so 70 clears all of them.
OFF_MAP_RADIUS = 70.0

# Far enough out to clear the tree whose leaves hang over two of the main
# island's opening spawn points.
RIM_HOP = 6.0

# Blocks per position packet on the way down: 20 a second, well under
# Minecraft's terminal velocity and slow enough to bracket the kill plane to
# within a couple of blocks.
DESCEND_STEP = 2.0

# How far below the kill plane to keep descending before levelling off.
DESCEND_MARGIN = 40.0

# Seconds to keep claiming a position below the plane before calling it not
# lethal. A fall is not instantaneous from the server's point of view: the
# arena's death check only runs while the lobby is in `Preparing` or `Playing`,
# and the scatter that puts a player on an arena happens at the top of a nine
# second `Preparing`, so a client that dives immediately can outrun the phase
# it needs. Holding is also simply what falling looks like.
HOLD_SECONDS = 60.0

# Where to hover before dropping through, and for how long.
#
# Diving straight past the plane and dying proves only that something below is
# lethal; it does not rule out the plane being higher than the map says, because
# a dive from y 100 to y -20 takes five seconds and the server has no chance to
# object on the way. So the descent stops just above the declared plane and sits
# there, in open air well off the edge of the island, long enough for the match
# to reach `Playing` and for the death check to have run over this player many
# times. Surviving that and then dying a few blocks lower is the two-sided
# claim: the plane is not above the number, and it is not missing below it.
ABOVE_PLANE_MARGIN = 5.0
ABOVE_PLANE_HOVER = 15.0


class Descent:
    """Walk a player off the edge and find the height the server kills at.

    This is the only check in the repository that reads the kill plane as
    anything but a number. `kill_y` has always been a field a map file sets and
    `is_out_of_bounds` compares against, and nothing has ever confirmed that
    the height it names is below the terrain, above the terrain, or anywhere a
    falling player would ever reach. A descent from well above the islands to
    well below the plane answers all three at once: the player must survive
    every block down to the plane, and must die within a few of crossing it.
    """

    def __init__(self, client, spec, region, check):
        self.client = client
        self.spec = spec
        self.check = check
        self.origin_x = region * REGION_STRIDE

        x, y, z = client.position
        dx, dz = x - self.origin_x, z
        length = math.hypot(dx, dz)
        if length < 1.0:
            ux, uz = 1.0, 0.0
        else:
            ux, uz = dx / length, dz / length

        # Out over the rim, up over the tallest block, then out to open air.
        # Straight up from a spawn would clip the leaves above two of them.
        client.on_ground = False
        client.path = [
            (x + ux * RIM_HOP, y, z + uz * RIM_HOP),
            (x + ux * RIM_HOP, ESCAPE_Y, z + uz * RIM_HOP),
            (self.origin_x + ux * OFF_MAP_RADIUS, ESCAPE_Y, uz * OFF_MAP_RADIUS),
        ]
        check.log(
            "%s heads off the edge of %s, out to radius %.0f at y %.0f"
            % (client.name, spec.name, OFF_MAP_RADIUS, ESCAPE_Y)
        )

        self.stage = "out"
        self.teleports_seen = len(client.teleports)
        self.alive_floor = ESCAPE_Y
        self.deaths_above_the_plane = []
        self.hold_until = None
        self.announced_hold = False
        self.hover_until = None
        self.hovered_for = 0.0
        self.world_spawns = [
            (spawn[0] + self.origin_x, spawn[1], spawn[2]) for spawn in spec.spawns
        ]

    def _descend_to(self, target, stage):
        x, y, z = self.client.position
        steps = max(int((y - target) / DESCEND_STEP), 1)
        self.client.path = [
            (x, y - DESCEND_STEP * step, z) for step in range(1, steps + 1)
        ] + [(x, target, z)]
        self.stage = stage

    def _start_descending(self):
        ledge = self.spec.kill_y + ABOVE_PLANE_MARGIN
        self._descend_to(ledge, "to the ledge")
        self.check.log(
            "%s falls from y %.0f to y %.0f, %.0f blocks above %s's kill plane y=%g"
            % (
                self.client.name,
                self.client.position[1],
                ledge,
                ABOVE_PLANE_MARGIN,
                self.spec.name,
                self.spec.kill_y,
            )
        )

    def _respawned(self):
        """A teleport onto one of this map's spawn points.

        Not merely "a teleport upwards". hyperion answers a position it will not
        account for with a teleport back to where the player was last tick, and
        those corrections arrive constantly while a client is walking. Reading
        any upward teleport as a respawn turns the first correction into a
        reported death at y 100, which is exactly the false positive this check
        would otherwise be famous for. `lives.rs` puts a respawning player on
        `Arena::spawn`, so the signal is landing on a spawn point and nothing
        else.
        """
        for teleport in self.client.teleports[self.teleports_seen :]:
            for spawn in self.world_spawns:
                if (
                    abs(teleport[0] - spawn[0]) < 1.5
                    and abs(teleport[1] - spawn[1]) < 1.5
                    and abs(teleport[2] - spawn[2]) < 1.5
                ):
                    return True
        self.teleports_seen = len(self.client.teleports)
        return False

    def tick(self):
        """One step. True once the run has an answer."""
        client = self.client

        if self.stage == "out":
            if not client.path:
                self._start_descending()
            return False

        y = client.position[1]

        if self.stage == "to the ledge":
            if self._respawned() or (client.health is not None and client.health <= 0.0):
                raise SystemExit(
                    "%s died at y %.1f on the way down, which is above %s's declared "
                    "kill plane y=%g: the game kills higher than the map says"
                    % (client.name, y, self.spec.name, self.spec.kill_y)
                )
            if client.path:
                return False
            if self.hover_until is None:
                self.hover_until = time.time() + ABOVE_PLANE_HOVER
                self.check.log(
                    "%s hovers at y %.1f, %.0f blocks above the plane, for %.0fs"
                    % (client.name, y, ABOVE_PLANE_MARGIN, ABOVE_PLANE_HOVER)
                )
            if time.time() < self.hover_until:
                return False
            self.hovered_for = ABOVE_PLANE_HOVER
            self._descend_to(self.spec.kill_y - DESCEND_MARGIN, "down")
            self.check.log(
                "%s survived %.0fs above the plane; now dropping through it"
                % (client.name, ABOVE_PLANE_HOVER)
            )
            return False

        if self._respawned():
            return self._finish(y)

        if y > self.spec.kill_y:
            # Still above the plane. A death here would be the bug this check
            # exists to catch, and it is recorded rather than asserted so the
            # report can say how far above.
            if client.health is not None and client.health <= 0.0:
                self.deaths_above_the_plane.append(y)
        self.alive_floor = min(self.alive_floor, y)

        if client.path:
            return False

        # Out of waypoints: hold here, well under the plane, and keep saying so.
        if self.hold_until is None:
            self.hold_until = time.time() + HOLD_SECONDS
        if not self.announced_hold:
            self.announced_hold = True
            self.check.log(
                "%s holds at y %.1f, %.1f blocks below %s's kill plane, waiting to die"
                % (client.name, y, self.spec.kill_y - y, self.spec.name)
            )
        if time.time() < self.hold_until:
            return False
        raise SystemExit(
            "%s claimed to be at y %.1f, which is %.1f blocks below %s's kill plane "
            "y=%g, for %.0f seconds and was never killed: the map declares a death "
            "plane the game does not enforce"
            % (
                client.name,
                y,
                self.spec.kill_y - y,
                self.spec.name,
                self.spec.kill_y,
                HOLD_SECONDS,
            )
        )

    def _finish(self, y):
        plane = self.spec.kill_y
        if self.deaths_above_the_plane:
            highest = max(self.deaths_above_the_plane)
            raise SystemExit(
                "%s died at y %.1f, which is %.1f blocks ABOVE %s's kill plane y=%g: "
                "the game kills higher than the map says"
                % (self.client.name, highest, highest - plane, self.spec.name, plane)
            )
        if self.alive_floor > plane:
            raise SystemExit(
                "%s was killed at y %.1f without ever crossing %s's kill plane y=%g"
                % (self.client.name, self.alive_floor, self.spec.name, plane)
            )
        self.check.prove(
            "the kill plane is where the map says",
            "%s fell from y %.0f to y %g, %.0f blocks above %s's declared kill plane "
            "y=%g, and hovered there in open air for %.0fs without dying; it then "
            "dropped through the plane, reached y %.1f and was killed and respawned. "
            "No death was reported at any height above the plane"
            % (
                self.client.name,
                ESCAPE_Y,
                plane + ABOVE_PLANE_MARGIN,
                ABOVE_PLANE_MARGIN,
                self.spec.name,
                plane,
                self.hovered_for,
                self.alive_floor,
            ),
        )
        return True


# --- the run ----------------------------------------------------------------


class Check:
    """The claims this run makes, and the evidence for each."""

    ORDER = [
        "the map file's blocks are in the world",
        "a player stands on the geometry",
        "the arena's blocks are in the world",
        "the kill plane is where the map says",
    ]

    def __init__(self, started):
        self.started = started
        self.proof = {claim: None for claim in self.ORDER}

    def log(self, line):
        print("%s %-5s %s" % (stamp(self.started), "", line), flush=True)

    def prove(self, claim, evidence):
        if self.proof[claim] is None:
            self.proof[claim] = evidence
            self.log("PROVED %s: %s" % (claim, evidence))

    def report(self):
        print("")
        print("=" * 78)
        unproved = [claim for claim, evidence in self.proof.items() if evidence is None]
        for claim in self.ORDER:
            evidence = self.proof[claim]
            print("  %-4s %s" % ("ok" if evidence else "MISS", claim))
            if evidence:
                print("       %s" % evidence)
        print("=" * 78)
        return 1 if unproved else 0


# The share of a map's chunk columns that has to have arrived before its
# geometry is compared.
COVERAGE = 0.95


def occupied_columns(maps):
    """The chunk columns the maps put blocks in, in world coordinates.

    Everything else is skipped without decoding its sections. hyperion sends
    thousands of columns of pure air around a player and decoding them costs
    tens of millions of Python iterations, which is enough to stall this
    client's own position packets and get it teleported back as a cheat. The
    maps together occupy about two hundred columns.
    """
    wanted = set()
    for region, spec in maps.items():
        origin_x = region * REGION_STRIDE
        for x, _y, z in spec.blocks:
            wanted.add(((x + origin_x) >> 4, z >> 4))
    return wanted


def region_of(x):
    """Which map region a world x lies in. `terrain.rs` spaces them by stride."""
    return int(round(x / REGION_STRIDE))


def compare_geometry(spec, region, client, names, check, claim):
    """Every block the map places, in every chunk this client was sent.

    Positives and negatives both, because a server that filled the world with
    stone would pass a check that only asked whether the platform blocks are
    present. The negatives are the air between the islands, which is the part
    of a Super Smash Mobs map that does the killing.
    """
    origin_x = region * REGION_STRIDE

    # Wait until nearly the whole map has arrived. Comparing the moment the
    # first column lands would pass on a fraction of the geometry and read, in
    # the report, exactly like comparing all of it.
    columns = {((at[0] + origin_x) >> 4, at[2] >> 4) for at in spec.blocks}
    arrived = columns & set(client.chunks)
    if len(arrived) < len(columns) * COVERAGE:
        return False

    low = min(at[1] for at in spec.blocks)
    high = max(at[1] for at in spec.blocks)

    matched = 0
    mismatched = []
    for (x, y, z), block in spec.blocks.items():
        world = (x + origin_x, y, z)
        if (world[0] >> 4, world[2] >> 4) not in client.chunks:
            continue
        state = client.blocks.get(world, 0)
        got = names.name(state)
        if got == block:
            matched += 1
        elif len(mismatched) < 5:
            mismatched.append((world, block, got))

    if mismatched:
        for world, want, got in mismatched:
            check.log("MISMATCH at %s: map says %s, server sent %s" % (world, want, got))
        raise SystemExit(
            "%s: %d blocks disagree with %s"
            % (spec.name, len(mismatched), spec.name)
        )
    if matched == 0:
        return False

    # The air. Every position inside the map's own bounding box that the file
    # leaves empty has to arrive empty.
    empty = 0
    filled = []
    xs = [at[0] for at in spec.blocks]
    zs = [at[2] for at in spec.blocks]
    for x in range(min(xs), max(xs) + 1, 3):
        for z in range(min(zs), max(zs) + 1, 3):
            for y in range(low, high + 1, 3):
                if (x, y, z) in spec.blocks:
                    continue
                world = (x + origin_x, y, z)
                if (world[0] >> 4, world[2] >> 4) not in client.chunks:
                    continue
                if client.blocks.get(world, 0) == 0:
                    empty += 1
                elif len(filled) < 5:
                    filled.append((world, names.name(client.blocks[world])))
    if filled:
        for world, got in filled:
            check.log("MISMATCH at %s: map says air, server sent %s" % (world, got))
        raise SystemExit("%s: %d gaps are not empty" % (spec.name, len(filled)))

    check.prove(
        claim,
        "%s: %d of the %d blocks the map file places arrived, across %d of its %d "
        "chunk columns, every one of them with the name the file gives it; and %d "
        "sampled gaps between the islands arrived as air"
        % (spec.name, matched, len(spec.blocks), len(arrived), len(columns), empty),
    )
    return True


# --- the schedule -----------------------------------------------------------


def solid_under(client, names, at):
    """The block hyperion reads to decide a player is standing.

    `ceil(y) - 1`, which is the check `terrain.rs::stand_on_something` makes
    against its own `Blocks` and this makes against the wire instead.
    """
    below = (int(math.floor(at[0])), int(math.ceil(at[1])) - 1, int(math.floor(at[2])))
    state = client.blocks.get(below, 0)
    return below, state, names.name(state)


def run(args):
    started = time.time()
    check = Check(started)
    names = BlockNames()
    maps = load_maps()

    band_low = min(min(at[1] for at in spec.blocks) for spec in maps.values())
    band_high = max(max(at[1] for at in spec.blocks) for spec in maps.values())
    check.log(
        "maps: %s"
        % ", ".join("%d=%s" % (index, spec.name) for index, spec in sorted(maps.items()))
    )
    wanted = occupied_columns(maps)
    check.log(
        "decoding y %d to y %d in the %d chunk columns the maps occupy"
        % (band_low, band_high, len(wanted))
    )

    clients = []
    for index in range(args.clients):
        client = MapClient(args.host, args.port, "P%d" % (index + 1), started)
        client.handshake(args.host, args.port, 2)
        client.login()
        client.configuration()
        client.enter_play()
        clients.append(client)
        client.log("configuration acknowledged")

    subject = clients[0]
    phase = "hub"
    phase_since = time.time()
    arena_region = None
    descent = None
    deadline = time.time() + args.timeout

    def handle(client, packet_id, payload):
        nonlocal phase, phase_since
        if packet_id == S2C_LOGIN:
            client.entity_id = struct.unpack(">i", payload[:4])[0]
            client.joined = True
        elif packet_id == S2C_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            client.send(C2S_ACCEPT_TELEPORTATION, var_int(teleport_id))
            client.teleports.append((x, y, z))
            moved = (
                abs(x - client.position[0]) > 1.0
                or abs(y - client.position[1]) > 1.0
                or abs(z - client.position[2]) > 1.0
            )
            client.position = (x, y, z)
            if moved:
                client.log("<- teleported to (%.1f, %.1f, %.1f)" % (x, y, z))
        elif packet_id == S2C_KEEP_ALIVE:
            client.send(C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == S2C_SET_HEALTH:
            health = struct.unpack(">f", payload[:4])[0]
            if client.health != health:
                client.log("<- health %.2f/20" % health)
            client.health = health
        elif packet_id == S2C_LEVEL_CHUNK_WITH_LIGHT:
            where = struct.unpack_from(">ii", payload, 0)
            if where not in wanted:
                return
            _, blocks = decode_chunk(payload, (band_low, band_high))
            client.chunks[where] = client.chunks.get(where, 0) + len(blocks)
            client.blocks.update(blocks)
        elif packet_id == S2C_SECTION_BLOCKS_UPDATE:
            where, changes = decode_section_blocks_update(payload)
            if where not in wanted:
                return
            client.chunks.setdefault(where, 0)
            for at, state in changes:
                if not band_low <= at[1] <= band_high:
                    continue
                if state == 0:
                    client.blocks.pop(at, None)
                else:
                    client.blocks[at] = state
                    client.chunks[where] += 1
        elif packet_id == S2C_DISCONNECT:
            client.alive = False
            raise SystemExit("%s was disconnected" % client.name)

    while time.time() < deadline:
        for client in clients:
            for packet_id, payload in client.drain():
                handle(client, packet_id, payload)
            client.step_towards()

        region = region_of(subject.position[0])

        if phase == "hub":
            if region == 0:
                if compare_geometry(
                    maps[0], 0, subject, names, check,
                    "the map file's blocks are in the world",
                ):
                    below, state, block = solid_under(subject, names, subject.position)
                    if state == 0:
                        raise SystemExit(
                            "%s stands at %s with air under them at %s"
                            % (subject.name, subject.position, below)
                        )
                    check.prove(
                        "a player stands on the geometry",
                        "%s claimed to be on the ground at (%.1f, %.1f, %.1f) for %.0fs "
                        "and was never corrected; the block under them at %s is %s, "
                        "which the map file puts there"
                        % (
                            subject.name,
                            subject.position[0],
                            subject.position[1],
                            subject.position[2],
                            time.time() - phase_since,
                            below,
                            block,
                        ),
                    )
                    phase = "waiting for the match"
                    phase_since = time.time()
                    check.log("waiting for the scatter onto an arena")

        elif phase == "waiting for the match":
            if region >= 1:
                phase = "arena"
                phase_since = time.time()
                # Fixed here rather than re-read each pass: once the descent
                # starts the subject is a long way off the island and a later
                # correction would be read against the wrong map.
                arena_region = region
                check.log(
                    "%s was scattered into region %d (%s)"
                    % (subject.name, region, maps[region].name)
                )

        elif phase == "arena":
            spec = maps[arena_region]
            if compare_geometry(
                spec, arena_region, subject, names, check,
                "the arena's blocks are in the world",
            ):
                below, state, block = solid_under(subject, names, subject.position)
                check.log(
                    "%s stands on %s at %s, %.0f blocks above %s's kill plane y=%g"
                    % (subject.name, block, below, subject.position[1] - spec.kill_y,
                       spec.name, spec.kill_y)
                )
                descent = Descent(subject, spec, arena_region, check)
                phase = "descending"
                phase_since = time.time()

        elif phase == "descending":
            if descent.tick():
                return check.report()

        time.sleep(0.005)

    check.log("timed out in phase %r" % phase)
    return check.report()


# --- entry point ------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    # Enough to fill the lobby, so `countdown_for` runs its shortest countdown
    # rather than its longest. Eight covers every `full_players` this server
    # has shipped; the comment used to say eight *was* `full_players`, which
    # was `LobbyConfig::default` restated here and went stale at #1019.
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--timeout", type=float, default=240.0)
    args = parser.parse_args()
    sys.exit(run(args))


if __name__ == "__main__":
    main()
