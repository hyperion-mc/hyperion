#!/usr/bin/env python3
"""The operator console, driven the way an operator drives it.

Everything the console claims needs both halves present at once: a real client
in the world and a real HTTP client on the port. Nothing in the crate can test
that pairing, because the crate has neither.

So this joins one scripted player, then talks to the console over HTTP and
checks the two directions meet:

  1. **The page is served and the port is not open.** `GET /` answers, and
     every path that carries data refuses a request with no bearer token. This
     is the assertion an operator's safety rests on, so it is first.
  2. **What a player types reaches the web.** The client sends a chat packet
     and the line has to arrive on `/events` in vanilla's `<Name> message`
     shape.
  3. **What the console says reaches the game.** `POST /say` has to arrive at
     the client as a `SystemChat`, attributed to the console, and has to appear
     on the feed too, so the operator sees their own message land.
  4. **A command runs through the real dispatch and answers back.** `POST
     /command` with a command nobody registered has to come back on the feed as
     `hyperion_command`'s own "Available commands" reply. That reply is written
     by the engine, unicast to the caller's connection, and the caller here has
     no socket -- so it arriving at all is the whole of the virtual-connection
     mechanism working end to end. A second command asks for a reply too big to
     fit under the compression threshold, because the short one and the long
     one take different branches of the console's decoder and only one of them
     is otherwise exercised.
  5. **The live state is live.** `/state` names the joined player and carries a
     tick rate.

Every assertion here has been watched failing against a live server, which is
the only reason to believe any of them. What was broken, and what went red:

  * the decoder handed the wrong compression threshold -- all three reply
    assertions, and nothing else. Worth stating plainly: that failure is
    **silent**. The decoder reads the `data_len` varint as a packet id, decides
    the frame is not chat, and drops it; the server logs nothing at any level.
    No unit test can see it either, because the crate has no server to unicast
    through. This file is the only thing between that bug and an operator.
  * `strip_formatting` removed from the chat tap -- both spoof assertions, with
    `<Name> \u00a74[Server] restarting` arriving on the feed exactly as the
    attack intends. The in-game line stayed clean, which is the point: the two
    paths trust the text differently.
  * `id="log"` renamed in `console.html` -- the page assertion, naming the
    marker it could not find. A title-substring check stayed green through the
    same edit, which is why this one keys on the ids the page's own script
    looks up.
  * the right token offered where a refusal is demanded -- every refusal
    assertion. `/events` needed the token in the query rather than a header to
    go red at all, since that is the only way it authenticates.

Exits non-zero on anything that is not true, after printing everything it saw.
"""

import argparse
import importlib.util
import json
import pathlib
import struct
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

TOOLS = pathlib.Path(__file__).resolve().parent


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


match = _load("smash_match", "smash-match.py")
chat_check = _load("chat_check", "chat-check.py")

take_nbt_string = match.take_nbt_string
var_int = match.base.var_int

# `hyperion_web_console::module::CONSOLE_NAME`, as the console prefixes its own
# messages in game.
CONSOLE_PREFIX = "[Console]"

# `hyperion::Global`'s own, set in `crates/hyperion/src/lib.rs`. The encoder
# compresses a packet whose body is longer than this and writes a `data_len` of
# zero otherwise, so a reply either side of it takes a different branch of the
# console's decoder.
COMPRESSION_THRESHOLD = 256


class Watcher(chat_check.Talker):
    """The scripted player, reused from the chat gate: it already knows how to
    send a serverbound chat packet and record every `SystemChat` it is sent."""


class Feed(threading.Thread):
    """`/events`, read the way a browser reads it.

    A thread because SSE is a response that never ends: the read blocks, and
    the rest of this file has a client to pump in the meantime.
    """

    def __init__(self, base, token):
        super().__init__(daemon=True)
        # Encoded the way the page encodes it. `console.html` builds this URL
        # with `encodeURIComponent`, and a gate that interpolates the raw token
        # is not driving the console the way an operator drives it: a standard
        # base64 token contains `+` and `/`, and sending those raw asks the
        # server a different question than the browser ever asks.
        self.url = "%s/events?token=%s" % (base, urllib.parse.quote(token, safe=""))
        self.lines = []
        self.error = None
        self.connected = threading.Event()

    def run(self):
        try:
            with urllib.request.urlopen(self.url, timeout=60) as response:
                self.connected.set()
                for raw in response:
                    text = raw.decode("utf-8", "replace").strip()
                    if not text.startswith("data:"):
                        continue
                    self.lines.append(json.loads(text[5:].strip()))
        except Exception as error:  # noqa: BLE001 - reported, not handled
            self.error = error
            self.connected.set()

    def texts(self, source=None):
        return [
            line["text"]
            for line in list(self.lines)
            if source is None or line["source"] == source
        ]


def request(base, path, token=None, body=None, method=None):
    """One HTTP request, returning (status, body). A refusal is a result here,
    not an exception: half the assertions below are about refusals."""
    url = "%s%s" % (base, path)
    data = body.encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    if token:
        req.add_header("Authorization", "Bearer %s" % token)
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            return response.status, response.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8", "replace")
    except urllib.error.URLError as error:
        return 0, str(error)
    # A read that never finishes is the shape `/events` has -- it is a stream,
    # and a body that ends is the abnormal case. Reaching it here means a path
    # that should have answered and closed did not, and that has to arrive as a
    # status this file can assert on. Uncaught, it ends the run in a traceback
    # partway down, which reports nothing about the checks that never ran.
    except (TimeoutError, OSError) as error:
        return 0, "no complete response: %r" % (error,)


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
    parser.add_argument("--console", default="127.0.0.1:8791")
    parser.add_argument("--token-file", required=True)
    parser.add_argument("--name", default="Watcher")
    args = parser.parse_args()

    token = pathlib.Path(args.token_file).read_text().strip()
    base = "http://%s" % args.console
    started = time.time()
    failures = []

    def check(ok, message):
        print("%s %s" % ("PASS" if ok else "FAIL", message), flush=True)
        if not ok:
            failures.append(message)

    # --- 1. the port is not open -----------------------------------------
    #
    # First, because everything after it assumes a console that is reachable,
    # and a console reachable *without* the token is worse than one that does
    # not work at all.
    status, body = request(base, "/")
    # The ids the page's own script looks up, and the URL it builds. A title
    # or a heading is prose and can be reworded without anything breaking; if
    # any of these three goes missing the page is served and does not work,
    # which is the failure worth catching. See `assets/console.html`.
    missing = [
        marker
        for marker in ('id="log"', 'id="token"', "/events?token=")
        if marker not in body
    ]
    check(
        status == 200 and not missing,
        "GET / serves a page the script can drive (%s, missing %r)" % (status, missing),
    )

    for path, method, payload in (
        ("/events", None, None),
        ("/state", None, None),
        ("/say", "POST", "hello"),
        ("/command", "POST", "perms"),
    ):
        status, _ = request(base, path, body=payload, method=method)
        check(status == 401, "%s without a token is refused (%s)" % (path, status))

    status, _ = request(base, "/state", token="not-the-token")
    check(status == 401, "a wrong token is refused (%s)" % status)

    # --- the world ---------------------------------------------------------
    client = Watcher(args.host, args.port, args.name, started)
    client.handshake(args.host, args.port, 2)
    client.login()
    client.configuration()
    client.enter_play()

    if not wait_until(client, lambda: client.joined, 60.0, "the client to join"):
        print("RESULT: failure (never joined)", flush=True)
        return 1

    feed = Feed(base, token)
    feed.start()
    feed.connected.wait(timeout=10)
    if feed.error is not None:
        print("FAIL /events with a token did not open: %r" % feed.error, flush=True)
        print("RESULT: failure (no event stream)", flush=True)
        return 1
    print("PASS /events with a token opens", flush=True)

    pump(client, 2.0)
    client.chats.clear()

    # --- 2. the game reaches the web --------------------------------------
    spoken = "console gate is watching"
    client.say(spoken)
    wanted = "<%s> %s" % (args.name, spoken)
    arrived = wait_until(
        client,
        lambda: wanted in feed.texts("chat"),
        10.0,
        "%r on the console feed" % wanted,
    )
    check(arrived, "a player's message reaches the web feed as %r (saw %r)" % (wanted, feed.texts("chat")))

    # --- 2b. a player cannot paint their own line -------------------------
    #
    # The page renders section signs, so a message carrying them is a player
    # writing colour into an operator's console: `\u00a74[Server] restarting`
    # arrives looking like the server said it. The engine's own
    # `strip_formatting` runs on the way to the feed, and it takes the sign and
    # leaves everything else, so the code letter stays as an ordinary
    # character. Same rule, and same expected shape, as `chat-check.py`'s third
    # assertion.
    spoof = "\u00a74[Server] restarting \u00a7kNOW"
    client.say(spoof)
    stripped = "<%s> 4[Server] restarting kNOW" % args.name
    spoofed = wait_until(
        client,
        lambda: any("[Server] restarting" in line for line in feed.texts("chat")),
        10.0,
        "the spoof attempt on the console feed",
    )
    painted = [line for line in feed.texts("chat") if "[Server] restarting" in line]
    check(
        spoofed and painted == [stripped],
        "a player's section signs are stripped before the web feed "
        "(wanted %r, saw %r)" % (stripped, painted),
    )
    check(
        all("\u00a7" not in line for line in painted),
        "no section sign a player typed survives onto the feed (saw %r)" % painted,
    )

    # --- 3. the web reaches the game --------------------------------------
    announcement = "the console can talk"
    status, body = request(base, "/say", token=token, body=announcement, method="POST")
    check(status == 202, "POST /say is accepted (%s %s)" % (status, body))

    heard = wait_until(
        client,
        lambda: any(
            CONSOLE_PREFIX in line and announcement in line for line in client.chats
        ),
        10.0,
        "the announcement at the client",
    )
    check(
        heard,
        "a console announcement reaches a real client attributed to the console (saw %r)"
        % client.chats,
    )
    check(
        any(announcement in line for line in feed.texts("console")),
        "the operator sees their own announcement on the feed (saw %r)"
        % feed.texts("console"),
    )

    # --- 4. a command runs, and answers back ------------------------------
    #
    # A command nobody registered, so the reply is `hyperion_command`'s own and
    # does not depend on which event is running. It is unicast to the caller's
    # connection, and the caller is the console, which has no socket -- so this
    # arriving is the virtual connection working.
    status, body = request(
        base, "/command", token=token, body="notacommand", method="POST"
    )
    check(status == 202, "POST /command is accepted (%s %s)" % (status, body))

    replied = wait_until(
        client,
        lambda: any("Available commands" in line for line in feed.texts("reply")),
        10.0,
        "a command reply on the feed",
    )
    check(
        replied,
        "a command's reply comes back to the console over its virtual "
        "connection (replies seen: %r)" % feed.texts("reply"),
    )

    # --- 4b. a reply the encoder compressed --------------------------------
    #
    # The reply above is under two hundred bytes, so the encoder writes it with
    # a `data_len` of zero and the decoder takes it verbatim. That leaves the
    # other half of `FrameDecoder` -- the branch that inflates -- unproven, and
    # a decoder told the wrong threshold reads a compressed frame as a raw one
    # and produces nonsense rather than an error. So this asks for a reply that
    # cannot fit under the threshold and checks it arrives whole.
    #
    # `perms --help` and not a smash command, because the console is engine
    # level and this gate should not depend on which event is running.
    status, body = request(
        base, "/command", token=token, body="perms --help", method="POST"
    )
    check(status == 202, "POST /command (a long one) is accepted (%s %s)" % (status, body))

    long_replies = []

    def long_reply():
        long_replies[:] = [
            text
            for text in feed.texts("reply")
            if len(text.encode("utf-8")) > COMPRESSION_THRESHOLD
        ]
        return bool(long_replies)

    inflated = wait_until(client, long_reply, 10.0, "a reply past the compression threshold")
    check(
        inflated,
        "a reply larger than the %d byte compression threshold reaches the "
        "console, so the decoder's inflate branch works (longest seen: %d bytes)"
        % (
            COMPRESSION_THRESHOLD,
            max((len(text.encode("utf-8")) for text in feed.texts("reply")), default=0),
        ),
    )
    check(
        any("perms" in text for text in long_replies),
        "that reply is the one asked for rather than any long line (saw %r)"
        % [text[:60] for text in long_replies],
    )

    # --- 5. live state -----------------------------------------------------
    status, body = request(base, "/state", token=token)
    check(status == 200, "GET /state with a token answers (%s)" % status)
    state = json.loads(body) if status == 200 else {}
    check(
        any(player["name"] == args.name for player in state.get("players", [])),
        "the roster names the joined player (%r)" % state.get("players"),
    )
    check(
        "tps" in state and "targetTps" in state,
        "the state carries a tick rate against the rate it is paced to (%r)" % state,
    )
    check(
        state.get("targetTps") == 20.0,
        "the target tick rate is the engine's own (%r)" % state.get("targetTps"),
    )

    print(
        "RESULT: %s" % ("success" if not failures else "failure (%d)" % len(failures)),
        flush=True,
    )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
