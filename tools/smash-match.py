#!/usr/bin/env python3
"""Drive several scripted clients through one whole Super Smash Mobs match.

`smash-client.py` answers "is the server joinable". This answers the next
question, which is the one a single client structurally cannot: does a *match*
happen. `min_players` is four, so nothing past the hub is reachable until four
clients are in the world at once, and four separate processes cannot attack each
other because neither knows the other's entity id. Both problems disappear if
one process owns every socket, so this drives all of them from a single loop and
prints one interleaved transcript.

What this proves and what it does not
-------------------------------------
It proves the server's own state machine: countdown, scatter onto spawn points,
hotbars, kill credit, lives, respawn, elimination, the end screen and the return
to the hub. Every line below is a packet the server actually sent.

It does not prove the game is *playable by a human*. These clients do not
render, do not simulate physics and never disagree with the server. In
particular they teleport rather than walk, so nothing here says the platforms
have collision from a real client's point of view, and nothing here exercises
the client-side prediction that knockback ultimately has to survive. A real
1.20.1-or-later client cannot be scripted here at all: it needs the game and a
Mojang account.

Protocol note: hyperion is mid-migration. Login and configuration are protocol
776 (26.2); the play state is still valence's 763 (1.20.1) for everything except
the join sequence itself. This client therefore speaks 776 to get in and 763
once it is in, which is neither version and is exactly what the server is today.
"""

from __future__ import annotations

import argparse
import socket
import struct
import sys
import time
import zlib

PROTOCOL = 776

# Pre-play, protocol 776. Ids from
# crates/hyperion-minecraft-proto/src/generated/packet_id.rs.
C2S_INTENTION = 0x00
C2S_HELLO = 0x00
C2S_LOGIN_ACKNOWLEDGED = 0x03
C2S_CONFIG_CLIENT_INFORMATION = 0x00
C2S_CONFIG_FINISH = 0x03
C2S_CONFIG_KEEP_ALIVE = 0x04
C2S_CONFIG_SELECT_KNOWN_PACKS = 0x07

S2C_LOGIN_DISCONNECT = 0x00
S2C_LOGIN_FINISHED = 0x02
S2C_LOGIN_COMPRESSION = 0x03
S2C_CONFIG_DISCONNECT = 0x02
S2C_CONFIG_FINISH = 0x03
S2C_CONFIG_KEEP_ALIVE = 0x04
S2C_CONFIG_SELECT_KNOWN_PACKS = 0x0E

# The join sequence, which hyperion has already moved to 776.
S2C_PLAY776_LOGIN = 0x31
S2C_PLAY776_PLAYER_POSITION = 0x48
C2S_PLAY776_ACCEPT_TELEPORTATION = 0x00

# Everything after the join, which is still valence 763.
C2S_TELEPORT_CONFIRM = 0x00
C2S_COMMAND = 0x04
C2S_CLIENT_SETTINGS = 0x08
C2S_INTERACT = 0x10
C2S_KEEP_ALIVE = 0x12
C2S_POSITION = 0x14
C2S_POSITION_LOOK = 0x15
C2S_SELECT_SLOT = 0x28
C2S_SWING = 0x2F
C2S_USE_ITEM = 0x32

S2C_DISCONNECT = 0x1A
S2C_KEEP_ALIVE = 0x23
S2C_POSITION_LOOK = 0x3C
S2C_ACTION_BAR = 0x46
S2C_VELOCITY = 0x54
S2C_HEALTH = 0x57
S2C_SCOREBOARD_SCORE = 0x5B
S2C_TITLE = 0x5D
S2C_CHAT = 0x64


def var_int(value):
    out = bytearray()
    value &= 0xFFFFFFFF
    while True:
        byte = value & 0x7F
        value >>= 7
        out.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(out)


def take_var_int(payload, offset=0):
    result = 0
    for shift in range(0, 35, 7):
        byte = payload[offset]
        offset += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, offset
    raise ValueError("var int too long")


def mc_string(text):
    raw = text.encode()
    return var_int(len(raw)) + raw


def take_string(payload, offset=0):
    length, offset = take_var_int(payload, offset)
    return payload[offset : offset + length].decode("utf-8", "replace"), offset + length


def readable(payload, limit=160):
    """The printable runs in a payload, which is where chat and titles live.

    The 763 chat packet is a length-prefixed JSON component followed by binary
    signature fields; pulling the printable runs out is enough to read the
    message without implementing the text codec a second time.
    """
    out = []
    run = []
    for byte in payload:
        if 32 <= byte < 127:
            run.append(chr(byte))
        else:
            if len(run) >= 4:
                out.append("".join(run))
            run = []
    if len(run) >= 4:
        out.append("".join(run))
    return " | ".join(out)[:limit]


START = time.time()


def stamp():
    return "%7.2fs" % (time.time() - START)


class Client:
    def __init__(self, host, port, name):
        self.sock = socket.create_connection((host, port))
        self.sock.settimeout(0.05)
        self.host = host
        self.port = port
        self.name = name
        self.threshold = -1
        self.entity_id = None
        self.joined = False
        self.pos = (0.0, 64.0, 0.0)
        self.buffer = b""
        self.last_position_sent = 0.0
        self.alive = True

    def log(self, line):
        print("%s [%-6s] %s" % (stamp(), self.name, line), flush=True)

    # --- framing -------------------------------------------------------

    def send(self, packet_id, payload=b""):
        body = var_int(packet_id) + payload
        if self.threshold >= 0:
            if len(body) >= self.threshold:
                body = var_int(len(body)) + zlib.compress(body)
            else:
                body = var_int(0) + body
        self.sock.sendall(var_int(len(body)) + body)

    def _fill(self, want):
        while len(self.buffer) < want:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("server closed the connection")
            self.buffer += chunk

    def _blocking_recv(self):
        """One packet, waiting for it. Used only while getting in."""
        self.sock.settimeout(20)
        try:
            result = 0
            shift = 0
            while True:
                self._fill(1)
                byte = self.buffer[0]
                self.buffer = self.buffer[1:]
                result |= (byte & 0x7F) << shift
                shift += 7
                if not byte & 0x80:
                    break
            self._fill(result)
            body, self.buffer = self.buffer[:result], self.buffer[result:]
        finally:
            self.sock.settimeout(0.05)
        return self._decode(body)

    def drain(self):
        """Every packet already readable, without blocking."""
        try:
            chunk = self.sock.recv(1 << 20)
            if not chunk:
                raise ConnectionError("server closed the connection")
            self.buffer += chunk
        except (socket.timeout, TimeoutError, BlockingIOError):
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
            out.append(self._decode(body))

    def _decode(self, body):
        if self.threshold >= 0:
            size, offset = take_var_int(body)
            body = zlib.decompress(body[offset:]) if size else body[offset:]
        packet_id, offset = take_var_int(body)
        return packet_id, body[offset:]

    # --- getting in ----------------------------------------------------

    def login(self):
        self.send(
            C2S_INTENTION,
            var_int(PROTOCOL)
            + mc_string(self.host)
            + struct.pack(">H", self.port)
            + var_int(2),
        )
        self.send(C2S_HELLO, mc_string(self.name) + b"\x00" * 16)

        while True:
            packet_id, payload = self._blocking_recv()
            if packet_id == S2C_LOGIN_COMPRESSION:
                self.threshold, _ = take_var_int(payload)
            elif packet_id == S2C_LOGIN_FINISHED:
                self.log("authenticated (not yet in the world)")
                self.send(C2S_LOGIN_ACKNOWLEDGED)
                return
            elif packet_id == S2C_LOGIN_DISCONNECT:
                raise SystemExit(
                    "%s: login refused: %s" % (self.name, readable(payload))
                )

    def configuration(self):
        while True:
            packet_id, payload = self._blocking_recv()
            if packet_id == S2C_CONFIG_SELECT_KNOWN_PACKS:
                count, offset = take_var_int(payload)
                packs = b""
                for _ in range(count):
                    namespace, offset = take_string(payload, offset)
                    pack_id, offset = take_string(payload, offset)
                    version, offset = take_string(payload, offset)
                    packs += (
                        mc_string(namespace) + mc_string(pack_id) + mc_string(version)
                    )
                self.send(C2S_CONFIG_SELECT_KNOWN_PACKS, var_int(count) + packs)
                self.send(
                    C2S_CONFIG_CLIENT_INFORMATION,
                    mc_string("en_us")
                    + bytes([10])
                    + var_int(0)
                    + b"\x01"
                    + bytes([0x7F])
                    + var_int(1)
                    + b"\x01\x01"
                    + var_int(0),
                )
            elif packet_id == S2C_CONFIG_KEEP_ALIVE:
                self.send(C2S_CONFIG_KEEP_ALIVE, payload[:8])
            elif packet_id == S2C_CONFIG_FINISH:
                self.send(C2S_CONFIG_FINISH)
                return
            elif packet_id == S2C_CONFIG_DISCONNECT:
                raise SystemExit(
                    "%s: configuration refused: %s" % (self.name, readable(payload))
                )

    # --- acting --------------------------------------------------------

    def command(self, text):
        self.log("-> /%s" % text)
        self.send(
            C2S_COMMAND,
            mc_string(text)
            + struct.pack(">q", 0)
            + struct.pack(">q", 0)
            + var_int(0)
            + var_int(0)
            + b"\x00\x00\x00",
        )

    def move_to(self, x, y, z, note=None):
        self.pos = (x, y, z)
        self.send(C2S_POSITION_LOOK, struct.pack(">dddff?", x, y, z, 0.0, 0.0, True))
        if note:
            self.log("-> moved to (%.1f, %.1f, %.1f)  %s" % (x, y, z, note))

    def keepalive_position(self):
        """Re-assert where we are, the way a real client does every tick.

        Without it the server's mirrored position goes stale and the arena's
        bounds check keeps reading wherever we last claimed to be.
        """
        now = time.time()
        if now - self.last_position_sent < 0.2:
            return
        self.last_position_sent = now
        x, y, z = self.pos
        self.send(C2S_POSITION, struct.pack(">ddd?", x, y, z, True))

    def attack(self, target_entity_id, target_name):
        self.log("-> attack %s (entity %d)" % (target_name, target_entity_id))
        self.send(C2S_INTERACT, var_int(target_entity_id) + var_int(1) + b"\x00")
        self.send(C2S_SWING, var_int(0))

    def use_slot(self, slot):
        self.log("-> right-click slot %d" % slot)
        self.send(C2S_SELECT_SLOT, struct.pack(">h", slot))
        self.send(C2S_USE_ITEM, var_int(0) + var_int(1))
        self.send(C2S_SWING, var_int(0))


def pump(client):
    """Read everything pending and narrate the interesting parts."""
    for packet_id, payload in client.drain():
        if packet_id == S2C_PLAY776_LOGIN:
            client.entity_id = struct.unpack(">i", payload[:4])[0]
            client.joined = True
            client.log("** in the world ** entity_id=%d" % client.entity_id)
            client.send(
                C2S_CLIENT_SETTINGS,
                mc_string("en_us")
                + bytes([10])
                + var_int(0)
                + b"\x01"
                + bytes([0x7F])
                + var_int(1)
                + b"\x01\x01",
            )
        elif packet_id == S2C_PLAY776_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            client.pos = (x, y, z)
            client.log("<- spawned at (%.1f, %.1f, %.1f)" % (x, y, z))
            client.send(C2S_PLAY776_ACCEPT_TELEPORTATION, var_int(teleport_id))
        elif packet_id == S2C_POSITION_LOOK:
            x, y, z, _yaw, _pitch = struct.unpack(">dddff", payload[:32])
            teleport_id, _ = take_var_int(payload, 33)
            client.pos = (x, y, z)
            client.log("<- teleported to (%.1f, %.1f, %.1f)" % (x, y, z))
            client.send(C2S_TELEPORT_CONFIRM, var_int(teleport_id))
        elif packet_id == S2C_KEEP_ALIVE:
            client.send(C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == S2C_CHAT:
            text = readable(payload)
            if text:
                client.log("<- chat: %s" % text)
        elif packet_id == S2C_TITLE:
            client.log("<- TITLE: %s" % readable(payload))
        elif packet_id == S2C_ACTION_BAR:
            client.log("<- action bar: %s" % readable(payload))
        elif packet_id == S2C_HEALTH:
            health = struct.unpack(">f", payload[:4])[0]
            client.log("<- health %.2f/20" % health)
        elif packet_id == S2C_VELOCITY:
            entity, offset = take_var_int(payload)
            vx, vy, vz = struct.unpack(">hhh", payload[offset : offset + 6])
            client.log("<- knockback on entity %d: (%d, %d, %d)" % (entity, vx, vy, vz))
        elif packet_id == S2C_SCOREBOARD_SCORE:
            client.log("<- sidebar row: %s" % readable(payload, 80))
        elif packet_id == S2C_DISCONNECT:
            client.log("<- DISCONNECTED: %s" % readable(payload))
            client.alive = False


def build_script(clients, kits, void_y, match_start):
    """The match, as a list of (seconds since start, thing to do).

    Written out rather than driven by reacting to server messages, because a
    transcript whose timings are fixed is one anybody can hold against the
    server's own configured countdowns.
    """
    schedule = []

    def at(seconds, action):
        schedule.append((seconds, action))

    for index, (client, kit) in enumerate(zip(clients, kits)):
        at(2.0 + index * 0.3, lambda c=client, k=kit: c.command("kit %s" % k))

    def fall(victim, killer):
        """Make `victim` walk off the map after `killer` hits it.

        Kill credit expires ten seconds after the last hit, so the hit and the
        fall have to be close together for the death to read as a smash rather
        than as an accident.
        """

        def go():
            if killer is not None and killer.entity_id is not None:
                killer.attack(victim.entity_id, victim.name)
            x, _y, z = victim.pos
            victim.move_to(x, void_y, z, "off the edge")

        return go

    clock = match_start
    # Three of the four lose all four lives, which is what ends a match: the
    # `Playing` phase ends the moment one player is left alive.
    victims = clients[1:]
    for _round in range(4):
        for victim in victims:
            killer = clients[0]
            at(clock, fall(victim, killer))
            clock += 1.5
        # Four seconds of spectating, then the respawn, then a moment for the
        # client to adopt the position the server teleported it to.
        clock += 6.0

    return sorted(schedule, key=lambda entry: entry[0])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument(
        "--kits",
        default="Skeleton,Iron Golem,Enderman,Slime",
        help="comma-separated, one per client; the client count comes from this",
    )
    parser.add_argument("--seconds", type=float, default=300.0)
    parser.add_argument(
        "--match-start",
        type=float,
        default=75.0,
        help="when the script expects to be on the map: the sixty-second "
        "countdown at the minimum player count plus the nine-second prepare",
    )
    parser.add_argument(
        "--void-y",
        type=float,
        default=-5.0,
        help="the Y a client claims to be at when the script wants it to die",
    )
    args = parser.parse_args()

    kits = [kit.strip() for kit in args.kits.split(",") if kit.strip()]
    names = ["P%d" % (index + 1) for index in range(len(kits))]

    clients = []
    for name in names:
        client = Client(args.host, args.port, name)
        client.login()
        client.configuration()
        clients.append(client)

    deadline = time.time() + args.seconds
    script = build_script(clients, kits, args.void_y, args.match_start)
    step = 0

    while time.time() < deadline:
        for client in clients:
            if client.alive:
                pump(client)
                if client.joined:
                    client.keepalive_position()
        if step < len(script):
            when, action = script[step]
            if time.time() - START >= when:
                action()
                step += 1
        time.sleep(0.02)

    print("\n%s transcript complete" % stamp(), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
