#!/usr/bin/env python3
"""Prove tab completion works, on the wire, against a running smash server.

A player who types a slash and sees nothing cannot tell which of two mechanisms
failed, and neither can a unit test. Both are checked here, in the order a
client meets them.

  * **the command graph** (`ClientboundCommandsPacket`) is what makes a slash
    list the server's commands at all, and what tells the client that an
    argument exists and how to parse it. It is also what marks an argument
    `minecraft:ask_server`, which is the only reason a vanilla client ever
    sends a suggestion request. One packet carries all of that and one thing
    sends it, an `OnSet Group` observer in `hyperion-permission` that fires
    when the player enters play. That is a load-bearing accident: a command
    graph is not a permissions concern, the observer's own comment argues only
    about when it is safe to send, and nothing but this gate notices if it
    stops firing. Claim one below is that guard.
  * **command suggestions** (`ServerboundCommandSuggestionPacket` and its
    reply) is what fills the popup once it does ask. This is the half that was
    broken: the old handler could only answer from a clap `ValueEnum`, so
    `/kit `, whose argument is a `Vec<String>`, matched nothing and sent no
    reply at all.

The assertions worth having are the ones with two independent sources. What
`/kit ` offers is compared against what `/kits` prints, so neither side is a
constant in this file: both are the live kit roster, reached by two different
paths through the server. What `/perms set <player> ` offers is compared
against the values clap names in its own parse error, which is the same clap
definition the graph was derived from, read back out through the chat channel
rather than the completion one.
"""

from __future__ import annotations

import argparse
import importlib.util
import pathlib
import re
import struct
import sys
import time

TOOLS = pathlib.Path(__file__).resolve().parent
ROOT = TOOLS.parent


def _load_client():
    """Import `client-26.2.py`, whose file name is not a Python identifier."""
    path = TOOLS / "client-26.2.py"
    spec = importlib.util.spec_from_file_location("client_26_2", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


base = _load_client()
var_int = base.var_int
take_var_int = base.take_var_int
mc_string = base.mc_string
take_string = base.take_string

# Serverbound play ids, from
# crates/hyperion-minecraft-proto/src/generated/packet_id.rs.
C2S_ACCEPT_TELEPORTATION = 0x00
C2S_CHAT_COMMAND = 0x07
C2S_COMMAND_SUGGESTION = 0x0F
C2S_KEEP_ALIVE = 0x1C

# Clientbound play ids this file decodes.
S2C_COMMAND_SUGGESTIONS = 0x0F
S2C_COMMANDS = 0x10
S2C_DISCONNECT = 0x20
S2C_KEEP_ALIVE = 0x2C
S2C_LOGIN = 0x31
S2C_PLAYER_POSITION = 0x48
S2C_SYSTEM_CHAT = 0x79

# `ClientboundCommandsPacket.NodeStub`, from
# crates/hyperion-minecraft-proto/src/packets/play/player.rs.
MASK_TYPE = 3
FLAG_REDIRECT = 8
FLAG_CUSTOM_SUGGESTIONS = 16

# The provider that makes a client ask the server rather than answer from the
# graph. Every hyperion argument is server state, so every one uses it.
ASK_SERVER = "minecraft:ask_server"

# `brigadier:string`'s one property, `StringArgumentKind`.
STRING_KINDS = {0: "single_word", 1: "quotable_phrase", 2: "greedy_phrase"}

# Minecraft's section-sign colour codes, which a client strips before display.
COLOUR = re.compile("§.")

# `KitsCommand` writes each kit as its name in yellow and then its blurb in
# grey, so the colour change is where the name ends. Matched on the raw line
# rather than the stripped one because the separator between them is the only
# thing that says which is which.
KIT_LINE = re.compile("§e(.+?)§7")

# clap's own account of what an argument accepts, from the error it raises when
# handed something else.
POSSIBLE_VALUES = re.compile(r"\[possible values: ([^\]]*)\]")


# --- the command graph, decoded ---------------------------------------------


def argument_type_names():
    """`minecraft:command_argument_type`, in network id order.

    Read out of `protocol.json` rather than listed here, because the ids are
    positions in that registry and a protocol bump moves them. A parser named
    wrongly would let a wrong-parser bug pass.
    """
    names = base.registry_entries("minecraft:command_argument_type")
    if "brigadier:string" not in names:
        raise SystemExit(
            "minecraft:command_argument_type in %s has %d entries and no "
            "brigadier:string, so this read the table wrong"
            % (base.PROTOCOL_JSON, len(names))
        )
    return names


def take_argument_type(payload, offset, names):
    """An argument type id and whatever properties it carries.

    The thirteen types that carry properties are spelled out because skipping
    them wrongly does not fail here: it reads the next node's fields as this
    one's and produces a plausible tree that is not the one the server sent.
    """
    raw, offset = take_var_int(payload, offset)
    if raw >= len(names):
        raise SystemExit("argument type id %d is not in this protocol version" % raw)
    name = names[raw]

    if name == "brigadier:string":
        kind, offset = take_var_int(payload, offset)
        return "%s/%s" % (name, STRING_KINDS.get(kind, kind)), offset
    if name in ("brigadier:float", "brigadier:double"):
        flags = payload[offset]
        offset += 1
        width = 4 if name.endswith("float") else 8
        return name, offset + width * (bool(flags & 1) + bool(flags & 2))
    if name in ("brigadier:integer", "brigadier:long"):
        flags = payload[offset]
        offset += 1
        width = 4 if name.endswith("integer") else 8
        return name, offset + width * (bool(flags & 1) + bool(flags & 2))
    if name in ("minecraft:entity", "minecraft:score_holder"):
        return name, offset + 1
    if name == "minecraft:time":
        return name, offset + 4
    if name in (
        "minecraft:resource_or_tag",
        "minecraft:resource_or_tag_key",
        "minecraft:resource",
        "minecraft:resource_key",
        "minecraft:resource_selector",
    ):
        _registry, offset = take_string(payload, offset)
        return name, offset
    # The other forty-four are a `SingletonArgumentInfo` and write nothing.
    return name, offset


class Node:
    def __init__(self, kind, name, parser, suggests, children):
        self.kind = kind
        self.name = name
        self.parser = parser
        self.suggests = suggests
        self.children = children


def decode_commands(payload, names):
    """`ClientboundCommandsPacket` into nodes, root first."""
    count, offset = take_var_int(payload)
    nodes = []
    for _ in range(count):
        flags = payload[offset]
        offset += 1
        child_count, offset = take_var_int(payload, offset)
        children = []
        for _ in range(child_count):
            child, offset = take_var_int(payload, offset)
            children.append(child)
        if flags & FLAG_REDIRECT:
            _redirect, offset = take_var_int(payload, offset)
        kind = flags & MASK_TYPE
        name = None
        parser = None
        suggests = None
        if kind == 1:
            name, offset = take_string(payload, offset)
        elif kind == 2:
            name, offset = take_string(payload, offset)
            parser, offset = take_argument_type(payload, offset, names)
            if flags & FLAG_CUSTOM_SUGGESTIONS:
                suggests, offset = take_string(payload, offset)
        nodes.append(Node(kind, name, parser, suggests, children))

    root, offset = take_var_int(payload, offset)
    if offset != len(payload):
        raise SystemExit(
            "Commands has %d trailing byte(s), so this decoded it wrong and "
            "every claim below would be about a tree nobody sent"
            % (len(payload) - offset)
        )
    if root != 0 or not nodes or nodes[root].kind != 0:
        raise SystemExit("Commands root index %d is not a root node" % root)
    for index, node in enumerate(nodes):
        for child in node.children:
            if not 0 <= child < len(nodes):
                raise SystemExit(
                    "node %d points at %d, which is not a node" % (index, child)
                )
    return nodes


def path_of(nodes, words):
    """Follow literal names from the root and return the node they reach."""
    at = 0
    for word in words:
        for child in nodes[at].children:
            if nodes[child].kind == 1 and nodes[child].name == word:
                at = child
                break
        else:
            return None
    return nodes[at]


def arguments_under(nodes, node):
    """The argument nodes directly under `node`."""
    return [nodes[child] for child in node.children if nodes[child].kind == 2]


# --- the client -------------------------------------------------------------


def strip_colour(text):
    return COLOUR.sub("", text)


def take_nbt_string(payload, offset):
    """A network-NBT tag that is a bare string, which is every line here.

    `Component::text(..).to_tag()` collapses a component with no style and no
    children to `Tag::String`, so a full NBT reader would be dead weight.
    """
    kind = payload[offset]
    offset += 1
    if kind != 0x08:
        raise SystemExit("chat carried NBT tag type 0x%02X, not a string" % kind)
    (length,) = struct.unpack(">H", payload[offset : offset + 2])
    offset += 2
    return payload[offset : offset + length].decode("utf-8", "replace"), offset + length


class Suggestion:
    """One `ClientboundCommandSuggestionsPacket`."""

    def __init__(self, start, length, entries):
        self.start = start
        self.length = length
        self.entries = entries

    def replaced(self, text):
        """The span of `text` a client overwrites when one is accepted."""
        return text[self.start : self.start + self.length]


class CompletionClient(base.Client):
    """One scripted player that asks for completions and reads the answers."""

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, lambda line: None)
        self.started = started
        self.log = self._log
        self.arg_types = argument_type_names()
        self.graph = None
        # Raw, colour codes and all: the codes are what say where a kit's name
        # ends and its blurb begins.
        self.chat = []
        self.answers = {}
        self.next_id = 1

    def _log(self, line):
        print(
            "[%6.1fs] [%s] %s" % (time.time() - self.started, self.name, line),
            flush=True,
        )

    # --- acting ---

    def run_command(self, text):
        self.log("-> /%s" % text)
        self.send(C2S_CHAT_COMMAND, mc_string(text))

    def ask(self, text):
        """Send a suggestion request and return the id its reply will carry."""
        identifier = self.next_id
        self.next_id += 1
        self.send(C2S_COMMAND_SUGGESTION, var_int(identifier) + mc_string(text))
        self.log("-> CommandSuggestion id=%d %r" % (identifier, text))
        return identifier

    # --- reading ---

    def pump(self, until, seconds, what):
        """Read packets until `until()` holds, or fail saying what was awaited."""
        deadline = time.time() + seconds
        while not until():
            if time.time() >= deadline:
                raise SystemExit("timed out after %.0fs waiting for %s" % (seconds, what))
            packet_id, payload = self.recv()
            self.handle(packet_id, payload)

    def handle(self, packet_id, payload):
        if packet_id == S2C_COMMANDS:
            try:
                self.graph = decode_commands(payload, self.arg_types)
            except Exception:
                self.log("Commands payload: %s" % payload.hex())
                raise
            self.log(
                "<- Commands %d nodes, %d commands at the root"
                % (len(self.graph), len(self.graph[0].children))
            )
        elif packet_id == S2C_COMMAND_SUGGESTIONS:
            self.answers.update([self.decode_suggestions(payload)])
        elif packet_id == S2C_SYSTEM_CHAT:
            text, _ = take_nbt_string(payload, 0)
            self.chat.extend(text.split("\n"))
            self.log("<- chat: %s" % strip_colour(text).replace("\n", " | ")[:200])
        elif packet_id == S2C_PLAYER_POSITION:
            teleport_id, _ = take_var_int(payload)
            self.send(C2S_ACCEPT_TELEPORTATION, var_int(teleport_id))
        elif packet_id == S2C_KEEP_ALIVE:
            self.send(C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == S2C_LOGIN:
            self.entity_id = struct.unpack(">i", payload[:4])[0]
            self.joined = True
            self.log("** in the world ** entity_id=%d" % self.entity_id)
        elif packet_id == S2C_DISCONNECT:
            raise SystemExit("disconnected: %s" % payload[:120].hex())

    def decode_suggestions(self, payload):
        identifier, offset = take_var_int(payload)
        start, offset = take_var_int(payload, offset)
        length, offset = take_var_int(payload, offset)
        count, offset = take_var_int(payload, offset)
        entries = []
        for _ in range(count):
            text, offset = take_string(payload, offset)
            has_tooltip = payload[offset]
            offset += 1
            if has_tooltip:
                _tooltip, offset = take_nbt_string(payload, offset)
            entries.append(text)
        if offset != len(payload):
            raise SystemExit(
                "CommandSuggestions has %d trailing byte(s)" % (len(payload) - offset)
            )
        self.log(
            "<- CommandSuggestions id=%d start=%d length=%d %s"
            % (identifier, start, length, entries)
        )
        return identifier, Suggestion(start, length, entries)

    def suggestions_for(self, text, seconds=20.0):
        """Ask, wait for the reply, and hand back the whole response."""
        identifier = self.ask(text)
        self.pump(
            lambda: identifier in self.answers,
            seconds,
            "a suggestion response for %r" % text,
        )
        return self.answers[identifier]

    def chat_after(self, command, until, seconds=20.0):
        """Run a command and read chat until `until(new_lines)` is satisfied."""
        mark = len(self.chat)
        self.run_command(command)
        self.pump(lambda: until(self.chat[mark:]), seconds, "the reply to /%s" % command)
        return self.chat[mark:]


# --- the claims -------------------------------------------------------------


class Check:
    """The claims this run makes, and the evidence for each."""

    ORDER = [
        "a joining player is sent the command graph",
        "an argument tells the client to ask the server",
        "tab on /kit offers the live kit roster",
        "a typed prefix narrows the offer and names the span it replaces",
        "a kit name with a space is completed as one value",
        "an argument whose clap type enumerates itself needs no declaration",
        "a player argument offers whoever is connected",
    ]

    def __init__(self, started):
        self.started = started
        self.proof = {claim: None for claim in self.ORDER}

    def log(self, line):
        print("[%6.1fs] %s" % (time.time() - self.started, line), flush=True)

    def prove(self, claim, evidence):
        self.proof[claim] = evidence
        self.log("PROVED %s: %s" % (claim, evidence))

    def report(self):
        print("")
        print("=" * 78)
        unproved = [claim for claim, evidence in self.proof.items() if evidence is None]
        for claim in self.ORDER:
            evidence = self.proof[claim]
            print("  %-4s %s" % ("ok" if evidence else "MISS", claim))
            if evidence:
                print("       %s" % evidence)
        print("=" * 78)
        if unproved:
            print(
                "RESULT: %d claim(s) unproved: %s" % (len(unproved), "; ".join(unproved))
            )
            return 1
        print("RESULT: every completion claim held")
        return 0


def kits_from_listing(lines):
    """The kit roster as `/kits` prints it.

    `KitsCommand` walks the same prefabs `/kit ` completes from, but reaches
    them through `kit::registry` rather than through the `(Suggests, Kit)`
    relation, so the two agreeing is two paths agreeing rather than one path
    asked twice.
    """
    return [match.group(1) for line in lines for match in [KIT_LINE.search(line)] if match]


def run(args):
    started = time.time()
    check = Check(started)

    client = CompletionClient(args.host, args.port, args.name, started)
    client.handshake(args.host, args.port, 2)
    client.login()
    client.configuration()
    client.log("configuration acknowledged")

    # 1. The graph arrives without anybody asking for it.
    client.pump(
        lambda: client.graph is not None and client.joined,
        args.timeout,
        "the command graph and the Login packet",
    )
    graph = client.graph
    literals = sorted(
        graph[child].name for child in graph[0].children if graph[child].kind == 1
    )
    for wanted in ("kit", "kits", "perms"):
        if wanted not in literals:
            raise SystemExit(
                "the command graph has no /%s, only %s. A client that never "
                "receives a command cannot complete it." % (wanted, literals)
            )
    check.prove(
        "a joining player is sent the command graph",
        "%d nodes arrived unasked for, listing %d commands at the root: %s"
        % (len(graph), len(literals), ", ".join(literals)),
    )

    # 2. And its arguments are marked ask_server, which is the only reason a
    #    vanilla client ever sends a suggestion request at all.
    kit_args = arguments_under(graph, path_of(graph, ["kit"]))
    if len(kit_args) != 1:
        raise SystemExit("/kit has %d argument nodes, expected one" % len(kit_args))
    name_arg = kit_args[0]
    if name_arg.parser != "brigadier:string/greedy_phrase":
        raise SystemExit(
            "/kit <%s> is %s. A kit name with a space in it needs a greedy "
            "string, or the client sends two arguments" % (name_arg.name, name_arg.parser)
        )
    if name_arg.suggests != ASK_SERVER:
        raise SystemExit(
            "/kit <%s> names suggestion provider %r, so a vanilla client would "
            "answer out of the graph and never ask" % (name_arg.name, name_arg.suggests)
        )

    set_node = path_of(graph, ["perms", "set"])
    if set_node is None:
        raise SystemExit("the graph has no /perms set, only %s" % literals)
    set_args = arguments_under(graph, set_node)
    if len(set_args) != 1 or set_args[0].suggests != ASK_SERVER:
        raise SystemExit("/perms set does not lead to one ask_server argument")
    check.prove(
        "an argument tells the client to ask the server",
        "/kit <%s> is %s with provider %s, and /perms set <%s> is %s with the "
        "same provider"
        % (
            name_arg.name,
            name_arg.parser,
            name_arg.suggests,
            set_args[0].name,
            set_args[0].parser,
        ),
    )

    # 3. What tab offers, against what the game says its kits are.
    listing = client.chat_after("kits", lambda lines: len(kits_from_listing(lines)) > 1)
    roster = kits_from_listing(listing)

    offered = client.suggestions_for("/kit ")
    if sorted(offered.entries) != sorted(roster):
        raise SystemExit(
            "tab on '/kit ' offered %r but /kits lists %r. The completions and "
            "the game disagree about what a kit is."
            % (sorted(offered.entries), sorted(roster))
        )
    if offered.start != len("/kit ") or offered.length != 0:
        raise SystemExit(
            "'/kit ' would replace %r, which is not the empty span after the space"
            % offered.replaced("/kit ")
        )
    check.prove(
        "tab on /kit offers the live kit roster",
        "the %d names tab offers are exactly the %d /kits prints (%s), reached "
        "through the (Suggests, Kit) relation rather than a second list"
        % (len(offered.entries), len(roster), ", ".join(sorted(roster))),
    )

    # 4. A prefix narrows it, and the span the client replaces is the prefix.
    subject = sorted(roster)[0]
    typed = "/kit " + subject[:2]
    narrowed = client.suggestions_for(typed)
    expected = sorted(k for k in roster if k.lower().startswith(subject[:2].lower()))
    if sorted(narrowed.entries) != expected:
        raise SystemExit(
            "tab on %r offered %r, expected %r"
            % (typed, sorted(narrowed.entries), expected)
        )
    if narrowed.replaced(typed) != subject[:2]:
        raise SystemExit(
            "tab on %r would replace %r rather than the %r that was typed"
            % (typed, narrowed.replaced(typed), subject[:2])
        )
    check.prove(
        "a typed prefix narrows the offer and names the span it replaces",
        "%r offered %s and marked %r for replacement, so accepting one leaves "
        "the line well formed" % (typed, expected, narrowed.replaced(typed)),
    )

    # 5. The greedy argument. Half the roster has a space in its name, and a
    #    client that replaces only the last word turns a half-typed "Iron Gol"
    #    into "Iron Iron Golem".
    spaced = next((kit for kit in sorted(roster) if " " in kit), None)
    if spaced is None:
        raise SystemExit("no kit has a space in its name, so nothing tests greediness")
    head, tail = spaced.split(" ", 1)
    typed = "/kit %s %s" % (head, tail[:2])
    greedy = client.suggestions_for(typed)
    if spaced not in greedy.entries:
        raise SystemExit(
            "tab on %r offered %r, which does not include %r"
            % (typed, greedy.entries, spaced)
        )
    if greedy.replaced(typed) != "%s %s" % (head, tail[:2]):
        raise SystemExit(
            "tab on %r would replace %r rather than the whole half-typed name, "
            "so accepting it would duplicate the first word"
            % (typed, greedy.replaced(typed))
        )
    check.prove(
        "a kit name with a space is completed as one value",
        "%r offered %r and marked %r for replacement, both words of it"
        % (typed, spaced, greedy.replaced(typed)),
    )

    # 6. The `ValueEnum` path, checked against clap's own account of what the
    #    argument accepts. Nothing in the server declares this one's values.
    refusal = client.chat_after(
        "perms set %s definitely-not-a-group" % args.name,
        lambda lines: any(POSSIBLE_VALUES.search(strip_colour(line)) for line in lines),
    )
    listed = [
        POSSIBLE_VALUES.search(strip_colour(line)).group(1)
        for line in refusal
        if POSSIBLE_VALUES.search(strip_colour(line))
    ][0]
    accepted = sorted(re.findall(r"[a-z][a-z0-9-]*", listed))

    groups = client.suggestions_for("/perms set %s " % args.name)
    if sorted(groups.entries) != accepted:
        raise SystemExit(
            "tab on '/perms set <player> ' offered %r but clap says the "
            "argument accepts %r" % (sorted(groups.entries), accepted)
        )
    check.prove(
        "an argument whose clap type enumerates itself needs no declaration",
        "tab offered %s, exactly what clap says it accepts, and no line in the "
        "server declares it: registration read the ValueEnum" % ", ".join(accepted),
    )

    # 7. The second live source: a different tag, owned by a different crate,
    #    reached through the same relation.
    #    Both spellings, because `set` and `get` are two nodes carrying the one
    #    clap id `player`: a declaration that wired only the first would pass
    #    on `set` and offer nothing on `get`.
    for command in ("/perms set ", "/perms get "):
        players = client.suggestions_for(command)
        if players.entries != [args.name]:
            raise SystemExit(
                "tab on %r offered %r, but %s is the only player connected"
                % (command, players.entries, args.name)
            )
    check.prove(
        "a player argument offers whoever is connected",
        "the one connected player, %s, is the one name offered under both "
        "/perms set and /perms get, queried through (Suggests, Player) at the "
        "moment tab was pressed" % args.name,
    )

    return check.report()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument("--name", default="Completer")
    parser.add_argument("--timeout", type=float, default=60.0)
    args = parser.parse_args()
    sys.exit(run(args))


if __name__ == "__main__":
    main()
