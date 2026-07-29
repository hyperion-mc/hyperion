#!/usr/bin/env python3
"""The daylight cycle is frozen, read off the wire by the client that joined.

26.2 stopped putting the day time in a field a server sets every tick. The sun
is now driven by a per-world clock the client advances *itself*: the server
sends the clock's `rate` once in a `ClientboundSetTimePacket` and the client
interpolates from there. hyperion never ticked that clock, so it sent no
`SetTime` at all, and a client with no clock state free-runs its own daylight
cycle -- the sun drifts across a sky the operator wanted static.

The fix sends the overworld clock once on join with `rate` 0.0, which is the
exact wire form a paused clock takes (`ServerClockManager.ClockInstance.
packNetworkState`): the client holds the day time and never advances it, with
no per-tick resend.

What this gate proves, all of it off the wire:

  * a `SetTime` (id 113) arrives during the join sequence at all. This is the
    fail-then-pass assertion: an unpatched server sends nothing here, so the
    gate fails without the change and passes with it.
  * it carries the overworld clock (network id 0 in `minecraft:world_clock`).
  * that clock's `rate` is exactly 0.0 -- the freeze form. A non-zero rate is
    a clock the client would advance, which is the bug.
  * its `total_ticks` is the fixed day time the server was configured with
    (noon, 6000, by default), not a drifting value.
  * `game_time` is the world age, a separate field, so anything that reads game
    time still gets a real value while only the day time is frozen.
  * over several seconds -- many server ticks -- no further `SetTime` moves the
    day time. With `rate` 0.0 the client holds the sun on its own; the server
    does not (and must not) resend time every tick to keep it there.

Exits non-zero on the first thing that is not true, after printing what it saw.
"""

import argparse
import importlib.util
import pathlib
import struct
import sys
import time

TOOLS = pathlib.Path(__file__).resolve().parent


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


match = _load("smash_match", "smash-match.py")
base = match.base

take_var_int = base.take_var_int
var_int = base.var_int

# `ClientboundSetTimePacket`, id 113 (0x71) in protocol 776. Named in
# tools/client-26.2.py's id table and
# crates/hyperion-minecraft-proto/src/generated/packet_id.rs.
S2C_SET_TIME = 0x71

# The overworld clock's network id in `minecraft:world_clock`. The registry the
# server sends during configuration lists `minecraft:overworld` first, so it is
# 0. Kept here as the number the *test* expects rather than imported, so a
# change to the Rust cannot also change what proves it.
OVERWORLD_CLOCK_ID = 0

# The default frozen day time hyperion sends: noon. `WorldTime::default()` in
# crates/hyperion/src/simulation/world_time.rs. Held here as the test's own
# expectation.
EXPECTED_DAY_TIME = 6000


def take_var_long(payload, offset=0):
    """Decode one Minecraft VarLong, returning (value, new_offset).

    Same continuation-bit encoding as a VarInt, but up to ten bytes and a
    signed 64-bit result. `take_var_int` tops out at 32 bits, so `total_ticks`
    -- a `VarLong` on the wire -- needs its own reader.
    """
    result = 0
    for i in range(10):
        byte = payload[offset]
        offset += 1
        result |= (byte & 0x7F) << (7 * i)
        if not byte & 0x80:
            break
    else:
        raise ValueError("VarLong ran past ten bytes")
    if result >= (1 << 63):
        result -= 1 << 64
    return result, offset


def decode_set_time(payload):
    """Decode a `ClientboundSetTimePacket` into its game time and clock map.

    Layout (protocol 776): a big-endian `long` game time, a `VarInt` count, then
    each entry a `VarInt` clock id and a `ClockNetworkState` of a `VarLong`
    `total_ticks`, an `f32` `partial_tick` and an `f32` `rate`.
    """
    game_time = struct.unpack(">q", payload[:8])[0]
    offset = 8
    count, offset = take_var_int(payload, offset)
    clocks = {}
    for _ in range(count):
        clock_id, offset = take_var_int(payload, offset)
        total_ticks, offset = take_var_long(payload, offset)
        partial_tick, rate = struct.unpack(">ff", payload[offset : offset + 8])
        offset += 8
        clocks[clock_id] = {
            "total_ticks": total_ticks,
            "partial_tick": partial_tick,
            "rate": rate,
        }
    return {"game_time": game_time, "clocks": clocks}


class Watcher(match.MatchClient):
    """A scripted player that records every SetTime it is sent."""

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, started)
        self.entity_id = None
        self.joined = False
        # Every SetTime received, decoded, in arrival order.
        self.set_times = []

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
        elif packet_id == S2C_SET_TIME:
            decoded = decode_set_time(payload)
            self.set_times.append(decoded)
            self.log(
                "<- SetTime game_time=%d clocks=%s"
                % (decoded["game_time"], decoded["clocks"])
            )
        elif packet_id == match.S2C_DISCONNECT:
            text, _ = match.take_nbt_string(payload, 0)
            self.log("<- DISCONNECTED: %s" % text)
            self.alive = False


def pump(client, seconds):
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
    parser.add_argument("--name", default="Horologist")
    args = parser.parse_args()

    started = time.time()
    failures = []

    def check(ok, message):
        print("%s %s" % ("PASS" if ok else "FAIL", message), flush=True)
        if not ok:
            failures.append(message)

    client = Watcher(args.host, args.port, args.name, started)
    client.handshake(args.host, args.port, 2)
    client.login()
    client.configuration()
    client.enter_play()

    if not wait_until(client, lambda: client.joined, 30.0, "the world"):
        print("RESULT: failure (never joined)", flush=True)
        return 1

    # The freeze SetTime is sent in the join sequence, right after the join
    # itself. Give it a moment to arrive, then a few seconds more to prove
    # nothing re-sends time.
    wait_until(
        client,
        lambda: len(client.set_times) >= 1,
        10.0,
        "a SetTime packet",
    )

    check(
        len(client.set_times) >= 1,
        "a SetTime (id 113) arrives on join -- an unpatched server sends none "
        "(got %d)" % len(client.set_times),
    )
    if not client.set_times:
        print("RESULT: failure (no SetTime, the sun would free-run)", flush=True)
        return 1

    first = client.set_times[0]
    clocks = first["clocks"]

    check(
        OVERWORLD_CLOCK_ID in clocks,
        "the SetTime carries the overworld clock (network id %d); it has %s"
        % (OVERWORLD_CLOCK_ID, sorted(clocks)),
    )
    overworld = clocks.get(OVERWORLD_CLOCK_ID)
    if overworld is None:
        print("RESULT: failure (no overworld clock in SetTime)", flush=True)
        return 1

    check(
        overworld["rate"] == 0.0,
        "the overworld clock's rate is the freeze value 0.0, not a rate the "
        "client would advance (got %r)" % overworld["rate"],
    )
    check(
        overworld["total_ticks"] == EXPECTED_DAY_TIME,
        "the frozen day time is the configured %d (noon), a fixed value "
        "(got %d)" % (EXPECTED_DAY_TIME, overworld["total_ticks"]),
    )
    check(
        overworld["partial_tick"] == 0.0,
        "the clock is parked on a whole tick (partial_tick 0.0, got %r)"
        % overworld["partial_tick"],
    )
    check(
        first["game_time"] >= 0,
        "game_time is a world age separate from the day time (got %d)"
        % first["game_time"],
    )

    # Prove the day time does not advance. With rate 0.0 the client holds the
    # sun itself, so a correct server sends no further SetTime that moves it.
    # Watch several seconds of ticks and confirm the day time the client would
    # render never changes.
    baseline = overworld["total_ticks"]
    before = len(client.set_times)
    pump(client, 6.0)
    moved = [
        st["clocks"][OVERWORLD_CLOCK_ID]["total_ticks"]
        for st in client.set_times[before:]
        if OVERWORLD_CLOCK_ID in st["clocks"]
        and st["clocks"][OVERWORLD_CLOCK_ID]["total_ticks"] != baseline
    ]
    check(
        not moved,
        "over 6 s of ticks the frozen day time never advances from %d "
        "(saw moves to %s)" % (baseline, moved),
    )
    # And any later SetTime that does arrive must still freeze (rate 0.0),
    # never hand the client a rate to run.
    running = [
        st["clocks"][OVERWORLD_CLOCK_ID]["rate"]
        for st in client.set_times[before:]
        if OVERWORLD_CLOCK_ID in st["clocks"]
        and st["clocks"][OVERWORLD_CLOCK_ID]["rate"] != 0.0
    ]
    check(
        not running,
        "no later SetTime hands the client a non-zero rate (saw %s)" % running,
    )

    if failures:
        print(
            "RESULT: failure (%d checks failed): %s"
            % (len(failures), "; ".join(failures)),
            flush=True,
        )
        return 1
    print("RESULT: success (the sun is frozen at day time %d)" % baseline, flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
