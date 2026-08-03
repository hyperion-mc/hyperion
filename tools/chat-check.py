#!/usr/bin/env python3
"""Player chat, proved the only way it can be: two clients, one talking.

smash decoded chat and broadcast nothing. `PacketId::Chat` was routed to a
handler, the handler pushed `event::ChatMessage`, and no system drained the
queue, so a message a player typed reached exactly nobody. Nothing in the crate
could see it -- there was no code to test -- and the only shape of evidence that
distinguishes "wired up" from "decoded and dropped" is a second connection
receiving what the first one said.

So this joins two clients and checks four things:

  1. **The speaker hears themselves.** Vanilla echoes your own message back
     from the server rather than drawing it locally, and a broadcast that
     skipped the sender would look right to them and be wrong.
  2. **The other client hears it**, in the vanilla shape `<Name> message`.
     This is the assertion that is red on an unpatched tree: no `SystemChat`
     carrying the message arrives at all.
  3. **A section sign a player typed is not a formatting code.** The client
     renders a literal `SystemChat` string through `StringDecomposer`, which
     applies legacy `§` codes as it goes, so `§k` from a bot scrambles the
     glyphs and `§4[Server]` paints a fake server notice. Both must arrive with
     the sign gone and the rest of the text intact.
  4. **A whitespace-only message is dropped**, and dropped rather than
     broadcast as `<Name>   `.

Assertions 1 to 3 are fail-then-pass by construction and have been watched
failing: with the module's import removed 1, 2 and 3 go red, and with the
section sign left in only 3 does. Assertion 4 is not -- a server that
broadcasts nothing satisfies "nothing was broadcast" -- which is why the
message after it has to arrive for the run to pass at all.

Exits non-zero on anything that is not true, after printing everything it saw.
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

var_int = match.base.var_int
mc_string = match.base.mc_string
take_nbt_string = match.take_nbt_string

# crates/hyperion-minecraft-proto/src/generated/packet_id.rs, protocol 776:
# `minecraft:chat` serverbound. Distinct from `chat_command` (7), which is the
# one every other gate here sends.
C2S_CHAT = 0x09

# `LastSeenMessagesTracker.window` is 20 wide, so the acknowledged bitset is a
# fixed three bytes. See `hyperion::simulation::packet::serverbound`.
LAST_SEEN_ACKNOWLEDGED_BYTES = 3


def chat_packet(message):
    """`ServerboundChatPacket`, unsigned, acknowledging nothing.

    Layout from `ServerboundChatPacket#STREAM_CODEC`: the message, the client's
    clock, the signature salt, an optional 256-byte signature, then the last
    seen window. This server does not verify signatures -- `chat_ack` and
    `chat_session_update` are both routed to `Route::Ignore` -- so the
    signature is absent and the salt is zero, which is what a client with no
    chat session sends.
    """
    return (
        mc_string(message)
        + struct.pack(">qq", int(time.time() * 1000), 0)
        + b"\x00"
        + var_int(0)
        + b"\x00" * LAST_SEEN_ACKNOWLEDGED_BYTES
        + b"\x00"
    )


class Talker(match.MatchClient):
    """A scripted player that records every chat line it is sent."""

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, started)
        self.joined = False
        self.chats = []

    def say(self, message):
        self.log("-> chat %r" % message)
        self.send(C2S_CHAT, chat_packet(message))

    def absorb(self, packet_id, payload):
        if packet_id == match.S2C_LOGIN:
            self.joined = True
            self.log("** in the world **")
        elif packet_id == match.S2C_PLAYER_POSITION:
            teleport_id, offset = match.base.take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            self.position = (x, y, z)
            self.send(match.C2S_ACCEPT_TELEPORTATION, var_int(teleport_id))
        elif packet_id == match.S2C_KEEP_ALIVE:
            self.send(match.C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == match.S2C_SYSTEM_CHAT:
            text, _ = take_nbt_string(payload, 0)
            self.chats.append(text)
            self.log("<- chat %r" % text)
        elif packet_id == match.S2C_DISCONNECT:
            text, _ = take_nbt_string(payload, 0)
            self.log("<- DISCONNECTED: %s" % text)
            self.alive = False


def pump(clients, seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        for client in clients:
            if not client.alive:
                continue
            for packet_id, payload in client.drain():
                client.absorb(packet_id, payload)
            if client.joined:
                client.repeat_position()
        time.sleep(0.01)


def wait_until(clients, predicate, seconds, what):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        pump(clients, 0.05)
        if predicate():
            return True
    print("TIMEOUT waiting for %s" % what, file=sys.stderr)
    return False


def connect(host, port, name, started):
    client = Talker(host, port, name, started)
    client.handshake(host, port, 2)
    client.login()
    client.configuration()
    client.enter_play()
    return client


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument("--speaker", default="Alice")
    parser.add_argument("--listener", default="Bob")
    args = parser.parse_args()

    started = time.time()
    failures = []

    def check(ok, message):
        print("%s %s" % ("PASS" if ok else "FAIL", message), flush=True)
        if not ok:
            failures.append(message)

    speaker = connect(args.host, args.port, args.speaker, started)
    listener = connect(args.host, args.port, args.listener, started)
    clients = [speaker, listener]

    if not wait_until(clients, lambda: all(c.joined for c in clients), 60.0, "both clients"):
        print("RESULT: failure (never joined)", flush=True)
        return 1

    # Anything the join path says -- the build stamp, the lobby -- lands before
    # the first message and is not what any assertion below is about.
    pump(clients, 2.0)
    for client in clients:
        client.chats.clear()

    def expect(sent, wanted, note, settle=5.0):
        """`sent` is typed by the speaker; `wanted` must reach both clients."""
        speaker.say(sent)
        arrived = wait_until(
            clients,
            lambda: all(wanted in c.chats for c in clients),
            settle,
            "%r on both clients" % wanted,
        )
        check(
            arrived,
            "%s: sent %r, both clients receive %r (speaker saw %r, listener saw %r)"
            % (note, sent, wanted, speaker.chats, listener.chats),
        )
        return arrived

    # The whole feature. An unpatched tree fails here and only here matters.
    hello = "hello from the other side"
    expect(
        hello,
        "<%s> %s" % (args.speaker, hello),
        "a message reaches every player in the vanilla shape",
    )

    # The speaker's own copy, called out separately because a broadcast that
    # excluded the sender would still pass a listener-only check and would look
    # wrong to the person typing.
    check(
        "<%s> %s" % (args.speaker, hello) in speaker.chats,
        "the speaker is sent their own message rather than drawing it locally",
    )

    # Formatting injection. `§k` is the obfuscate code and `§4` is dark red; a
    # client renders both out of a literal string, so leaving them in lets a bot
    # scramble its own text and impersonate a server notice.
    expect(
        "§4[Server] restarting §kNOW",
        "<%s> 4[Server] restarting kNOW" % args.speaker,
        "a section sign a player typed is stripped, and nothing else is",
    )

    # A message that is only whitespace. Checked by sending a real one after it
    # and requiring that the real one is the next thing anybody sees, so this
    # cannot pass by the server simply being slow.
    before = list(listener.chats)
    speaker.say("   ")
    pump(clients, 2.0)
    blank = [line for line in listener.chats[len(before) :]]
    check(
        not any(line.startswith("<%s>" % args.speaker) for line in blank),
        "a whitespace-only message is dropped rather than broadcast (saw %r)" % blank,
    )
    expect(
        "still here",
        "<%s> still here" % args.speaker,
        "chat still works after a dropped message",
    )

    print(
        "RESULT: %s" % ("success" if not failures else "failure (%d)" % len(failures)),
        flush=True,
    )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
