#!/usr/bin/env python3
"""A scripted Minecraft 1.20.1 client, for proving the game server is joinable.

Not a bot framework. It exists to answer the one question the proxy cannot: does
a client reach *play* state, or does it authenticate and then sit on "Joining
world..." forever because the server never sent it the Login packet?

Offline mode, protocol 763, which is what crates/hyperion/src/net/mod.rs
declares.
"""

from __future__ import annotations

import argparse
import socket
import struct
import sys
import time
import zlib

PROTOCOL = 763

# Clientbound play ids, from valence's extracted/packets.json.
S2C_NAMES = {
    0x14: "ScreenHandlerSlotUpdateS2C",
    0x1A: "DisconnectS2C",
    0x1F: "GameStateChangeS2C",
    0x23: "KeepAliveS2C",
    0x24: "ChunkDataS2C",
    0x28: "GameJoinS2C",
    0x3C: "PlayerPositionLookS2C",
    0x41: "PlayerRespawnS2C",
    0x46: "OverlayMessageS2C",
    0x4E: "ChunkRenderDistanceCenterS2C",
    0x51: "ScoreboardDisplayS2C",
    0x54: "EntityVelocityUpdateS2C",
    0x57: "HealthUpdateS2C",
    0x58: "ScoreboardObjectiveUpdateS2C",
    0x5B: "ScoreboardPlayerUpdateS2C",
    0x64: "GameMessageS2C",
}

C2S_TELEPORT_CONFIRM = 0x00
C2S_COMMAND = 0x04
C2S_CLIENT_SETTINGS = 0x08
C2S_INTERACT = 0x10
C2S_KEEP_ALIVE = 0x12
C2S_FULL = 0x15
C2S_SELECT_SLOT = 0x28
C2S_SWING = 0x2F
C2S_USE_ITEM = 0x32


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


def printable(payload, limit=180):
    text = "".join(chr(b) if 32 <= b < 127 else "." for b in payload)
    return text[:limit]


class Client:
    def __init__(self, host, port, name, log):
        self.sock = socket.create_connection((host, port))
        self.sock.settimeout(20)
        self.name = name
        self.threshold = -1
        self.log = log
        self.entity_id = None
        self.joined = False

    def read_exact(self, count):
        buf = b""
        while len(buf) < count:
            chunk = self.sock.recv(count - len(buf))
            if not chunk:
                raise ConnectionError("server closed the connection")
            buf += chunk
        return buf

    def read_var_int_stream(self):
        result = 0
        for shift in range(0, 35, 7):
            byte = self.read_exact(1)[0]
            result |= (byte & 0x7F) << shift
            if not byte & 0x80:
                return result
        raise ValueError("var int too long")

    def send(self, packet_id, payload=b""):
        body = var_int(packet_id) + payload
        if self.threshold >= 0:
            if len(body) >= self.threshold:
                body = var_int(len(body)) + zlib.compress(body)
            else:
                body = var_int(0) + body
        self.sock.sendall(var_int(len(body)) + body)

    def recv(self):
        length = self.read_var_int_stream()
        body = self.read_exact(length)
        if self.threshold >= 0:
            size, offset = take_var_int(body)
            body = zlib.decompress(body[offset:]) if size else body[offset:]
        packet_id, offset = take_var_int(body)
        return packet_id, body[offset:]

    def login(self, port):
        self.log("-> HandshakeC2S protocol=%d next_state=login" % PROTOCOL)
        self.send(
            0x00,
            var_int(PROTOCOL)
            + mc_string("127.0.0.1")
            + struct.pack(">H", port)
            + var_int(2),
        )
        self.log("-> LoginHelloC2S name=%s" % self.name)
        self.send(0x00, mc_string(self.name) + b"\x00")

        while True:
            packet_id, payload = self.recv()
            if packet_id == 0x03:
                self.threshold, _ = take_var_int(payload)
                self.log("<- LoginCompressionS2C threshold=%d" % self.threshold)
            elif packet_id == 0x02:
                self.log("<- LoginSuccessS2C (authenticated, NOT yet in the world)")
                return
            elif packet_id == 0x00:
                raise SystemExit("login refused: %s" % printable(payload))
            else:
                self.log("<- login packet 0x%02X" % packet_id)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25575)
    parser.add_argument("--name", default="Smasher")
    parser.add_argument("--seconds", type=float, default=25.0)
    parser.add_argument("--act-after", type=float, default=6.0)
    parser.add_argument("--command", action="append", default=[])
    parser.add_argument("--use-slot", type=int, default=None)
    parser.add_argument("--attack", type=int, default=None)
    parser.add_argument("--show-slots", action="store_true")
    parser.add_argument(
        "--act-again-after",
        type=float,
        default=None,
        help="seconds after joining to fire --use-slot and --attack; defaults to "
        "--act-after, but a kit command needs a tick or two to land first",
    )
    args = parser.parse_args()

    def log(line):
        print("[%s] %s" % (args.name, line), flush=True)

    client = Client(args.host, args.port, args.name, log)
    client.login(args.port)

    started = time.time()
    deadline = started + args.seconds
    acted = False
    acted_again = False
    seen = {}
    act_again_after = (
        args.act_again_after if args.act_again_after is not None else args.act_after
    )

    while time.time() < deadline:
        try:
            packet_id, payload = client.recv()
        except (socket.timeout, TimeoutError):
            break
        except ConnectionError as error:
            log("connection lost: %s" % error)
            break

        name = S2C_NAMES.get(packet_id)
        if name:
            seen[name] = seen.get(name, 0) + 1

        if packet_id == 0x28:
            client.entity_id = struct.unpack(">i", payload[:4])[0]
            client.joined = True
            log("<- GameJoinS2C entity_id=%d  ** REACHED PLAY STATE **" % client.entity_id)
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
        elif packet_id == 0x3C:
            x, y, z, yaw, pitch = struct.unpack(">dddff", payload[:32])
            teleport_id, _ = take_var_int(payload, 33)
            log(
                "<- PlayerPositionLookS2C (%.1f, %.1f, %.1f) teleport_id=%d"
                % (x, y, z, teleport_id)
            )
            client.send(C2S_TELEPORT_CONFIRM, var_int(teleport_id))
            client.send(C2S_FULL, struct.pack(">dddff?", x, y, z, yaw, pitch, True))
        elif packet_id == 0x23:
            client.send(C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == 0x64:
            log("<- GameMessageS2C %s" % printable(payload))
        elif packet_id == 0x46:
            log("<- OverlayMessageS2C %s" % printable(payload))
        elif packet_id == 0x57:
            health = struct.unpack(">f", payload[:4])[0]
            log("<- HealthUpdateS2C health=%.2f" % health)
        elif packet_id == 0x54:
            entity, offset = take_var_int(payload)
            vx, vy, vz = struct.unpack(">hhh", payload[offset : offset + 6])
            log(
                "<- EntityVelocityUpdateS2C entity=%d velocity=(%d, %d, %d)"
                % (entity, vx, vy, vz)
            )
        elif packet_id == 0x58:
            log("<- ScoreboardObjectiveUpdateS2C %s" % printable(payload, 100))
        elif packet_id == 0x5B:
            log("<- ScoreboardPlayerUpdateS2C %s" % printable(payload, 100))
        elif packet_id == 0x14 and args.show_slots:
            log("<- ScreenHandlerSlotUpdateS2C %s" % printable(payload, 120))
        elif packet_id == 0x1A:
            log("<- DisconnectS2C %s" % printable(payload))
            break

        if client.joined and not acted and time.time() - started > args.act_after:
            acted = True
            for command in args.command:
                log("-> CommandExecutionC2S /%s" % command)
                # command, timestamp, salt, no argument signatures, no
                # acknowledged messages, and the fixed 20-bit acknowledgement
                # bitset that 1.20.1 always carries.
                client.send(
                    C2S_COMMAND,
                    mc_string(command)
                    + struct.pack(">q", 0)
                    + struct.pack(">q", 0)
                    + var_int(0)
                    + var_int(0)
                    + b"\x00\x00\x00",
                )

        # A kit command has to land, and its hotbar has to arrive, before a
        # right-click means anything -- an empty hand never produces an
        # ItemInteract at all. So the acting is two phases, not one.
        if (
            client.joined
            and acted
            and not acted_again
            and time.time() - started > act_again_after
        ):
            acted_again = True
            if args.use_slot is not None:
                log("-> UpdateSelectedSlotC2S slot=%d" % args.use_slot)
                client.send(C2S_SELECT_SLOT, struct.pack(">h", args.use_slot))
                # Twice on purpose. The second one lands while the ability is on
                # cooldown, and the refusal that comes back on the action bar is
                # the only externally visible proof that the click reached the
                # game's ability gate rather than being dropped somewhere.
                for attempt in (1, 2):
                    log("-> PlayerInteractItemC2S right click #%d" % attempt)
                    client.send(C2S_USE_ITEM, var_int(0) + var_int(attempt))
                    client.send(C2S_SWING, var_int(0))
            if args.attack is not None:
                log("-> PlayerInteractEntityC2S attack entity=%d" % args.attack)
                client.send(C2S_INTERACT, var_int(args.attack) + var_int(1) + b"\x00")
                client.send(C2S_SWING, var_int(0))

    log("summary: " + ", ".join("%s x%d" % (k, v) for k, v in sorted(seen.items())))
    if client.joined:
        log("RESULT: reached play state (GameJoinS2C received)")
        return 0
    log("RESULT: never reached play state (client would sit on 'Joining world...')")
    return 1


if __name__ == "__main__":
    sys.exit(main())
