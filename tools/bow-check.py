#!/usr/bin/env python3
"""What a bow does, read off the wire by the client that fired it.

The bow had a working charge, a working cooldown and a working arrow, and no
gate that would notice if any of them stopped. Everything below is a claim a
client can check for itself, because everything below arrives as a packet.

  * `/bow` puts a real `minecraft:bow` and real `minecraft:arrow`s on the bar,
    which is what proves the item the command grants and the `ItemKind::Bow`
    the module compares against are the same thing
  * releasing a drawn bow spawns an entity of type `minecraft:arrow` and the
    shooter is told about it, in a `ClientboundAddEntity`
  * that packet carries the launch velocity, so the charge curve is readable
    directly rather than inferred from where the arrow lands
  * a full draw launches at exactly 3.0 blocks a tick
  * a longer draw launches faster than a shorter one
  * an arrow that hits a block stops
  * a second release inside the 150 ms cooldown launches nothing
  * firing spends one arrow

What it does not pin, deliberately: the absolute speed of a *partial* draw.
An earlier version checked one against vanilla's curve and it was dropped,
because at a quarter draw the quadratic and the linear bug it was meant to
catch are 0.2 blocks a tick apart while the script's own scheduling moves the
answer by more than that. It passed with the bug present, which is the
definition of a check that is not one. The saturated draw is the discriminating
assertion and it is exact: `min(f, 1)` means any hold past a second is 1.0,
so the expected speed is 3.0 with no dependence on timing at all.

Two of the three bugs this shipped alongside are not covered here either. A
self-hit needs an arrow that comes back to its shooter, and `ArrowsInEntity`
is entity metadata that only another client can see, so both want a second
connection this gate does not open.

The velocity assertion is the one this file was written for. `get_charge`
returned *seconds* clamped to 1.2 and the caller multiplied by 3.0, so a fully
drawn bow fired at 3.6 blocks a tick -- a fifth faster than vanilla can -- under
a comment claiming 3.0 was the maximum. Nothing caught it because nothing looked
at the number. A full draw is the right thing to assert on because vanilla's
curve saturates: `min(f, 1)` means any hold past one second gives exactly 1.0,
so the expected speed is 3.0 with no dependence on how precisely this script
sleeps.

Exits non-zero on the first thing that is not true, after printing what it saw.
"""

import argparse
import importlib.util
import math
import pathlib
import struct
import sys
import time

TOOLS = pathlib.Path(__file__).resolve().parent


def _load(name, filename):
    """Import a sibling tool whose file name is not a Python identifier."""
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


match = _load("smash_match", "smash-match.py")
monitor = _load("packet_monitor", "packet_monitor.py")
base = match.base

take_var_int = base.take_var_int
var_int = base.var_int

S2C_ADD_ENTITY = 0x01
# `ClientboundEntityPositionSyncPacket`, id 35 in protocol 776. This is the
# absolute per-tick position the server broadcasts for a flying arrow; without
# it a client renders the spawn and nothing after.
S2C_ENTITY_POSITION_SYNC = 0x23

# `PlayerInventory::HOTBAR_START_SLOT`: `ClientboundContainerSetSlot` numbers
# the whole inventory, and the nine keys a player can see begin here.
HOTBAR_START_SLOT = match.HOTBAR_START_SLOT

# `BowItem.releaseUsing` passes `f * 3.0` to `shootFromRotation`. Kept here as
# the number the *test* expects, deliberately not imported from anywhere, so
# that changing the constant in Rust cannot also change what proves it.
MAX_ARROW_SPEED = 3.0



def look_angles(motion):
    """Vanilla's projectile facing for a velocity, the server's own `look_angles`.

    A projectile stores `yaw = atan2(dx, dz)` and `pitch = atan2(dy, horizontal)`
    (`AbstractArrow.tick`), which is the sign-flip of the look convention a
    shooter's own yaw uses. Kept here rather than imported, so changing the Rust
    cannot also change what proves it.
    """
    mx, my, mz = motion
    horizontal = math.hypot(mx, mz)
    yaw = math.degrees(math.atan2(mx, mz))
    pitch = math.degrees(math.atan2(my, horizontal))
    return yaw, pitch


def angle_delta(a, b):
    """The shorter arc between two angles in degrees, so 179 and -179 are two
    apart rather than 358."""
    d = (a - b) % 360.0
    return min(d, 360.0 - d)


def entity_type_id(name):
    """The network id of `name` in `minecraft:entity_type`.

    `base.registry_entries` reads `protocol.json`, where the ids are the
    indices. This file used to scrape the generated Rust instead, which cost it
    a wrong answer on its very first run: the regex also matched the registry's
    own `name` field, so every id came out one too high and the gate looked for
    an arrow of type 7 while the server sent type 6.
    """
    entries = base.registry_entries("minecraft:entity_type")
    if name not in entries:
        raise SystemExit(
            "%s is not in minecraft:entity_type, which has %d entries, so this "
            "gate cannot tell an arrow from any other entity."
            % (name, len(entries))
        )
    return entries.index(name)


class Archer(match.MatchClient):
    """A scripted player that remembers every entity it was told about."""

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, started)
        # Every AddEntity this client received, as dicts.
        self.spawned = []
        self.slots = {}
        # entity id -> list of absolute positions from EntityPositionSync,
        # in arrival order (one per server tick while it flies).
        self.syncs = {}

    def absorb(self, packet_id, payload):
        if packet_id == match.S2C_LOGIN:
            self.entity_id = struct.unpack(">i", payload[:4])[0]
            self.joined = True
            self.log("** in the world ** entity_id=%d" % self.entity_id)
        elif packet_id == match.S2C_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            self.position = (x, y, z)
            self.send(match.C2S_ACCEPT_TELEPORTATION, var_int(teleport_id))
            self.log("<- teleported to (%.1f, %.1f, %.1f)" % (x, y, z))
        elif packet_id == match.S2C_KEEP_ALIVE:
            self.send(match.C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == S2C_ADD_ENTITY:
            self.absorb_add_entity(payload)
        elif packet_id == S2C_ENTITY_POSITION_SYNC:
            self.absorb_position_sync(payload)
        elif packet_id == match.S2C_CONTAINER_SET_SLOT:
            self.absorb_slot(payload)
        elif packet_id == match.S2C_SYSTEM_CHAT:
            text, _ = match.take_nbt_string(payload, 0)
            self.log("<- chat: %s" % text)
        elif packet_id == match.S2C_DISCONNECT:
            text, _ = match.take_nbt_string(payload, 0)
            self.log("<- DISCONNECTED: %s" % text)
            self.alive = False

    def absorb_add_entity(self, payload):
        """Decode one AddEntity with the shared `packet_monitor` decoder.

        The launch velocity (the charge curve) and the two rotation bytes (the
        client-visible heading) both come from that one decoder, so this gate
        and the skin gates read the packet the same way. `wire_yaw`/`wire_pitch`
        are its `yaw`/`pitch`, named for what the heading assertions below ask.
        """
        entity = monitor.decode_add_entity(payload)
        motion = entity["motion"]
        speed = (motion[0] ** 2 + motion[1] ** 2 + motion[2] ** 2) ** 0.5
        entry = {
            "id": entity["id"],
            "type": entity["type"],
            "position": entity["position"],
            "motion": motion,
            "speed": speed,
            "wire_yaw": entity["yaw"],
            "wire_pitch": entity["pitch"],
        }
        self.spawned.append(entry)
        self.log(
            "<- AddEntity id=%d type=%d at (%.2f, %.2f, %.2f) motion=(%.3f, "
            "%.3f, %.3f) |v|=%.3f yaw=%.1f pitch=%.1f"
            % (
                (entry["id"], entry["type"])
                + entry["position"]
                + motion
                + (speed, entry["wire_yaw"], entry["wire_pitch"])
            )
        )

    def absorb_position_sync(self, payload):
        """`ClientboundEntityPositionSyncPacket`: id, then absolute position,
        the velocity a client predicts between syncs from, and rotation. The
        whole point of this gate is that the server sends one of these per tick
        as the arrow flies. Both the position and the velocity are kept: the
        arc check anchors on the first sync's own state, so a spawn-time quirk
        (a re-add a block away) cannot skew it."""
        entity_id, offset = take_var_int(payload)
        x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
        offset += 24
        vx, vy, vz = struct.unpack(">ddd", payload[offset : offset + 24])
        self.syncs.setdefault(entity_id, []).append(((x, y, z), (vx, vy, vz)))

    def absorb_slot(self, payload):
        _container, offset = take_var_int(payload)
        _state, offset = take_var_int(payload, offset)
        (slot,) = struct.unpack(">h", payload[offset : offset + 2])
        offset += 2
        count, offset = take_var_int(payload, offset)
        if count <= 0:
            self.slots.pop(slot, None)
            return
        item_id, offset = take_var_int(payload, offset)
        name = ITEMS[item_id] if item_id < len(ITEMS) else "<%d>" % item_id
        self.slots[slot] = (name, count)
        self.log("<- slot %d now holds %d x %s" % (slot, count, name))

    def hotbar_slot_of(self, item):
        """The visible key holding `item`, or None."""
        for slot, (name, _count) in sorted(self.slots.items()):
            key = slot - HOTBAR_START_SLOT
            if name == item and 0 <= key < 9:
                return key
        return None

    def count_of(self, item):
        return sum(count for name, count in self.slots.values() if name == item)


ITEMS = match.load_item_names()


def pump(client, seconds):
    """Read the socket for a while, keeping the connection alive."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if not client.alive:
            return
        for packet_id, payload in client.drain():
            client.absorb(packet_id, payload)
        if client.joined:
            client.repeat_position()
        time.sleep(0.01)


def wait_until(client, predicate, seconds, what):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        pump(client, 0.05)
        if predicate():
            return True
    print("TIMEOUT waiting for %s" % what, file=sys.stderr)
    return False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument("--name", default="Archer")
    args = parser.parse_args()

    started = time.time()
    failures = []

    def check(ok, message):
        print("%s %s" % ("PASS" if ok else "FAIL", message), flush=True)
        if not ok:
            failures.append(message)

    arrow_type = entity_type_id("minecraft:arrow")
    print("minecraft:arrow is entity type %d" % arrow_type, flush=True)

    client = Archer(args.host, args.port, args.name, started)
    client.handshake(args.host, args.port, 2)
    client.login()
    client.configuration()
    client.enter_play()

    if not wait_until(client, lambda: client.joined, 30.0, "the world"):
        return 1
    pump(client, 1.0)

    # --- the bow itself ------------------------------------------------

    client.command("bow")
    wait_until(
        client,
        lambda: client.hotbar_slot_of("minecraft:bow") is not None,
        10.0,
        "a bow on the bar",
    )
    bow_slot = client.hotbar_slot_of("minecraft:bow")
    arrows = client.count_of("minecraft:arrow")
    check(
        bow_slot is not None,
        "/bow puts a minecraft:bow on a visible key (key %s)" % bow_slot,
    )
    check(
        arrows > 0,
        "/bow puts minecraft:arrow in the inventory (%d of them)" % arrows,
    )
    if bow_slot is None:
        print("RESULT: failure (no bow to draw)", flush=True)
        return 1

    def arrows_seen():
        """Every distinct arrow entity, as (launch, latest).

        One arrow shows up in more than one `AddEntity`: the launch, and then
        another when `arrow_block_hit` pins it into whatever it ran into. They
        share an entity id, so the id is what counts an arrow and the order
        gives the before and after.
        """
        out = {}
        for entry in client.spawned:
            if entry["type"] != arrow_type:
                continue
            if entry["id"] in out:
                out[entry["id"]][1] = entry
            else:
                out[entry["id"]] = [entry, entry]
        return list(out.values())

    def draw(seconds, note):
        """Nock, hold for `seconds`, release. Returns the arrows that flew."""
        client.spawned.clear()
        client.use_slot(bow_slot, "(nock)")
        pump(client, seconds)
        client.release_slot(bow_slot, note)
        pump(client, 1.0)
        return arrows_seen()

    # --- a full draw ---------------------------------------------------

    before = client.count_of("minecraft:arrow")
    full = draw(1.5, "(full draw)")
    check(
        len(full) == 1,
        "a full draw spawns exactly one minecraft:arrow the client is told "
        "about (got %d)" % len(full),
    )
    if not full:
        print("RESULT: failure (nothing was fired)", flush=True)
        return 1

    launch, latest = full[0]
    speed = launch["speed"]
    check(
        abs(speed - MAX_ARROW_SPEED) < 0.05,
        "a full draw launches at vanilla's %.1f blocks a tick, not the 3.6 the "
        "old seconds-as-charge produced (got %.3f)" % (MAX_ARROW_SPEED, speed),
    )

    # `arrow_block_hit` zeroes the velocity and pins the arrow at the collision
    # point, and the client is told again. Free to assert here because the
    # world bedwars loads has something to hit in every direction.
    check(
        latest is not launch and latest["speed"] == 0.0,
        "an arrow that hits a block stops: it was re-sent at (%.2f, %.2f, "
        "%.2f) with |v|=%.3f"
        % (latest["position"] + (latest["speed"],)),
    )

    pump(client, 0.5)
    after = client.count_of("minecraft:arrow")
    check(
        after == before - 1,
        "firing spends exactly one arrow (%d -> %d)" % (before, after),
    )

    # --- a shorter draw ------------------------------------------------

    pump(client, 0.5)
    short = draw(0.25, "(quarter draw)")
    check(
        len(short) == 1,
        "a short draw still fires (got %d arrows)" % len(short),
    )
    if short:
        short_speed = short[0][0]["speed"]
        check(
            short_speed < speed,
            "a shorter draw launches slower than a full one (%.3f < %.3f)"
            % (short_speed, speed),
        )

    # --- the cooldown --------------------------------------------------

    pump(client, 0.5)
    client.spawned.clear()
    client.use_slot(bow_slot, "(nock, first of two)")
    pump(client, 1.2)
    # Both releases before the next socket read, so the server sees them
    # within a tick of each other, far inside `LastFireTime::can_fire`'s 150 ms
    # window. An earlier version slept 50 ms between them -- still inside the
    # window, but a loaded server could stall the two releases more than 150 ms
    # apart and fire both, a wall-clock race that failed the gate under load.
    # Back to back asserts the same thing without the race.
    client.release_slot(bow_slot, "(first release)")
    client.release_slot(bow_slot, "(second release, inside the cooldown)")
    pump(client, 1.0)
    burst = arrows_seen()
    check(
        len(burst) == 1,
        "two releases inside the 150 ms cooldown fire once, not twice "
        "(got %d arrows)" % len(burst),
    )

    # --- the heading on the wire --------------------------------------
    #
    # The arc was always right; what a bystander saw was not. A projectile
    # stores yaw = atan2(dx, dz), pitch = atan2(dy, horizontal) -- the
    # sign-flip of the shooter's own look -- and hyperion used to send the
    # shooter's own yaw instead, so every arrow rendered mirrored across its
    # line of flight. Fire off a coordinate axis so the sign shows, and read
    # the two rotation bytes straight out of the AddEntity. Nothing here is
    # inferred: this is the number the client turns the arrow model by.
    pump(client, 0.5)
    client.aim(35.0, -20.0)
    client.send_position()
    pump(client, 0.3)
    angled = draw(1.5, "(full draw, aimed yaw 35 pitch -20)")
    check(
        len(angled) == 1,
        "an off-axis full draw fires exactly one arrow (got %d)" % len(angled),
    )
    if angled:
        launch = angled[0][0]
        exp_yaw, exp_pitch = look_angles(launch["motion"])
        wire_yaw = launch["wire_yaw"]
        wire_pitch = launch["wire_pitch"]
        # One angle byte is 360/256 = 1.40625 degrees; allow two steps for the
        # quantisation plus the f32 the server aims in.
        tol = 2.0 * 360.0 / 256.0
        check(
            angle_delta(wire_yaw, exp_yaw) < tol,
            "the arrow's wire yaw is atan2(dx, dz) = %.1f (got %.1f)"
            % (exp_yaw, wire_yaw),
        )
        check(
            angle_delta(wire_pitch, exp_pitch) < tol,
            "the arrow's wire pitch is atan2(dy, horizontal) = %.1f (got %.1f)"
            % (exp_pitch, wire_pitch),
        )
        # And it is the projectile convention, not the shooter's look: the old
        # bug sent the player's own +35 where vanilla sends its sign-flip. This
        # is the assertion that fails loudly if the mirrored heading returns.
        check(
            angle_delta(wire_yaw, 35.0) > 10.0,
            "the wire yaw is not the shooter's look yaw of 35 (the mirrored-"
            "heading bug); got %.1f" % wire_yaw,
        )

    # --- the arrow actually flies, on the wire ------------------------
    #
    # Everything above reads the launch. This is the part the operator cares
    # about: after the spawn, does the server keep telling a client where the
    # arrow is, and does it travel the vanilla arc? Fire steeply up into open
    # sky so nothing stops it, then follow the EntityPositionSync stream for the
    # arrow and hold it against the arc integrated forward from its own launch
    # velocity (pos += v; v *= 0.99; v.y -= 0.05, AbstractArrow.tick). This
    # fails if the arrow does not move, moves at the wrong speed, never gets a
    # position update, or does not fall.
    client.spawned.clear()
    client.syncs.clear()
    client.aim(0.0, -80.0)
    client.send_position()
    pump(client, 0.3)
    flew = draw(1.5, "(full draw, aimed up)")
    pump(client, 1.2)
    check(
        len(flew) == 1,
        "the upward full draw fires one arrow (got %d)" % len(flew),
    )
    if flew:
        launch = flew[0][0]
        arrow_id = launch["id"]
        spawn = launch["position"]
        vx, vy, vz = launch["motion"]
        pairs = client.syncs.get(arrow_id, [])
        samples = [pos for pos, _vel in pairs]

        check(
            len(samples) >= 20,
            "the server broadcasts the arrow's position every tick as it flies "
            "(got %d EntityPositionSync packets; 0 means the client is never "
            "told the arrow moved)" % len(samples),
        )
        if len(samples) < 2:
            print("RESULT: failure (no flight to trace)", flush=True)
            return 1

        # Vanilla's own integration, forward from the arrow's own first synced
        # state: pos += v; v *= 0.99; v.y -= 0.05 (AbstractArrow.tick). Anchoring
        # on the first sync rather than the spawn keeps a one-block spawn re-add
        # out of the measurement; what is under test is that every later synced
        # position lies on the arc this start implies.
        px, py, pz = samples[0]
        v = list(pairs[0][1])
        arc = [(px, py, pz)]
        for _ in range(120):
            px += v[0]
            py += v[1]
            pz += v[2]
            v[0] *= 0.99
            v[1] *= 0.99
            v[2] *= 0.99
            v[1] -= 0.05
            arc.append((px, py, pz))

        def dist_to_arc(point):
            """Least distance from `point` to the piecewise-linear vanilla arc,
            and how far along it (segment index) that nearest point sits.

            Matching to the nearest point on the curve rather than to a fixed
            tick makes this robust to a dropped or coalesced position packet: a
            correct arrow lies *on* the arc whatever the packet cadence, so this
            measures the shape, not the timing."""
            best_d = float("inf")
            best_k = 0
            px0, py0, pz0 = point
            for k in range(len(arc) - 1):
                ax, ay, az = arc[k]
                bx, by, bz = arc[k + 1]
                dx, dy, dz = bx - ax, by - ay, bz - az
                length2 = dx * dx + dy * dy + dz * dz
                if length2 == 0.0:
                    t = 0.0
                else:
                    t = ((px0 - ax) * dx + (py0 - ay) * dy + (pz0 - az) * dz) / length2
                    t = max(0.0, min(1.0, t))
                cx, cy, cz = ax + t * dx, ay + t * dy, az + t * dz
                d = ((px0 - cx) ** 2 + (py0 - cy) ** 2 + (pz0 - cz) ** 2) ** 0.5
                if d < best_d:
                    best_d, best_k = d, k
            return best_d, best_k

        # Loose on purpose: the wire velocity is LP-quantised and the server
        # integrates in f32, so this is a bound, not a fit. It is far tighter
        # than a frozen arrow, a wrong launch speed, or a missing gravity term,
        # each of which walks the wire trajectory off the arc by whole blocks.
        tol = 1.0
        worst = 0.0
        reached = 0
        for wx, wy, wz in samples:
            d, k = dist_to_arc((wx, wy, wz))
            worst = max(worst, d)
            reached = max(reached, k)
        # A couple of the first spawn-adjacent samples can predate the first
        # integration tick; the shape over the whole flight is the claim.
        print(
            "arc match: %d samples, worst off-arc %.3f blocks, reached tick %d"
            % (len(samples), worst, reached),
            flush=True,
        )
        check(
            worst < tol,
            "the arrow flies the vanilla arc on the wire, not some other path "
            "(worst off-arc distance %.3f blocks over %d samples, tolerance "
            "%.2f)" % (worst, len(samples), tol),
        )
        check(
            reached >= 12,
            "the arrow flies a real stretch of the arc, not just the first "
            "tick or two (nearest arc point reached tick %d)" % reached,
        )

        # And it went somewhere: the last sync is a long way from the spawn.
        if samples:
            lx, ly, lz = samples[-1]
            moved = ((lx - spawn[0]) ** 2 + (ly - spawn[1]) ** 2 + (lz - spawn[2]) ** 2) ** 0.5
            check(
                moved > 8.0,
                "the arrow travelled a real distance (%.1f blocks from spawn "
                "after %d ticks)" % (moved, len(samples)),
            )

    print(
        "RESULT: %s (%d checks failed)"
        % ("ok" if not failures else "failure", len(failures)),
        flush=True,
    )
    for failure in failures:
        print("  failed: %s" % failure, file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
