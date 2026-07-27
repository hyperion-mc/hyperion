#!/usr/bin/env python3
"""A scripted Minecraft 26.2 client, for proving the game server is joinable.

Not a bot framework. It answers the one question the proxy cannot: does a
client reach *play* state, or does it authenticate and then sit on "Joining
world..." forever because the server never sent it the Login packet? On 763
that meant watching for GameJoinS2c; on 776 there are two more gates before it,
the configuration state and the known-pack negotiation, and either one can
silently swallow a client.

Offline mode, protocol 776, which is what crates/hyperion/src/net/mod.rs
declares.
"""

from __future__ import annotations

import argparse
import socket
import struct
import sys
import time
import zlib

PROTOCOL = 776
VERSION = "26.2"

# Serverbound ids, from crates/hyperion-minecraft-proto/src/generated/packet_id.rs.
C2S_INTENTION = 0x00
C2S_STATUS_REQUEST = 0x00
C2S_PING_REQUEST = 0x01
C2S_HELLO = 0x00
C2S_LOGIN_ACKNOWLEDGED = 0x03
C2S_CONFIG_CLIENT_INFORMATION = 0x00
C2S_CONFIG_FINISH = 0x03
C2S_CONFIG_KEEP_ALIVE = 0x04
C2S_CONFIG_SELECT_KNOWN_PACKS = 0x07
C2S_PLAY_ACCEPT_TELEPORTATION = 0x00
C2S_PLAY_KEEP_ALIVE = 0x1C

# Clientbound ids, same source.
S2C_STATUS_RESPONSE = 0x00
S2C_PONG_RESPONSE = 0x01
S2C_LOGIN_DISCONNECT = 0x00
S2C_LOGIN_FINISHED = 0x02
S2C_LOGIN_COMPRESSION = 0x03
S2C_CONFIG_CUSTOM_PAYLOAD = 0x01
S2C_CONFIG_DISCONNECT = 0x02
S2C_CONFIG_FINISH = 0x03
S2C_CONFIG_KEEP_ALIVE = 0x04
S2C_CONFIG_REGISTRY_DATA = 0x07
S2C_CONFIG_UPDATE_ENABLED_FEATURES = 0x0C
S2C_CONFIG_UPDATE_TAGS = 0x0D
S2C_CONFIG_SELECT_KNOWN_PACKS = 0x0E
S2C_PLAY_DISCONNECT = 0x20
S2C_PLAY_GAME_EVENT = 0x26
S2C_PLAY_KEEP_ALIVE = 0x2C
S2C_PLAY_LEVEL_CHUNK = 0x2D
S2C_PLAY_LOGIN = 0x31
S2C_PLAY_PLAYER_POSITION = 0x48
S2C_PLAY_SET_CHUNK_CACHE_CENTER = 0x5E
S2C_PLAY_SET_DEFAULT_SPAWN = 0x61

PLAY_NAMES = {
    S2C_PLAY_DISCONNECT: "Disconnect",
    S2C_PLAY_GAME_EVENT: "GameEvent",
    S2C_PLAY_KEEP_ALIVE: "KeepAlive",
    S2C_PLAY_LEVEL_CHUNK: "LevelChunkWithLight",
    S2C_PLAY_LOGIN: "Login",
    S2C_PLAY_PLAYER_POSITION: "PlayerPosition",
    S2C_PLAY_SET_CHUNK_CACHE_CENTER: "SetChunkCacheCenter",
    S2C_PLAY_SET_DEFAULT_SPAWN: "SetDefaultSpawnPosition",
}


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
    return payload[offset : offset + length].decode(), offset + length


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

    def handshake(self, host, port, intent):
        self.log(
            "-> Intention protocol=%d intent=%s"
            % (PROTOCOL, "status" if intent == 1 else "login")
        )
        self.send(
            C2S_INTENTION,
            var_int(PROTOCOL) + mc_string(host) + struct.pack(">H", port) + var_int(intent),
        )

    def status(self):
        self.send(C2S_STATUS_REQUEST)
        packet_id, payload = self.recv()
        if packet_id != S2C_STATUS_RESPONSE:
            raise SystemExit("status: expected 0x00, got 0x%02X" % packet_id)
        text, _ = take_string(payload)
        self.log("<- StatusResponse %s" % text)

        self.send(C2S_PING_REQUEST, struct.pack(">q", 0x1234))
        packet_id, payload = self.recv()
        if packet_id != S2C_PONG_RESPONSE:
            raise SystemExit("status: expected pong 0x01, got 0x%02X" % packet_id)
        (echoed,) = struct.unpack(">q", payload[:8])
        self.log("<- PongResponse time=0x%X" % echoed)
        return text

    def login(self):
        self.log("-> Hello name=%s" % self.name)
        self.send(C2S_HELLO, mc_string(self.name) + b"\x00" * 16)

        while True:
            packet_id, payload = self.recv()
            if packet_id == S2C_LOGIN_COMPRESSION:
                self.threshold, _ = take_var_int(payload)
                self.log("<- LoginCompression threshold=%d" % self.threshold)
            elif packet_id == S2C_LOGIN_FINISHED:
                name, _ = take_string(payload, 16)
                self.log(
                    "<- LoginFinished profile=%s name=%s "
                    "(authenticated, NOT yet in the world)" % (payload[:16].hex(), name)
                )
                self.log("-> LoginAcknowledged")
                self.send(C2S_LOGIN_ACKNOWLEDGED)
                return
            elif packet_id == S2C_LOGIN_DISCONNECT:
                reason, _ = take_string(payload)
                raise SystemExit("login refused: %s" % reason)
            else:
                self.log("<- login packet 0x%02X" % packet_id)

    def configuration(self):
        """Run the configuration state until the server says it is finished."""
        registries = []
        while True:
            packet_id, payload = self.recv()
            if packet_id == S2C_CONFIG_SELECT_KNOWN_PACKS:
                count, offset = take_var_int(payload)
                packs = []
                for _ in range(count):
                    namespace, offset = take_string(payload, offset)
                    ident, offset = take_string(payload, offset)
                    version, offset = take_string(payload, offset)
                    packs.append((namespace, ident, version))
                self.log("<- SelectKnownPacks %s" % packs)
                # Echo them back verbatim. A vanilla client reports the packs it
                # shipped with, and this client claims the same ones so the
                # server may leave registry contents out.
                body = var_int(len(packs))
                for namespace, ident, version in packs:
                    body += mc_string(namespace) + mc_string(ident) + mc_string(version)
                self.log("-> SelectKnownPacks (accepting all)")
                self.send(C2S_CONFIG_SELECT_KNOWN_PACKS, body)
                self.log("-> ClientInformation view_distance=10 skin_parts=0x7F")
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
            elif packet_id == S2C_CONFIG_REGISTRY_DATA:
                name, offset = take_string(payload)
                count, offset = take_var_int(payload, offset)
                registries.append((name, count))
            elif packet_id == S2C_CONFIG_UPDATE_TAGS:
                count, _ = take_var_int(payload)
                self.log("<- UpdateTags registries=%d" % count)
            elif packet_id == S2C_CONFIG_UPDATE_ENABLED_FEATURES:
                count, offset = take_var_int(payload)
                names = []
                for _ in range(count):
                    feature, offset = take_string(payload, offset)
                    names.append(feature)
                self.log("<- UpdateEnabledFeatures %s" % names)
            elif packet_id == S2C_CONFIG_CUSTOM_PAYLOAD:
                channel, offset = take_string(payload)
                self.log("<- CustomPayload %s %s" % (channel, printable(payload[offset:], 40)))
            elif packet_id == S2C_CONFIG_KEEP_ALIVE:
                self.send(C2S_CONFIG_KEEP_ALIVE, payload[:8])
            elif packet_id == S2C_CONFIG_FINISH:
                total = sum(count for _, count in registries)
                self.log(
                    "<- RegistryData x%d (%d elements): %s"
                    % (len(registries), total, ", ".join(n for n, _ in registries))
                )
                self.log("<- FinishConfiguration")
                self.log("-> FinishConfiguration (ack)")
                self.send(C2S_CONFIG_FINISH)
                return
            elif packet_id == S2C_CONFIG_DISCONNECT:
                raise SystemExit("configuration refused: %s" % printable(payload))
            else:
                self.log("<- configuration packet 0x%02X len=%d %s" % (packet_id, len(payload), payload[:48].hex()))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument("--name", default="Prober")
    parser.add_argument("--seconds", type=float, default=15.0)
    parser.add_argument(
        "--status-only",
        action="store_true",
        help="do the server list ping and stop, the way a client showing the "
        "multiplayer menu does",
    )
    args = parser.parse_args()

    def log(line):
        print("[%s] %s" % (args.name, line), flush=True)

    client = Client(args.host, args.port, args.name, log)

    if args.status_only:
        client.handshake(args.host, args.port, 1)
        client.status()
        log("RESULT: status ok")
        return 0

    client.handshake(args.host, args.port, 2)
    client.login()
    client.configuration()

    started = time.time()
    deadline = started + args.seconds
    seen = {}

    while time.time() < deadline:
        try:
            packet_id, payload = client.recv()
        except (socket.timeout, TimeoutError):
            break
        except ConnectionError as error:
            log("connection lost: %s" % error)
            break

        name = PLAY_NAMES.get(packet_id, "0x%02X" % packet_id)
        seen[name] = seen.get(name, 0) + 1

        if packet_id == S2C_PLAY_LOGIN:
            client.entity_id = struct.unpack(">i", payload[:4])[0]
            client.joined = True
            log("<- Login entity_id=%d  ** REACHED PLAY STATE **" % client.entity_id)
        elif packet_id == S2C_PLAY_SET_CHUNK_CACHE_CENTER:
            x, offset = take_var_int(payload)
            z, _ = take_var_int(payload, offset)
            log("<- SetChunkCacheCenter (%d, %d)" % (x, z))
        elif packet_id == S2C_PLAY_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            log(
                "<- PlayerPosition (%.1f, %.1f, %.1f) teleport_id=%d"
                % (x, y, z, teleport_id)
            )
            client.send(C2S_PLAY_ACCEPT_TELEPORTATION, var_int(teleport_id))
        elif packet_id == S2C_PLAY_SET_DEFAULT_SPAWN:
            log("<- SetDefaultSpawnPosition")
        elif packet_id == S2C_PLAY_KEEP_ALIVE:
            client.send(C2S_PLAY_KEEP_ALIVE, payload[:8])
        elif packet_id == S2C_PLAY_DISCONNECT:
            log("<- Disconnect %s" % printable(payload))
            break

    log("summary: " + ", ".join("%s x%d" % (k, v) for k, v in sorted(seen.items())))
    if client.joined:
        log("RESULT: reached play state (Login received)")
        return 0
    log("RESULT: never reached play state (client would sit on 'Joining world...')")
    return 1


if __name__ == "__main__":
    sys.exit(main())
