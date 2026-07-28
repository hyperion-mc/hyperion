#!/usr/bin/env python3
"""Where a kit's abilities land on the nine keys a player actually has.

The bug this exists to refuse was invisible to every other gate. Twelve of the
fifteen kits numbered their abilities from slot 1, leaving slot 0 empty. Every
ability was present, correctly bound and reachable; the bar was simply one key
to the right of where a client's hand starts. A player spawned, right-clicked,
and nothing happened.

Nothing on the server side noticed, because nothing on the server side has an
opinion about which key a hand rests on. `ClientboundSetSelectedSlot` is not
sent on join at all: a fresh client selects slot 0 on its own, and that fact
lives in the client, not in the protocol. So this reads the inventory packets
the server does send, per kit, and holds them against the server's own registry:

  * every kit fills hotbar slot 0, which is the key selected on spawn
  * the filled keys are exactly the ones `/abilities` says they are, so the
    registry and the wire cannot drift apart
  * they run from 0 upwards with no holes
  * switching kit leaves nothing of the last one behind

One client and no match. One player is below any minimum this server is
configured to run, so a single connection stands in the hub indefinitely and
can change kit as often as it likes, which is the cheapest place to ask this
question. Said that way on purpose: this used to claim the lobby needs four,
which was `LobbyConfig::default` restated here and stopped being true when
#1019 made it two.

Exits non-zero on the first thing that is not true, after printing what it saw.
"""

import argparse
import importlib.util
import json
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

# `PlayerInventory::HOTBAR_START_SLOT`: `ClientboundContainerSetSlot` numbers
# the whole inventory and the nine keys begin here. Everything below counts in
# the nine a player sees, which is what the ability registry means by a slot.
HOTBAR_START_SLOT = match.HOTBAR_START_SLOT

# What a fresh client has selected when it reaches the world. Vanilla's
# `Inventory.selected` starts at 0 and no join sequence changes it, which is
# the whole reason an empty slot 0 is a bug rather than a preference.
SELECTED_ON_SPAWN = 0


class Bar(match.MatchClient):
    """A scripted player that remembers what is on its hotbar."""

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, started)
        self.manifest = []
        self.manifest_expected = None

    def absorb(self, packet_id, payload):
        if packet_id == match.S2C_LOGIN:
            self.entity_id = struct.unpack(">i", payload[:4])[0]
            self.joined = True
            self.log("** in the world ** entity_id=%d" % self.entity_id)
        elif packet_id == match.S2C_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            self.position = (x, y, z)
            # An unacknowledged teleport makes the server keep re-sending one.
            self.send(match.C2S_ACCEPT_TELEPORTATION, var_int(teleport_id))
            self.log("<- teleported to (%.1f, %.1f, %.1f)" % (x, y, z))
        elif packet_id == match.S2C_KEEP_ALIVE:
            self.send(match.C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == match.S2C_SYSTEM_CHAT:
            text, _ = match.take_nbt_string(payload, 0)
            if text.startswith(match.MANIFEST_PREFIX):
                self.manifest.append(json.loads(text[len(match.MANIFEST_PREFIX) :]))
                return
            if text.startswith(match.MANIFEST_END_PREFIX):
                self.manifest_expected = int(text[len(match.MANIFEST_END_PREFIX) :])
                return
            self.log("<- chat: %s" % text)
            if text.startswith("Kit set to "):
                self.kit = text[len("Kit set to ") :].rstrip(".")
        elif packet_id == match.S2C_CONTAINER_SET_SLOT:
            self.absorb_slot(payload)
        elif packet_id == match.S2C_DISCONNECT:
            text, _ = match.take_nbt_string(payload, 0)
            self.log("<- DISCONNECTED: %s" % text)
            self.alive = False

    def absorb_slot(self, payload):
        _container, offset = take_var_int(payload)
        _state, offset = take_var_int(payload, offset)
        (slot,) = struct.unpack(">h", payload[offset : offset + 2])
        offset += 2
        slot -= HOTBAR_START_SLOT
        if not 0 <= slot < 9:
            return
        count, offset = take_var_int(payload, offset)
        if count <= 0:
            # An emptied key is as much of a claim as a filled one: a kit
            # change clears the bar before refilling it, and a check that only
            # read fills would see the union of every kit tried so far.
            self.hotbar.pop(slot, None)
            self.log("<- slot %d emptied" % slot)
            return
        item_id, offset = take_var_int(payload, offset)
        self.hotbar[slot] = ITEMS[item_id] if item_id < len(ITEMS) else "<%d>" % item_id
        self.log("<- slot %d now holds %s" % (slot, self.hotbar[slot]))


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
        time.sleep(0.02)


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
    parser.add_argument("--name", default="Hotbar")
    parser.add_argument(
        "--kits",
        default="",
        help="comma-separated kits to check; the default is every kit the "
        "server's own registry names, which is the point",
    )
    args = parser.parse_args()

    started = time.time()
    failures = []

    def check(ok, message):
        print("%s %s" % ("PASS" if ok else "FAIL", message), flush=True)
        if not ok:
            failures.append(message)

    client = Bar(args.host, args.port, args.name, started)
    client.handshake(args.host, args.port, 2)
    client.login()
    client.configuration()
    client.enter_play()

    if not wait_until(client, lambda: client.joined, 60.0, "the client to reach the world"):
        check(False, "the client never reached the world")
        return 1

    client.command("abilities")
    ready = lambda: (
        client.manifest_expected is not None
        and len(client.manifest) >= client.manifest_expected
    )
    if not wait_until(client, ready, 30.0, "the ability registry"):
        check(
            False,
            "the server answered /abilities with %d of %s lines"
            % (len(client.manifest), client.manifest_expected),
        )
        return 1

    wanted = {}
    for entry in client.manifest:
        wanted.setdefault(entry["kit"], []).append(entry)
    check(
        len(wanted) >= 15,
        "the registry names %d kits, so this run covers the roster: %s"
        % (len(wanted), ", ".join(sorted(wanted))),
    )

    names = (
        [kit.strip() for kit in args.kits.split(",") if kit.strip()]
        if args.kits
        else sorted(wanted)
    )

    for name in names:
        entries = wanted.get(name)
        if entries is None:
            check(False, "the server's registry has no kit called %r" % name)
            continue

        # The bar the kit implies, as the registry describes it. The Smash
        # Crystal's ability is not granted at spawn, so it is not on the bar
        # yet and must not be.
        expected = {
            entry["slot"]: entry["item"]
            for entry in entries
            if not entry["ultimate"]
        }
        crystal = {entry["slot"] for entry in entries if entry["ultimate"]}

        client.kit = None
        client.hotbar.clear()
        client.command("kit %s" % name)
        if not wait_until(client, lambda: client.kit == name, 20.0, "kit %s" % name):
            check(False, "%s never equipped" % name)
            continue
        # The hotbar is rebuilt a tick after the kit lands, in PostUpdate.
        wait_until(
            client,
            lambda: set(client.hotbar) == set(expected),
            10.0,
            "%s's hotbar" % name,
        )
        got = dict(client.hotbar)

        check(
            SELECTED_ON_SPAWN in got,
            "%s fills slot %d, the key a client has selected when it spawns "
            "(bar: %s)" % (name, SELECTED_ON_SPAWN, layout(got)),
        )
        check(
            set(got) == set(expected),
            "%s puts items on exactly the keys its registry entries name: "
            "wire %s, registry %s" % (name, sorted(got), sorted(expected)),
        )
        check(
            sorted(got) == list(range(len(got))),
            "%s runs from slot 0 upwards with no holes: %s" % (name, sorted(got)),
        )
        wrong = {
            slot: (item, expected[slot])
            for slot, item in got.items()
            if slot in expected and item != expected[slot]
        }
        check(
            not wrong,
            "%s puts the item its registry names on each key: %s"
            % (name, wrong if wrong else layout(got)),
        )
        check(
            not (crystal & set(got)),
            "%s leaves the Smash Crystal's key %s empty until a crystal is "
            "picked up" % (name, sorted(crystal)),
        )

    print(
        "RESULT: %s (%d checks failed)"
        % ("ok" if not failures else "failure", len(failures)),
        flush=True,
    )
    for failure in failures:
        print("  failed: %s" % failure, file=sys.stderr)
    return 1 if failures else 0


def layout(bar):
    return ", ".join("%d:%s" % (slot, bar[slot]) for slot in sorted(bar))


if __name__ == "__main__":
    sys.exit(main())
