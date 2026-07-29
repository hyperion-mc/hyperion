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
base = match.base

take_var_int = base.take_var_int
var_int = base.var_int

S2C_ADD_ENTITY = 0x01

# `PlayerInventory::HOTBAR_START_SLOT`: `ClientboundContainerSetSlot` numbers
# the whole inventory, and the nine keys a player can see begin here.
HOTBAR_START_SLOT = match.HOTBAR_START_SLOT

# `BowItem.releaseUsing` passes `f * 3.0` to `shootFromRotation`. Kept here as
# the number the *test* expects, deliberately not imported from anywhere, so
# that changing the constant in Rust cannot also change what proves it.
MAX_ARROW_SPEED = 3.0

# `LastFireTime::can_fire`, in seconds.
FIRE_COOLDOWN = 0.150


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
        """`ClientboundAddEntityPacket#STREAM_CODEC`, field for field.

        Since 26.2 this packet carries the entity's velocity itself, through
        the same packed codec `set_entity_motion` uses, which is the whole
        reason a launch speed is checkable from a client at all.
        """
        entity_id, offset = take_var_int(payload)
        offset += 16  # uuid
        type_id, offset = take_var_int(payload, offset)
        x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
        offset += 24
        motion, offset = match.take_lp_vec3(payload, offset)
        speed = (motion[0] ** 2 + motion[1] ** 2 + motion[2] ** 2) ** 0.5
        # Since 26.2 the facing rides right after the velocity: a signed byte
        # of pitch then one of yaw, each 1/256 of a turn (`Mth.packDegrees`).
        # This is the client-visible heading the rotation gate below reads.
        (x_rot, y_rot) = struct.unpack(">bb", payload[offset : offset + 2])
        offset += 2
        wire_pitch = x_rot * 360.0 / 256.0
        wire_yaw = y_rot * 360.0 / 256.0
        entry = {
            "id": entity_id,
            "type": type_id,
            "position": (x, y, z),
            "motion": motion,
            "speed": speed,
            "wire_yaw": wire_yaw,
            "wire_pitch": wire_pitch,
        }
        self.spawned.append(entry)
        self.log(
            "<- AddEntity id=%d type=%d at (%.2f, %.2f, %.2f) motion=(%.3f, "
            "%.3f, %.3f) |v|=%.3f yaw=%.1f pitch=%.1f"
            % ((entity_id, type_id, x, y, z) + motion + (speed, wire_yaw, wire_pitch))
        )

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
    client.release_slot(bow_slot, "(first release)")
    # Deliberately inside `LastFireTime::can_fire`'s 150 ms window.
    time.sleep(FIRE_COOLDOWN / 3.0)
    client.release_slot(bow_slot, "(second release, inside the cooldown)")
    pump(client, 1.0)
    burst = arrows_seen()
    check(
        len(burst) == 1,
        "two releases %d ms apart fire once, not twice (got %d arrows)"
        % (int(FIRE_COOLDOWN / 3.0 * 1000), len(burst)),
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
