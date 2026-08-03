#!/usr/bin/env python3
"""The tab list's tick rate and ping, read off the wire by the client that joined.

Two features, and one of them can only be proved by a real client.

# The tick rate

`ClientboundTabListPacket` was sent zero times by smash before this: only
bedwars built one, and it built both halves itself, every tick, for every
player. `hyperion::egress::tab_list` now owns the packet and writes the footer
with the rate the tick loop actually managed, against the rate it is paced to:

    TPS 19.8 / 20.0
    3 players online

So the first assertion here is fail-then-pass by construction -- an unpatched
server sends no `TabList` at all -- and the rest read the label back and check
it says something a loop could have produced.

# The ping, and the one thing only a client can settle

`roster.rs` sent `ping: 0` at join and nothing ever again, which drew five full
bars for a measurement nobody had taken: the game server routed a serverbound
`keep_alive` to `Route::Ignore` and never sent a clientbound one.

hyperion now probes with a keep-alive and times the answer. hyperion also puts
a *proxy* between the client and the game server, and this is the one place the
design could quietly lie: if the proxy answered keep-alives itself, the game
server would be timing the proxy, the number would look entirely plausible, and
nothing in a Rust test could tell the difference.

Reading `crates/hyperion-proxy` says it does not -- there is no keep-alive
handling in it at all -- but that is an argument, not a measurement. This is the
measurement, and it is the reason this gate exists rather than a unit test:

  1. join, and read the latency out of the roster: it must be -1, the client's
     own "no reading" sprite, and not the 0 that used to draw five bars.
  2. answer keep-alives for a few seconds. A real latency must arrive in an
     `UPDATE_LATENCY` delta, and on loopback it must be in the top bucket.
  3. **stop answering, and keep the connection otherwise busy.** After the
     server's keep-alive timeout the latency must fall back to -1.

Step 3 is the whole argument. If anything between this script and the game
server were answering keep-alives, the server would go on measuring a healthy
round trip while this client sat mute, and the reading would never go unknown.
It only goes unknown if the thing answering is *this process*.

  4. start answering again, and watch a real reading come back, so what step 3
     proved is a measurement resuming and not a connection that died.

Exits non-zero on the first thing that is not true, after printing what it saw.
"""

import argparse
import importlib.util
import pathlib
import re
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
monitor = _load("packet_monitor", "packet_monitor.py")

take_var_int = match.base.take_var_int
var_int = match.base.var_int
take_nbt_string = match.take_nbt_string
parse_player_info_update = monitor.parse_player_info_update

# crates/hyperion-minecraft-proto/src/generated/packet_id.rs, protocol 776.
S2C_TAB_LIST = 0x7A
S2C_PLAYER_INFO_UPDATE = monitor.S2C_PLAYER_INFO_UPDATE

UPDATE_LATENCY = monitor.UPDATE_LATENCY
ADD_PLAYER = monitor.ADD_PLAYER

# `hyperion::egress::ping::UNKNOWN`, which is the client's own `latency < 0`
# branch in `PlayerTabOverlay.extractPingIcon` and draws `icon/ping_unknown`.
UNKNOWN = -1

# `PlayerTabOverlay.extractPingIcon` again: under 150 ms is the five bar
# sprite. Anything on loopback that is not in this bucket is not a round trip,
# it is a bug.
FIVE_BARS_BELOW = 150

# `hyperion::Global::keep_alive_timeout`. How long a probe goes unanswered
# before the readout gives up on it, plus room for the next probe to be sent,
# answered nowhere, and time out in turn.
KEEP_ALIVE_TIMEOUT = 20.0
MUTE_SECONDS = KEEP_ALIVE_TIMEOUT + 10.0

# `hyperion::egress::tab_list::footer_readout`. Both numbers, because the
# second is what makes the first checkable: a label carrying only "19.8" could
# be anything.
TPS_LABEL = re.compile(r"^TPS (\d+\.\d) / (\d+\.\d)$")
SAMPLING_LABEL = "TPS sampling"
PLAYERS_LABEL = re.compile(r"^(\d+) players? online$")

# `hyperion::TICKS_PER_SECOND`, held here as the test's own expectation rather
# than read from the label being tested.
TARGET_TPS = 20.0


class Watcher(match.MatchClient):
    """A scripted player that can be told to stop answering keep-alives."""

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, started)
        self.entity_id = None
        self.joined = False
        # Whether to answer a keep-alive. Flipping this off is the experiment.
        self.answer_keep_alives = True
        # How many arrived, so "the server stopped probing" and "this client
        # stopped answering" cannot be confused for one another.
        self.keep_alives = 0
        # Every tab list, as the two plain strings a player would read.
        self.tab_lists = []
        # Every latency this client was told about itself, in arrival order,
        # tagged with whether it came in the joining roster or a later delta.
        self.latencies = []

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
            self.keep_alives += 1
            if self.answer_keep_alives:
                self.send(match.C2S_KEEP_ALIVE, payload[:8])
            else:
                self.log("<- keep-alive #%d, deliberately not answered" % self.keep_alives)
        elif packet_id == S2C_TAB_LIST:
            header, offset = take_nbt_string(payload, 0)
            footer, _ = take_nbt_string(payload, offset)
            self.tab_lists.append({"header": header, "footer": footer})
            self.log("<- tab list footer %r" % footer)
        elif packet_id == S2C_PLAYER_INFO_UPDATE:
            actions, entries = parse_player_info_update(payload)
            if not actions & UPDATE_LATENCY:
                return
            for entry in entries:
                self.latencies.append(
                    {
                        "uuid": entry["uuid"],
                        "latency": entry["latency"],
                        "roster": bool(actions & ADD_PLAYER),
                        "at": time.monotonic(),
                    }
                )
                self.log(
                    "<- latency %d ms (%s)"
                    % (entry["latency"], "roster" if actions & ADD_PLAYER else "delta")
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
            # Keeps the connection busy while mute, so a lost reading in step 3
            # is the keep-alive going unanswered and not the whole client
            # having gone quiet.
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


def footer_lines(footer):
    return footer.split("\n")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument("--name", default="Tabby")
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

    # --- the tick rate ----------------------------------------------------
    #
    # A joining client is unicast the current text, so one arrives without
    # waiting for anything to change. The measured number needs a full window
    # first, so allow for the label starting at "sampling".
    wait_until(
        client,
        lambda: any(
            TPS_LABEL.match(footer_lines(t["footer"])[0]) for t in client.tab_lists
        ),
        20.0,
        "a tab list carrying a measured tick rate",
    )

    check(
        bool(client.tab_lists),
        "a TabList (id 122) arrives at all -- this server sent none before "
        "(got %d)" % len(client.tab_lists),
    )
    if not client.tab_lists:
        print("RESULT: failure (no tab list)", flush=True)
        return 1

    measured = [
        (t, TPS_LABEL.match(footer_lines(t["footer"])[0]))
        for t in client.tab_lists
        if TPS_LABEL.match(footer_lines(t["footer"])[0])
    ]
    check(
        bool(measured),
        "the footer's first line carries a measured rate and the rate it is "
        "paced to; the footers seen were %r"
        % [t["footer"] for t in client.tab_lists],
    )
    if not measured:
        print("RESULT: failure (no TPS label)", flush=True)
        return 1

    last, groups = measured[-1]
    rate, target = float(groups.group(1)), float(groups.group(2))
    check(
        target == TARGET_TPS,
        "the label prints the ceiling it is drawn against (%.1f, expected "
        "%.1f)" % (target, TARGET_TPS),
    )
    check(
        0.0 < rate <= target,
        "the measured rate is a rate this loop could have produced: 0 < %.1f "
        "<= %.1f" % (rate, target),
    )
    players = PLAYERS_LABEL.match(footer_lines(last["footer"])[1])
    check(
        players is not None and int(players.group(1)) >= 1,
        "the footer's second line counts the players actually connected "
        "(%r)" % footer_lines(last["footer"])[1],
    )

    # A constant cannot produce this line. If the client got in before the
    # first measurement window closed, the server said so rather than guessing,
    # which is the strongest evidence available here that the number is
    # measured. Not required -- a client that joins later never sees it.
    sampled = any(
        footer_lines(t["footer"])[0] == SAMPLING_LABEL for t in client.tab_lists
    )
    print(
        "NOTE  the first window %s observed as %r"
        % ("was" if sampled else "was not", SAMPLING_LABEL),
        flush=True,
    )

    # --- the ping ---------------------------------------------------------
    roster = [entry for entry in client.latencies if entry["roster"]]
    check(
        bool(roster) and all(entry["latency"] == UNKNOWN for entry in roster),
        "the joining roster carries %d (no reading yet) and not the 0 that "
        "used to draw five full bars (got %r)"
        % (UNKNOWN, [entry["latency"] for entry in roster]),
    )

    wait_until(
        client,
        lambda: any(
            entry["latency"] >= 0 and not entry["roster"] for entry in client.latencies
        ),
        20.0,
        "a measured latency",
    )
    real = [
        entry for entry in client.latencies if entry["latency"] >= 0 and not entry["roster"]
    ]
    check(
        bool(real),
        "a real round trip arrives as an UPDATE_LATENCY delta once keep-alives "
        "are being answered (got %r)" % [e["latency"] for e in client.latencies],
    )
    if not real:
        print("RESULT: failure (no latency was ever measured)", flush=True)
        return 1
    check(
        0 <= real[-1]["latency"] < FIVE_BARS_BELOW,
        "the loopback round trip is in the client's top bucket (%d ms, must "
        "be under %d)" % (real[-1]["latency"], FIVE_BARS_BELOW),
    )
    check(
        client.keep_alives >= 2,
        "the server probes repeatedly rather than once (%d keep-alives)"
        % client.keep_alives,
    )

    # --- who answers keep-alives ------------------------------------------
    #
    # Go mute while staying otherwise busy. Only this process can answer a
    # keep-alive, so only this process going quiet can take the reading away.
    print(
        "NOTE  going mute for %.0f s: not answering keep-alives, still sending "
        "position" % MUTE_SECONDS,
        flush=True,
    )
    before_mute = len(client.latencies)
    keep_alives_before = client.keep_alives
    client.answer_keep_alives = False
    wait_until(
        client,
        lambda: any(
            entry["latency"] == UNKNOWN for entry in client.latencies[before_mute:]
        ),
        MUTE_SECONDS,
        "the reading to go unknown while nothing answers keep-alives",
    )
    went_unknown = [
        entry for entry in client.latencies[before_mute:] if entry["latency"] == UNKNOWN
    ]
    check(
        bool(went_unknown),
        "with this client refusing to answer, the reading falls back to %d -- "
        "so the thing answering keep-alives is the client and not the proxy "
        "(latencies seen while mute: %r)"
        % (UNKNOWN, [e["latency"] for e in client.latencies[before_mute:]]),
    )
    check(
        client.keep_alives > keep_alives_before,
        "keep-alives kept arriving while mute (%d more), so the fallback is a "
        "timeout and not the server having stopped probing"
        % (client.keep_alives - keep_alives_before),
    )
    check(
        client.alive,
        "the connection survives an unanswered keep-alive; hyperion does not "
        "disconnect over one",
    )

    # --- and back ---------------------------------------------------------
    print("NOTE  answering keep-alives again", flush=True)
    before_resume = len(client.latencies)
    client.answer_keep_alives = True
    wait_until(
        client,
        lambda: any(
            entry["latency"] >= 0 for entry in client.latencies[before_resume:]
        ),
        30.0,
        "the reading to come back",
    )
    recovered = [
        entry for entry in client.latencies[before_resume:] if entry["latency"] >= 0
    ]
    check(
        bool(recovered),
        "the reading comes back once answers resume, so what went unknown was "
        "a measurement and not a dead connection (got %r)"
        % [e["latency"] for e in client.latencies[before_resume:]],
    )

    if failures:
        print(
            "RESULT: failure (%d checks failed): %s"
            % (len(failures), "; ".join(failures)),
            flush=True,
        )
        return 1
    print(
        "RESULT: success (tick rate %.1f / %.1f on the wire; ping measured "
        "against the client, which is the only thing that answers a "
        "keep-alive)" % (rate, target),
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
