#!/usr/bin/env python3
"""The smash bow, read off the wire by the player who drew it.

Smash's arrows are ability projectiles, not the vanilla bow bedwars grants: the
Skeleton kit's Barrage is `item: minecraft:bow`, held to charge and released to
fire. This gate is the smash-side mirror of `bow-check.py`: a real 776 client
joins, takes the Skeleton kit, draws the bow for a known number of ticks, lets
go, and reads the arrow it fired straight off the packets it was sent.

Every claim below is one a client can settle for itself:

  * releasing a drawn bow spawns at least one `minecraft:arrow` in an AddEntity
  * that arrow spawns at the shooter's EYE, not their feet
  * its launch heading is `look_angles(velocity)`, not the raw look yaw
  * the server broadcasts the arrow's position every tick as it flies
  * a longer draw launches faster than a shorter one (the charge curve)

Exits non-zero on the first untrue claim, after printing what it saw.
"""

import argparse
import importlib.util
import math
import pathlib
import time

TOOLS = pathlib.Path(__file__).resolve().parent


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


match = _load("smash_match", "smash-match.py")
monitor = _load("packet_monitor", "packet_monitor.py")
bowcheck = _load("bow_check", "bow-check.py")

Archer = bowcheck.Archer
look_angles = bowcheck.look_angles
angle_delta = bowcheck.angle_delta
entity_type_id = bowcheck.entity_type_id
pump = bowcheck.pump
wait_until = bowcheck.wait_until

# Vanilla eye height and the half-block muzzle offset bedwars fires from. The
# number the test expects, not imported, so the Rust cannot move the oracle.
EYE_HEIGHT = 1.62
MUZZLE_OFFSET = 0.5

# getPowerForTime(full) * 3.0 == 3.0 blocks a tick; the curve saturates at 1.0.
MAX_ARROW_SPEED = 3.0


def look_direction(yaw, pitch):
    """get_direction_from_rotation: x=-cos(p)sin(y), y=-sin(p), z=cos(p)cos(y)."""
    yr = math.radians(yaw)
    pr = math.radians(pitch)
    return (
        -math.cos(pr) * math.sin(yr),
        -math.sin(pr),
        math.cos(pr) * math.cos(yr),
    )


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

    client.on_ground = True
    client.aim(35.0, -20.0)
    client.send_position()
    pump(client, 0.3)

    client.command("kit skeleton")
    wait_until(
        client,
        lambda: client.hotbar_slot_of("minecraft:bow") is not None,
        10.0,
        "a bow on the bar from /kit skeleton",
    )
    bow_slot = client.hotbar_slot_of("minecraft:bow")
    check(
        bow_slot is not None,
        "/kit skeleton puts a minecraft:bow on a visible key (key %s)" % bow_slot,
    )
    if bow_slot is None:
        print("RESULT: failure (no bow to draw)", flush=True)
        return 1

    def arrows_from(spawned):
        return [e for e in spawned if e["type"] == arrow_type]

    def draw(seconds, note):
        client.spawned.clear()
        client.syncs.clear()
        client.motions.clear()
        client.aim(35.0, -20.0)
        client.send_position()
        client.use_slot(bow_slot, "(nock) " + note)
        pump(client, seconds)
        client.release_slot(bow_slot, "(release) " + note)
        pump(client, 1.5)
        return arrows_from(client.spawned)

    full = draw(2.6, "full draw")
    check(
        len(full) >= 1,
        "a full draw spawns at least one minecraft:arrow (got %d)" % len(full),
    )
    if not full:
        print("RESULT: failure (nothing was fired)", flush=True)
        return 1

    launch = full[0]
    spawn = launch["position"]
    motion = launch["motion"]
    full_speed = launch["speed"]
    feet = client.position

    eye = (feet[0], feet[1] + EYE_HEIGHT, feet[2])
    # The spawn packet arrives an unknown number of ticks into the flight (the
    # exact count varies with server startup timing), so reconstructing the
    # launch point tick-by-tick is fragile. Instead assert the geometry that is
    # invariant to it: the arrow's trajectory, extrapolated backward along its
    # launch velocity, must pass through the shooter's EYE. Measure the
    # perpendicular distance from the eye to that line. Eye-origin gives ~0; a
    # feet-origin leaves the line a full eye height (~1.6) above the feet.
    def perp_distance(point):
        mlen = math.dist((0.0, 0.0, 0.0), motion)
        d = tuple(m / mlen for m in motion) if mlen > 1e-6 else (0.0, 0.0, 1.0)
        w = tuple(point[i] - spawn[i] for i in range(3))
        proj = sum(w[i] * d[i] for i in range(3))
        return math.dist((0.0, 0.0, 0.0), tuple(w[i] - proj * d[i] for i in range(3)))

    eye_line_dist = perp_distance(eye)
    feet_line_dist = perp_distance(feet)
    print(
        "spawn=(%.2f,%.2f,%.2f) eye_off_line=%.3f feet_off_line=%.3f"
        % (spawn + (eye_line_dist, feet_line_dist)),
        flush=True,
    )
    check(
        eye_line_dist < 0.4,
        "the arrow's back-extrapolated trajectory passes through the shooter's "
        "eye, not the feet (%.2f off the eye line; a feet-origin is ~1.6, here "
        "%.2f)" % (eye_line_dist, feet_line_dist),
    )

    exp_yaw, exp_pitch = look_angles(motion)
    wire_yaw = launch["wire_yaw"]
    wire_pitch = launch["wire_pitch"]
    tol = 2.0 * 360.0 / 256.0
    check(
        angle_delta(wire_yaw, exp_yaw) < tol and angle_delta(wire_pitch, exp_pitch) < tol,
        "the arrow's wire heading is look_angles(velocity) = (%.1f, %.1f) "
        "(got (%.1f, %.1f))" % (exp_yaw, exp_pitch, wire_yaw, wire_pitch),
    )
    check(
        angle_delta(wire_yaw, 35.0) > 10.0,
        "the wire yaw is not the shooter's raw look yaw of 35 (got %.1f)" % wire_yaw,
    )

    arrow_id = launch["id"]
    velocities = client.motions.get(arrow_id, [])
    syncs = client.syncs.get(arrow_id, [])
    print(
        "flight: %d SetEntityMotion (per-tick velocity) packets, %d absolute "
        "EntityPositionSync, for arrow %d" % (len(velocities), len(syncs), arrow_id),
        flush=True,
    )
    # Smoothness (the vanilla wire pattern): the arrow is driven by per-tick
    # velocity, which the client predicts smooth position from -- NOT by per-tick
    # absolute position teleports. An arrow has no client interpolation handler,
    # so an absolute EntityPositionSync hard-snaps it ~3 blocks a tick = jagged.
    check(
        len(velocities) >= 12,
        "the server sends the arrow's velocity every tick (SetEntityMotion), the "
        "vanilla representation the client predicts smooth motion from (got %d)"
        % len(velocities),
    )
    check(
        len(syncs) == 0,
        "the arrow is never hard-teleported by a per-tick absolute "
        "EntityPositionSync (got %d; any per-tick absolute sync IS the jagged "
        "~3-block-a-tick snap this fixes)" % len(syncs),
    )
    # A real flight: integrate the per-tick velocity stream from the launch and
    # confirm it travels a real distance and shows gravity in vy.
    if len(velocities) >= 2:
        px, py, pz = spawn
        for vx, vy, vz in velocities:
            px += vx
            py += vy
            pz += vz
        moved = math.dist((px, py, pz), spawn)
        vy_first, vy_last = velocities[0][1], velocities[-1][1]
        print(
            "velocity arc: integrated %.1f blocks; vy %.3f -> %.3f"
            % (moved, vy_first, vy_last),
            flush=True,
        )
        check(
            moved > 6.0,
            "the velocity stream integrates to a real flight (%.1f blocks)" % moved,
        )
        check(
            vy_last < vy_first,
            "gravity shows in the per-tick velocity (vy %.3f -> %.3f)"
            % (vy_first, vy_last),
        )

    pump(client, 0.6)
    short = draw(0.4, "short draw")
    check(len(short) >= 1, "a short draw still fires (got %d arrows)" % len(short))
    if short:
        short_speed = short[0]["speed"]
        print("full_speed=%.3f short_speed=%.3f" % (full_speed, short_speed), flush=True)
        check(
            abs(full_speed - MAX_ARROW_SPEED) < 0.1,
            "a full draw launches at vanilla's %.1f blocks a tick (got %.3f)"
            % (MAX_ARROW_SPEED, full_speed),
        )
        check(
            short_speed < full_speed - 0.15,
            "a shorter draw launches slower than a full one (%.3f < %.3f)"
            % (short_speed, full_speed),
        )

    print(
        "RESULT: %s (%d checks failed)"
        % ("ok" if not failures else "failure", len(failures)),
        flush=True,
    )
    for failure in failures:
        import sys
        print("  failed: %s" % failure, file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    import sys
    sys.exit(main())
