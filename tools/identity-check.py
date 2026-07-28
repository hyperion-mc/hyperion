#!/usr/bin/env python3
"""Who a player is, what they may do, and what they look like.

Three questions one connection cannot answer and this client can, driven
against a live smash server:

  1. Two people with one IGN both get in. Two connections announce the same
     name; both must reach play with distinct profile ids, and each must show
     up in the other's tab list as its own entry. Deriving a profile id from
     the name, which is what this server used to do, fails here rather than in
     production: the client's `playerInfoMap` is keyed on profile id and filled
     with `putIfAbsent`, so the second arrival was silently dropped.

  2. A player cannot break the arena. The server puts everyone in adventure, so
     the tab list has to say adventure and a dig has to leave the block alone.
     Both halves matter: the mode is what a vanilla client obeys, and the
     refusal is what stops a client that does not.

  3. A player wears their kit's mob. After `/kit <name>` the profile the server
     publishes must carry a `textures` property equal to the payload committed
     for that mob, with its signature attached. An unsigned property would look
     identical from the server and be invisible to every other player.

Exits non-zero on the first thing that is not true, after printing what it saw.
"""

import argparse
import importlib.util
import pathlib
import struct
import sys
import time

TOOLS = pathlib.Path(__file__).resolve().parent
ROOT = TOOLS.parent


def _load(name, filename):
    """Import a sibling tool whose file name is not a Python identifier."""
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


match = _load("smash_match", "smash-match.py")
base = match.base

take_var_int = base.take_var_int
take_string = base.take_string
mc_string = base.mc_string
var_int = base.var_int

S2C_PLAYER_INFO_UPDATE = 0x46
S2C_BLOCK_UPDATE = 0x08
S2C_BLOCK_CHANGED_ACK = 0x04
C2S_PLAYER_ACTION = 0x29

# `PlayerInfoActions` in hyperion-minecraft-proto, which is the same bit order
# as `ClientboundPlayerInfoUpdatePacket.Action`.
ADD_PLAYER = 1 << 0
INITIALIZE_CHAT = 1 << 1
UPDATE_GAME_MODE = 1 << 2
UPDATE_LISTED = 1 << 3
UPDATE_LATENCY = 1 << 4
UPDATE_DISPLAY_NAME = 1 << 5
UPDATE_LIST_ORDER = 1 << 6
UPDATE_HAT = 1 << 7

ADVENTURE = 2

# `player_action::Action`: the start of a break and the end of one, which is
# what an instant break looks like on the wire.
START_DESTROY_BLOCK = 0
STOP_DESTROY_BLOCK = 2


def take_optional_string(payload, offset):
    present = payload[offset]
    offset += 1
    if not present:
        return None, offset
    return take_string(payload, offset)


def parse_player_info_update(payload):
    """Every entry in one `PlayerInfoUpdate`, as far as the actions describe.

    Nothing in the body says how long an entry is, so a reader that stops
    understanding a field loses the rest of the packet. Raising rather than
    returning what was read so far is deliberate: a half-read packet is a
    disagreement about the wire format, not a shorter list of players.
    """
    actions = payload[0]
    count, offset = take_var_int(payload, 1)
    entries = []
    for _ in range(count):
        entry = {"uuid": payload[offset : offset + 16].hex(), "properties": {}}
        offset += 16
        if actions & ADD_PLAYER:
            entry["name"], offset = take_string(payload, offset)
            properties, offset = take_var_int(payload, offset)
            for _ in range(properties):
                name, offset = take_string(payload, offset)
                value, offset = take_string(payload, offset)
                signature, offset = take_optional_string(payload, offset)
                entry["properties"][name] = (value, signature)
        if actions & INITIALIZE_CHAT:
            raise ValueError("this server does not sign chat, so it cannot send a session")
        if actions & UPDATE_GAME_MODE:
            entry["game_mode"], offset = take_var_int(payload, offset)
        if actions & UPDATE_LISTED:
            entry["listed"] = bool(payload[offset])
            offset += 1
        if actions & UPDATE_LATENCY:
            _, offset = take_var_int(payload, offset)
        if actions & UPDATE_DISPLAY_NAME:
            present = payload[offset]
            offset += 1
            if present:
                # A text component, whose length this reader has no business
                # knowing. hyperion sends none, so seeing one means the server
                # changed and this parser has to grow rather than guess.
                raise ValueError("PlayerInfoUpdate carried a display name")
        if actions & UPDATE_LIST_ORDER:
            _, offset = take_var_int(payload, offset)
        if actions & UPDATE_HAT:
            offset += 1
        entries.append(entry)
    if offset != len(payload):
        raise ValueError(
            "PlayerInfoUpdate has %d trailing byte(s); the action set and this "
            "reader disagree" % (len(payload) - offset)
        )
    return actions, entries


def block_position(x, y, z):
    """The packed long a `BlockPos` rides as: 26 bits x, 26 z, 12 y."""
    value = ((x & 0x3FFFFFF) << 38) | ((z & 0x3FFFFFF) << 12) | (y & 0xFFF)
    return struct.pack(">Q", value)


class Probe(match.MatchClient):
    """A scripted player that remembers the roster it was told about."""

    def __init__(self, host, port, name, tag, started):
        super().__init__(host, port, name, started)
        self.tag = tag
        self.roster = {}
        self.block_acks = []
        self.block_updates = []

    def _log(self, line):
        print("%s [%-6s] %s" % (match.stamp(self.started), self.tag, line), flush=True)

    def absorb(self, packet_id, payload):
        if packet_id == match.S2C_LOGIN:
            self.entity_id = struct.unpack(">i", payload[:4])[0]
            self.joined = True
            self.log("** in the world ** entity_id=%d" % self.entity_id)
        elif packet_id == match.S2C_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            self.position = (x, y, z)
            # Unacknowledged teleports make the server keep re-sending one, and
            # a client that never settles has no position to dig under.
            self.send(match.C2S_ACCEPT_TELEPORTATION, var_int(teleport_id))
            self.log("<- teleported to (%.1f, %.1f, %.1f)" % (x, y, z))
        elif packet_id == match.S2C_KEEP_ALIVE:
            self.send(match.C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == match.S2C_SYSTEM_CHAT:
            text, _ = match.take_nbt_string(payload, 0)
            self.log("<- chat: %s" % text)
            if text.startswith("Kit set to "):
                self.kit = text[len("Kit set to ") :].rstrip(".")
        elif packet_id == match.S2C_DISCONNECT:
            text, _ = match.take_nbt_string(payload, 0)
            self.log("<- DISCONNECTED: %s" % text)
            self.alive = False
        elif packet_id == S2C_PLAYER_INFO_UPDATE:
            actions, entries = parse_player_info_update(payload)
            for entry in entries:
                known = self.roster.setdefault(entry["uuid"], {"properties": {}})
                known.update(entry)
            self.log(
                "<- PlayerInfoUpdate actions=0x%02X %s"
                % (
                    actions,
                    ", ".join(
                        "%s%s%s"
                        % (
                            entry["uuid"][:8],
                            "/" + entry["name"] if "name" in entry else "",
                            "/textures" if "textures" in entry["properties"] else "",
                        )
                        for entry in entries
                    ),
                )
            )
        elif packet_id == S2C_BLOCK_CHANGED_ACK:
            sequence, _ = take_var_int(payload)
            self.block_acks.append(sequence)
        elif packet_id == S2C_BLOCK_UPDATE:
            (packed,) = struct.unpack(">Q", payload[:8])
            state, _ = take_var_int(payload, 8)
            self.block_updates.append((packed, state))

    def dig(self, x, y, z, sequence):
        """Start and finish breaking a block, the way an instant break looks."""
        for action in (START_DESTROY_BLOCK, STOP_DESTROY_BLOCK):
            self.send(
                C2S_PLAYER_ACTION,
                var_int(action) + block_position(x, y, z) + bytes([1]) + var_int(sequence),
            )
        self.log("-> dig (%d, %d, %d) sequence=%d" % (x, y, z, sequence))


def pump(clients, seconds):
    """Read every client for a while, keeping them all alive.

    Position too, not only keep-alives: hyperion drops a player that stops
    reporting where it is, and a gate whose clients quietly time out reads as
    a protocol failure rather than as the idling it is.
    """
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        for client in clients:
            if not client.alive:
                continue
            for packet_id, payload in client.drain():
                client.absorb(packet_id, payload)
            if client.joined:
                client.repeat_position()
        time.sleep(0.02)


def wait_until(clients, predicate, seconds, what):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        pump(clients, 0.1)
        if predicate():
            return True
    print("TIMEOUT waiting for %s" % what, file=sys.stderr)
    return False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument(
        "--name",
        default="Twin",
        help="the IGN both connections announce; the point is that it is one name",
    )
    parser.add_argument(
        "--kit",
        default="Zombie",
        help="the kit to select, whose committed skin the profile must then carry",
    )
    args = parser.parse_args()

    started = time.monotonic()
    failures = []

    def check(ok, message):
        print("%s %s" % ("PASS" if ok else "FAIL", message), flush=True)
        if not ok:
            failures.append(message)

    clients = []
    for tag in ("first", "second"):
        client = Probe(args.host, args.port, args.name, tag, started)
        client.handshake(args.host, args.port, 2)
        client.login()
        client.configuration()
        client.enter_play()
        clients.append(client)
        # Sequential rather than at once: two logins racing would leave a
        # failure ambiguous between "one IGN is refused" and "two simultaneous
        # logins are refused", and only the first is what this asks about.
        pump(clients, 0.5)

    first, second = clients

    # 1. Both got in, under one name, as two different people.
    check(
        all(client.profile_id for client in clients),
        "both connections announcing the IGN %r finished login" % args.name,
    )
    check(
        first.profile_id != second.profile_id,
        "the two share an IGN and have different profile ids (%s, %s)"
        % (first.profile_id, second.profile_id),
    )

    ids = {first.profile_id, second.profile_id}
    ok = wait_until(
        clients,
        lambda: all(ids <= set(client.roster) for client in clients),
        30.0,
        "each client's tab list to hold both players",
    )
    check(ok, "each client's tab list holds both players as separate entries")

    for client in clients:
        names = {entry.get("name") for entry in client.roster.values() if "name" in entry}
        check(
            names == {args.name},
            "%s sees the name exactly as typed, unsuffixed: %s" % (client.tag, sorted(names)),
        )

    # 2. Adventure, said and enforced.
    modes = {
        client.tag: client.roster.get(client.profile_id, {}).get("game_mode")
        for client in clients
    }
    check(
        all(mode == ADVENTURE for mode in modes.values()),
        "the server publishes adventure mode for both players: %s" % modes,
    )

    # Straight down from where the server put the player, which is solid by
    # construction: a player standing on air would already be falling through
    # the kill plane and this gate would be reading a corpse.
    x, y, z = (int(value) for value in first.position)
    target = (x, y - 1, z)
    before = len(first.block_updates)
    first.dig(target[0], target[1], target[2], 1)
    pump(clients, 3.0)

    check(
        1 in first.block_acks,
        "the server acked the dig sequence, so the client rolls its prediction "
        "back: %s" % first.block_acks,
    )
    packed = struct.unpack(">Q", block_position(*target))[0]
    broke = [update for update in first.block_updates[before:] if update[0] == packed]
    check(
        not broke,
        "digging (%d, %d, %d) did not change the block: %s" % (target + (broke,)),
    )

    # 3. The kit's mob is the skin the profile carries.
    skins = ROOT / "events" / "smash" / "skins"
    value_path = skins / (args.kit.lower() + ".value")
    sig_path = skins / (args.kit.lower() + ".sig")
    if not value_path.exists():
        check(False, "no committed skin for kit %r at %s" % (args.kit, value_path))
    else:
        expected = value_path.read_text().strip()
        first.command("kit %s" % args.kit)

        def dressed():
            entry = second.roster.get(first.profile_id, {})
            return entry.get("properties", {}).get("textures")

        ok = wait_until(clients, lambda: dressed() is not None, 30.0, "a textures property")
        check(ok, "the profile the other player sees carries a textures property")
        if ok:
            value, signature = dressed()
            check(value == expected, "that texture is the one committed for %s" % args.kit)
            check(
                signature == sig_path.read_text().strip(),
                "and it carries its Mojang signature, without which only the "
                "wearer would see it",
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
