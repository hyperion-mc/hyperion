#!/usr/bin/env python3
"""Ask a Minecraft server for its status, the way a client's server list does.

Why this exists: `systemctl is-active hyperion-proxy` is not evidence. The proxy
binds its listener at startup and dials the game server per connection, so it
reads `active` while the game server is completely unreachable. For twelve hours
it did exactly that. This speaks the actual protocol end to end, so a pass means
a player would get in.

Run it from inside the east-west group, where the names resolve:

    ix shell hyperion-game -- sh -c 'python3 /tmp/mcping.py hyperion-proxy-0.ix.internal 25565'

Against the game server's own port it is expected to *fail*, with a TLS alert
rather than a status: that port wants a client certificate, which is the whole
reason the game server has no public address.
"""
from __future__ import annotations

import json
import socket
import struct
import sys

# Handshake state, packet 0x00. The intent byte is 1 for status, which is what a
# client sends to draw one line in its server list.
_HANDSHAKE = 0x00
_INTENT_STATUS = 1
# Any recent protocol number does; a server answering status does not gate on it.
_PROTOCOL = 776


def _varint(value: int) -> bytes:
    """Encode an int as Minecraft's base-128 varint, low group first."""
    out = b""
    while True:
        group = value & 0x7F
        value >>= 7
        out += bytes([group | (0x80 if value else 0)])
        if not value:
            return out


def _read_varint(sock: socket.socket) -> int:
    """Read one varint. Raise on a closed connection rather than returning 0."""
    value = 0
    for shift in range(0, 35, 7):
        chunk = sock.recv(1)
        if not chunk:
            msg = "connection closed while reading a varint"
            raise EOFError(msg)
        value |= (chunk[0] & 0x7F) << shift
        if not chunk[0] & 0x80:
            return value
    msg = "varint longer than five bytes"
    raise ValueError(msg)


def _read_exactly(sock: socket.socket, count: int) -> bytes:
    """Read exactly count bytes, or raise saying how far it got."""
    buf = b""
    while len(buf) < count:
        chunk = sock.recv(count - len(buf))
        if not chunk:
            msg = f"connection closed after {len(buf)} of {count} bytes"
            raise EOFError(msg)
        buf += chunk
    return buf


def status(host: str, port: int, timeout: float = 10.0) -> dict[str, object]:
    """Return the server's status document, or raise."""
    with socket.create_connection((host, port), timeout=timeout) as sock:
        # The address the client used goes in the packet. A proxy that routes
        # virtual hosts reads exactly this field, so sending the real name keeps
        # the test honest against one.
        name = host.encode()
        handshake = (
            bytes([_HANDSHAKE])
            + _varint(_PROTOCOL)
            + _varint(len(name))
            + name
            + struct.pack(">H", port)
            + _varint(_INTENT_STATUS)
        )
        sock.sendall(_varint(len(handshake)) + handshake)
        sock.sendall(_varint(1) + b"\x00")

        _read_varint(sock)  # packet length, unused: the string carries its own
        packet_id = _read_varint(sock)
        if packet_id != 0:
            msg = f"expected status response (packet 0), got packet {packet_id}"
            raise ValueError(msg)
        body = _read_exactly(sock, _read_varint(sock))
    return json.loads(body.decode("utf-8"))


def main() -> int:
    """Print the status of the server named by argv, or usage on argv error."""
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} <host> <port>", file=sys.stderr)
        return 2
    doc = status(sys.argv[1], int(sys.argv[2]))
    description = doc.get("description")
    # Servers send either a bare string or a chat component; show both readably.
    if isinstance(description, dict):
        description = description.get("text", description)
    print(f"version:     {doc.get('version')}")
    print(f"players:     {doc.get('players')}")
    print(f"description: {description}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
