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
  * an arrow fired into the ground stops in it, rather than falling through

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

    def draw(seconds, note, yaw=35.0, pitch=-20.0, settle=1.5):
        client.spawned.clear()
        client.syncs.clear()
        client.motions.clear()
        client.aim(yaw, pitch)
        client.send_position()
        client.use_slot(bow_slot, "(nock) " + note)
        pump(client, seconds)
        client.release_slot(bow_slot, "(release) " + note)
        pump(client, settle)
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

    # --- the arrow stops in the ground it was fired into ---
    #
    # Every other claim in this file is about a shot into open sky, which is
    # what the arrow scenarios in `docs/differential-testing.md` are too: they
    # prove the flight and say nothing about what it hits. This is the one that
    # exercises the terrain seam against real loaded chunks, and it is here
    # rather than in a Rust test because no Rust test can. `tests/
    # projectile_blocks.rs` drives a `Cubes` fixture; the host half --
    # `HyperionBlocks::sweep` reaching into hyperion's block store for the
    # arena's actual blocks -- has no mock, so a bug in it passes every unit
    # test in the crate. That is the shape of the `Flying` mirror bug recorded
    # in the repo's CLAUDE.md, and this is the assertion that would have caught
    # this feature's version of it.
    #
    # Straight down, so the geometry needs nothing from the map: the shooter is
    # above the arena floor, so a block is under the muzzle whatever the map is
    # and wherever on it they stand.
    #
    # From up in the air, and that is not a detail. Standing on the floor there
    # is one eye height of clearance, and `smash::draw_projectiles` only
    # decorates the projectile after `smash::fly` has already integrated it in
    # the same phase -- so the AddEntity the client is told about lands half a
    # block into a flight that is over in two ticks, and every velocity
    # broadcast after it is a tail of zeros. That is ENG-12082: the drop
    # measured `-0.00` blocks and the check called it a PASS. Climbing first
    # buys about ten blocks of fall, which is twenty ticks of observable travel
    # after the arrow exists on the wire.
    #
    # hyperion takes the client's own position, so the climb is just position
    # packets; there is nothing to ask permission for. It is checked below
    # rather than assumed, because a server that did rubber-band would leave
    # every assertion after it measuring the old vacuous shot again.
    ground = client.position[1]
    for step in range(1, 11):
        client.position = (client.position[0], ground + step, client.position[2])
        client.on_ground = False
        client.send_position()
        pump(client, 0.1)
    climbed = client.position[1] - ground
    print("climbed %.2f blocks above the floor (y %.2f -> %.2f)"
          % (climbed, ground, client.position[1]), flush=True)
    check(
        climbed > 8.0,
        "the shooter can get above the floor, so the downward shot has room to "
        "be seen flying (climbed %.2f blocks)" % climbed,
    )

    down = draw(0.4, "straight down", pitch=90.0, settle=2.0)
    check(len(down) >= 1, "a downward draw fires (got %d arrows)" % len(down))
    if down:
        launched = down[0]
        down_id = launched["id"]
        down_velocities = client.motions.get(down_id, [])

        # The launch, asserted before anything about the stop. Without this the
        # checks below pass just as loudly for an arrow that never moved:
        # "fell 0.00 blocks" and "every broadcast at rest" are what a projectile
        # fired at zero speed looks like too, and a gate that cannot tell the
        # feature working from the feature never firing is not evidence of
        # either. The AddEntity motion is the launch as the wire carried it, one
        # packet before any collision could have touched it.
        launch_vy = launched["motion"][1]
        print("downward launch: speed %.3f, vy %.3f"
              % (launched["speed"], launch_vy), flush=True)
        check(
            launch_vy < -0.2,
            "the downward arrow launched downwards at speed (vy %.3f blocks a tick)"
            % launch_vy,
        )
        check(
            len(down_velocities) >= 2,
            "the downward arrow is broadcast while it flies (got %d SetEntityMotion)"
            % len(down_velocities),
        )

        # The shape that says "it flew, then it stopped", and the one the old
        # check could not see: a non-zero broadcast strictly before the first
        # zero. `len(stopped) >= 1` on its own is satisfied *most* loudly by an
        # arrow that never moved on the wire -- eighteen of eighteen ticks at
        # rest was its best possible score.
        moving = [
            i for i, v in enumerate(down_velocities) if v != (0.0, 0.0, 0.0)
        ]
        first_stop = next(
            (i for i, v in enumerate(down_velocities) if v == (0.0, 0.0, 0.0)),
            None,
        )
        print("downward stream: %d broadcasts, %d moving, first zero at %s"
              % (len(down_velocities), len(moving), first_stop), flush=True)
        check(
            first_stop is not None and first_stop >= 1,
            "the downward arrow was seen flying and then seen stopping "
            "(%d broadcasts, %d of them moving, first zero at index %s)"
            % (len(down_velocities), len(moving), first_stop),
        )

        # Integrate the velocity stream, exactly as the open-sky check above
        # does. There is no absolute position on the wire for an arrow -- the
        # gate asserts that too, a few checks up -- so the drop is the sum of
        # the per-tick velocities, and the client dead-reckons it the same way.
        drop = -sum(vy for _, vy, _ in down_velocities)
        # The muzzle sits `climbed + EYE_HEIGHT` above the floor and the client
        # only hears about the arrow part way down, so the drop it can see is at
        # most that and usually less. An unstopped arrow keeps accelerating
        # under smash's gravity for the whole window this samples, and before
        # this feature it fell until the projectile's timer expired, straight
        # through the map. The bound is the clearance plus a block of slack for
        # where in the block it lands.
        ceiling = climbed + EYE_HEIGHT + 1.0
        print("downward arrow: fell %.2f blocks over %d ticks (clearance %.2f)"
              % (drop, len(down_velocities), climbed + EYE_HEIGHT), flush=True)
        check(
            drop < ceiling,
            "an arrow fired straight down stops in the floor rather than falling "
            "through it (fell %.2f blocks against %.2f of clearance; it launched "
            "at %.2f a tick)" % (drop, ceiling, -launch_vy),
        )

        # And the stop is a stop, not merely a slow one. `smash::fly` zeroes the
        # flight on impact and `advance_drawn_projectiles` puts that on the wire,
        # so a zero-velocity broadcast is the impact as a client sees it.
        stopped = [v for v in down_velocities if v == (0.0, 0.0, 0.0)]
        check(
            len(stopped) >= 1,
            "the impact reaches the client as a zero-velocity broadcast (got %d of %d "
            "ticks at rest)" % (len(stopped), len(down_velocities)),
        )

    # Back on the floor for whatever comes next.
    client.position = (client.position[0], ground, client.position[2])
    client.on_ground = True
    client.send_position()
    pump(client, 0.5)

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
