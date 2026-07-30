#!/usr/bin/env python3
"""The heads-up display, read off the wire.

Everything the server draws on a player's screen is a packet, and three of the
packets this asks about were sent zero times by this server until now:
`ClientboundSetExperiencePacket`, `ClientboundBossEventPacket` and
`ClientboundSetSubtitleTextPacket`. A Rust test can prove the game *decided* to
draw something; only a client can say whether it arrived, under the id a 26.2
client reads, carrying the numbers the game meant.

Two halves, because they need different numbers of players.

  1. One client in the hub. It picks a kit, holds each of three slots in turn,
     and reads the experience bar back: full with no number for an ability that
     is ready, filling from empty with the seconds beside it for one that is
     recharging, empty with no number for a slot holding nothing. It also fires
     the ability and follows the bar all the way back to full, which is the
     claim the whole feature rests on and the one that a bar wired to the wrong
     slot would still pass half of.

  2. Enough clients to fill the lobby, so it runs its shortest countdown
     rather than its sixty second one. That gets the phase machine
     through Countdown, Preparing and into Playing inside twenty seconds, and
     the titles, subtitles and the percentage bar with it.

Exits non-zero on the first thing that is not true, after printing what it saw.
"""

import argparse
import importlib.util
import math
import pathlib
import re
import struct
import sys
import time

TOOLS = pathlib.Path(__file__).resolve().parent

# `§` followed by one character is a legacy colour code, which the server writes
# into chat and a client renders rather than reads.
COLOUR = re.compile("§.")


def uncoloured(text):
    return COLOUR.sub("", text)


def _load(name, filename):
    """Import a sibling tool whose file name is not a Python identifier."""
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


match = _load("smash_match", "smash-match.py")
take_var_int = match.take_var_int
var_int = match.var_int

# crates/hyperion-minecraft-proto/src/generated/packet_id.rs, protocol 776.
S2C_BOSS_EVENT = 0x09
S2C_SET_EXPERIENCE = 0x67
S2C_SET_SUBTITLE_TEXT = 0x70
S2C_SET_TITLES_ANIMATION = 0x73

# `BossEventOperation::type_id`, in ordinal order.
BOSS_ADD = 0
BOSS_OPERATIONS = [
    "add",
    "remove",
    "update_progress",
    "update_name",
    "update_style",
    "update_properties",
]
# `BossBarColor`, as ordinals.
BOSS_COLOURS = ["pink", "blue", "red", "green", "yellow", "purple", "white"]

# `egress::server_load`. Both labels carry the reading and the ceiling it is
# drawn against, which is what makes "the fill is the quotient of the two
# numbers in the label" a thing this file can check rather than a claim.
CPU_LABEL = re.compile(r"CPU (\d+)% of (\d+)%")
MEMORY_LABEL = re.compile(r"MEM (\d+\.\d+) GiB of (\d+\.\d+) GiB")
# `egress::server_load::STEPS`, which is also `hud.rs`'s `METER_STEPS`.
LOAD_STEPS = 64.0

# A kit's abilities take the hotbar slots in the order it declares them, so for
# the Iron Golem slot 2 is Seismic Slam at seven seconds, slot 1 is Iron Hook at
# eight, and slot 5 holds nothing. Named here rather than read off `/abilities`
# because the point of this gate is the *display*, and a fixed cooldown is what
# makes the number beside the bar predictable; `smash-hotbar-e2e` is the gate
# that holds the layout itself.
KIT = "Iron Golem"
FIRED_SLOT = 2
FIRED_COOLDOWN = 7.0
IDLE_SLOT = 1
EMPTY_SLOT = 5

# events/smash/src/module/hud.rs: `METER_STEPS`. One step is the finest the bar
# is allowed to move, so it is also the tolerance every progress check here
# gets.
METER_STEPS = 64.0
STEP = 1.0 / METER_STEPS

# events/smash/src/module/hud.rs: `COUNTDOWN_TITLE_SECONDS`.
COUNTDOWN_DIGITS = ["3", "2", "1"]


class Screen(match.MatchClient):
    """A scripted player that remembers what was drawn on it."""

    def __init__(self, host, port, name, started, uuid=None):
        super().__init__(host, port, name, started, uuid=uuid)
        # Every experience bar, boss bar and title in the order they arrived, so
        # a check can ask about the last one or about the whole sequence.
        self.experience = []
        # What each bar looks like right now, keyed on its id. The client's own
        # `BossHealthOverlay` is exactly this map, so this is the client, and
        # every check about what a player can *see* reads it rather than
        # reading packets.
        self.bar_state = {}
        # One entry per resulting state, in order, so a check written against
        # the old `Add`-every-time server still asks the same question.
        self.bars = []
        # One entry per packet: `(id, operation, fields that actually moved)`.
        # This is the other half, and the only place "an update sends only the
        # update" is visible at all: the states above come out identical
        # whether the server resent the whole bar or moved one field.
        self.bar_ops = []
        self.titles = []
        self.subtitles = []
        self.animations = []
        # Every system chat line, in order. `/perms get` answers in chat and
        # nowhere else, so this is how the seeded group is read back.
        self.chat = []
        # The three title packets in arrival order, which is the only place the
        # ordering they have to arrive in is observable at all.
        self.screen = []

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
            self.send(match.C2S_KEEP_ALIVE, payload[:8])
        elif packet_id == match.S2C_SYSTEM_CHAT:
            text, _ = match.take_nbt_string(payload, 0)
            self.log("<- chat: %s" % text)
            self.chat.append(text)
            if text.startswith("Kit set to "):
                self.kit = text[len("Kit set to ") :].rstrip(".")
        elif packet_id == match.S2C_CONTAINER_SET_SLOT:
            self.absorb_slot(payload)
        elif packet_id == match.S2C_DISCONNECT:
            text, _ = match.take_nbt_string(payload, 0)
            self.log("<- DISCONNECTED: %s" % text)
            self.alive = False
        elif packet_id == S2C_SET_EXPERIENCE:
            (progress,) = struct.unpack(">f", payload[:4])
            level, offset = take_var_int(payload, 4)
            total, _ = take_var_int(payload, offset)
            self.experience.append({"progress": progress, "level": level, "total": total})
            self.log("<- experience bar %.4f full, level %d" % (progress, level))
        elif packet_id == S2C_BOSS_EVENT:
            self.absorb_boss_event(payload)
        elif packet_id == match.S2C_SET_TITLE_TEXT:
            text, _ = match.take_nbt_string(payload, 0)
            self.titles.append(text)
            self.screen.append(("title", text))
            self.log("<- title: %r" % text)
        elif packet_id == S2C_SET_SUBTITLE_TEXT:
            text, _ = match.take_nbt_string(payload, 0)
            self.subtitles.append(text)
            self.screen.append(("subtitle", text))
            self.log("<- subtitle: %r" % text)
        elif packet_id == S2C_SET_TITLES_ANIMATION:
            # Three plain big-endian ints, not varints: the generated
            # `SetTitlesAnimation` carries no `#[proto(varint)]`. Reading them as
            # varints decodes without complaining and reports rubbish, which is
            # why this comment names the layout it was checked against.
            fade_in, stay, fade_out = struct.unpack(">iii", payload[:12])
            self.animations.append((fade_in, stay, fade_out))
            self.screen.append(("times", (fade_in, stay, fade_out)))

    def absorb_boss_event(self, payload):
        """Apply one boss bar operation to this client's own picture of it."""
        uuid, operation, fields = decode_boss_event(payload)
        before = self.bar_state.get(uuid)
        self.bar_ops.append(
            (uuid, operation, moved_fields(before, fields), before is not None)
        )
        if operation == "remove":
            self.bar_state.pop(uuid, None)
            self.log("<- boss bar %s removed" % uuid[:8])
            return
        if operation == "add":
            self.bar_state[uuid] = dict(fields, uuid=uuid)
        elif uuid in self.bar_state:
            self.bar_state[uuid].update(fields)
        else:
            # An update for a bar this client was never given. The server is
            # not allowed to do that, and saying so here beats silently
            # inventing a bar to hang the field on.
            self.log("<- boss bar %s %s with no Add before it" % (uuid[:8], operation))
            return
        bar = dict(self.bar_state[uuid])
        bar["colour"] = colour_name(bar["colour"])
        self.bars.append(bar)
        self.log(
            "<- boss bar %s %r %.3f full, %s"
            % (operation, bar["title"], bar["progress"], bar["colour"])
        )

    def ops_for(self, uuid):
        """Every operation this client was sent for one bar, in order."""
        return [name for bar, name, _, _ in self.bar_ops if bar == uuid]

    def bars_titled(self, pattern):
        """Every id this client holds whose current title matches."""
        return [
            uuid for uuid, bar in self.bar_state.items() if pattern.match(bar["title"])
        ]

    def absorb_slot(self, payload):
        """Only enough of `ContainerSetSlot` to know the hotbar has arrived."""
        _container, offset = take_var_int(payload)
        _state, offset = take_var_int(payload, offset)
        (slot,) = struct.unpack_from(">h", payload, offset)
        offset += 2
        slot -= match.HOTBAR_START_SLOT
        if not 0 <= slot < 9:
            return
        count, offset = take_var_int(payload, offset)
        if count <= 0:
            self.hotbar.pop(slot, None)
        else:
            item, _ = take_var_int(payload, offset)
            self.hotbar[slot] = item

    def carry(self, slot):
        """Hold a slot without using it.

        The distinction this gate exists to check: `use_slot` changes the held
        item *and* right-clicks, so a bar wired to "the last ability you fired"
        would be indistinguishable from one wired to "the slot you are holding"
        if nothing ever only held.
        """
        self.log("-> hold hotbar slot %d" % slot)
        self.send(match.C2S_SET_CARRIED_ITEM, struct.pack(">h", slot))


def decode_boss_event(payload):
    """One `ClientboundBossEventPacket`: which bar, which operation, and the
    fields that operation carries.

    Every operation, not only `Add`. `egress::boss_bar` sends one packet per
    field that actually moved, so a decoder that read `Add` and returned `None`
    for the rest -- which is what this was, when the server only ever sent
    `Add` -- would not fail against the new server. It would report that no
    boss bar ever arrived, which is a silent pass, and it is why the model
    below exists rather than a list of `Add`s.
    """
    uuid = payload[:16].hex()
    operation, offset = take_var_int(payload, 16)
    name = BOSS_OPERATIONS[operation] if operation < len(BOSS_OPERATIONS) else operation
    fields = {}
    if operation == BOSS_ADD:
        fields["title"], offset = match.take_nbt_string(payload, offset)
        (fields["progress"],) = struct.unpack_from(">f", payload, offset)
        offset += 4
        colour, offset = take_var_int(payload, offset)
        fields["colour"] = colour
        fields["overlay"], offset = take_var_int(payload, offset)
        fields["flags"] = payload[offset]
    elif name == "update_progress":
        (fields["progress"],) = struct.unpack_from(">f", payload, offset)
    elif name == "update_name":
        fields["title"], offset = match.take_nbt_string(payload, offset)
    elif name == "update_style":
        fields["colour"], offset = take_var_int(payload, offset)
        fields["overlay"], offset = take_var_int(payload, offset)
    elif name == "update_properties":
        fields["flags"] = payload[offset]
    return uuid, name, fields


def colour_name(ordinal):
    return BOSS_COLOURS[ordinal] if ordinal < len(BOSS_COLOURS) else ordinal


# A bar has four fields, and the protocol has one operation for each. Colour
# and overlay are one field and not two: `UpdateStyle` carries both and there
# is no operation that carries either alone, so "only the changed field" can
# only ever mean "only the changed *pair*" for those.
BOSS_FIELDS = {
    "title": ("title",),
    "progress": ("progress",),
    "style": ("colour", "overlay"),
    "flags": ("flags",),
}


def moved_fields(before, fields):
    """Which of a bar's four fields this packet actually changed.

    `None` for a bar the client did not have, where every field it carries is
    new by definition.
    """
    if before is None:
        return set(BOSS_FIELDS)
    moved = set()
    for name, keys in BOSS_FIELDS.items():
        if any(key in fields and fields[key] != before[key] for key in keys):
            moved.add(name)
    return moved


class Run:
    """The clients, the checks and the verdict."""

    def __init__(self, args):
        self.args = args
        self.started = time.time()
        self.clients = []
        self.failures = []
        # The profile ids the server was told hold `Admin`, before it started,
        # through `HYPERION_PERMISSIONS`. See `hyperion_permission::seed`, and
        # `flake.nix`'s `hudAdmins`, which is the one list both the server's
        # configuration and this argument are built from.
        self.admins = list(args.admin_uuid or [])

    def log(self, line):
        print("%s %-5s %s" % (match.stamp(self.started), "", line), flush=True)

    def check(self, ok, message):
        print(
            "%s %s %s" % (match.stamp(self.started), "PASS" if ok else "FAIL", message),
            flush=True,
        )
        if not ok:
            self.failures.append(message)
        return ok

    def admin_uuid(self, index):
        """The profile id of the `index`-th account configured as `Admin`.

        A client cannot promote itself. `/perms set` is an `Admin` command, and
        it is reachable only by an account an operator named before the server
        started; this gate used to reach it by promoting itself through a hole
        in `/perms` (ENG-10871), which is closed.
        """
        if index >= len(self.admins):
            raise SystemExit(
                "this gate needs %d configured admin(s) and was given %d. Pass "
                "--admin-uuid once per account named as Admin in "
                "HYPERION_PERMISSIONS." % (index + 1, len(self.admins))
            )
        return self.admins[index]

    def connect(self, count):
        for _ in range(count):
            name = "H%d" % (len(self.clients) + 1)
            # The two clients the load bars need an `Admin` command from are
            # the first two, so they log in under the two configured ids and
            # the rest let the server mint one as any player would.
            index = len(self.clients)
            uuid = self.admin_uuid(index) if index < 2 else None
            client = Screen(self.args.host, self.args.port, name, self.started, uuid=uuid)
            client.handshake(self.args.host, self.args.port, 2)
            client.login()
            client.configuration()
            client.enter_play()
            self.clients.append(client)
            client.log("configuration acknowledged")

    def pump(self, seconds):
        deadline = time.time() + seconds
        while True:
            for client in self.clients:
                if not client.alive:
                    continue
                for packet_id, payload in client.drain():
                    client.absorb(packet_id, payload)
                if client.joined:
                    client.repeat_position()
            if time.time() >= deadline:
                return
            time.sleep(0.01)

    def until(self, predicate, seconds, what):
        deadline = time.time() + seconds
        while time.time() < deadline:
            self.pump(0.05)
            if predicate():
                return True
        self.log("TIMED OUT waiting for %s" % what)
        return False

    # --- the experience bar --------------------------------------------

    def experience_check(self):
        client = self.clients[0]

        # The bar is pushed on the first tick after the player exists, which is
        # a tick after the Login packet this has already read.
        self.until(
            lambda: any("Waiting for players" in bar["title"] for bar in client.bars),
            10.0,
            "the hub's boss bar",
        )
        self.check(
            any("Waiting for players" in bar["title"] for bar in client.bars),
            "the hub puts a boss bar up saying what it is waiting for: %s"
            % [bar["title"] for bar in client.bars],
        )
        lobby = [bar for bar in client.bars if "Waiting for players" in bar["title"]]
        if lobby:
            # The denominator is `LobbyConfig::min_players`, read off the bar
            # rather than written down here. It was written down here, as the
            # literal `1/4`, and hyperion#1019 moved that constant to two and
            # left this line asserting a number the server had stopped saying.
            # A gate that restates a server constant is a second place to
            # change it and the first one to be forgotten.
            counted = re.search(r"(\d+)/(\d+)$", lobby[0]["title"])
            needed = int(counted.group(2)) if counted else 0
            self.check(
                lobby[0]["colour"] == "blue"
                and counted is not None
                and int(counted.group(1)) == 1
                and needed >= 2
                and math.isclose(lobby[0]["progress"], 1.0 / needed, abs_tol=STEP),
                "and it counts how many more it needs, with the bar under it "
                "drawing the same fraction: %s" % lobby[0],
            )

        client.command("kit %s" % KIT)
        if not self.until(lambda: len(client.hotbar) >= 2, 30.0, "the kit's hotbar"):
            self.check(False, "the kit reached the hotbar")
            return

        # Every step below is a *transition*, and that is not incidental: the
        # server sends nothing when the bar would not change, so a check that
        # holds a slot reading the same thing as the last one waits for a packet
        # that is correctly never sent. A kit's first ability sits in slot 0,
        # which is where a hand already rests, so the bar is full the moment the
        # kit lands and the empty slot has to come first for anything to move.

        # 1. An empty slot is an empty bar and no number.
        client.experience.clear()
        client.carry(EMPTY_SLOT)
        empty = self.until(
            lambda: any(
                bar["progress"] < 1e-6 and bar["level"] == 0 for bar in client.experience
            ),
            10.0,
            "an empty bar for an empty slot",
        )
        self.check(
            empty,
            "holding a slot with no ability in it is an empty bar and no number: %s"
            % client.experience,
        )

        # 2. A ready ability is a full bar and no number.
        client.experience.clear()
        client.carry(FIRED_SLOT)
        ready = self.until(
            lambda: any(
                bar["progress"] > 1.0 - 1e-6 and bar["level"] == 0
                for bar in client.experience
            ),
            10.0,
            "a full experience bar for a ready ability",
        )
        self.check(
            ready,
            "holding a ready ability is a full bar and no number: %s" % client.experience,
        )

        # 3. Firing it empties the bar and puts the seconds beside it.
        client.experience.clear()
        client.use_slot(FIRED_SLOT, "(the ability whose recharge the bar shows)")
        fired = self.until(
            lambda: any(bar["level"] > 0 for bar in client.experience),
            5.0,
            "the bar to start recharging",
        )
        if not self.check(fired, "firing an ability starts the bar refilling"):
            return
        first = next(bar for bar in client.experience if bar["level"] > 0)
        self.check(
            first["level"] == round(FIRED_COOLDOWN),
            "the number beside the bar is the whole seconds left, %d of %.0f: %s"
            % (first["level"], FIRED_COOLDOWN, first),
        )
        self.check(
            first["progress"] < 3.0 * STEP,
            "and the bar starts near empty rather than near full: %.4f" % first["progress"],
        )

        # 4. It fills, monotonically, and never reads full while it is still
        #    recharging. That last clause is the whole reason the server floors
        #    the fraction rather than rounding it.
        self.pump(FIRED_COOLDOWN + 1.5)
        recharging = [bar for bar in client.experience if bar["level"] > 0]
        rose = all(
            later["progress"] >= earlier["progress"] - 1e-6
            for earlier, later in zip(recharging, recharging[1:])
        )
        self.check(rose, "the bar only ever fills, never drains: %d samples" % len(recharging))
        self.check(
            all(bar["progress"] < 1.0 for bar in recharging),
            "and never reads full while the ability is still refusing: %s"
            % [bar for bar in recharging if bar["progress"] >= 1.0],
        )
        counted = [bar["level"] for bar in recharging]
        self.check(
            counted == sorted(counted, reverse=True) and counted[-1] >= 1,
            "the number counts down and never reaches zero early: %s" % counted,
        )
        self.check(
            client.experience[-1]["progress"] > 1.0 - 1e-6
            and client.experience[-1]["level"] == 0,
            "and the finished cooldown is a full bar with the number gone: %s"
            % client.experience[-1],
        )

        # 5. The bar follows the slot being held, not the last thing fired: a
        #    different, untouched ability reads full even though the one just
        #    used is the last thing this player did.
        client.experience.clear()
        client.carry(EMPTY_SLOT)
        self.until(lambda: client.experience, 5.0, "the bar to notice the empty slot")
        client.experience.clear()
        client.carry(IDLE_SLOT)
        idle = self.until(
            lambda: any(
                bar["progress"] > 1.0 - 1e-6 and bar["level"] == 0
                for bar in client.experience
            ),
            5.0,
            "a full bar for an untouched slot",
        )
        self.check(
            idle,
            "and holding a different, untouched ability is full again: %s" % client.experience,
        )

    # --- the match ------------------------------------------------------

    def match_check(self):
        narrator = self.clients[0]
        narrator.titles.clear()
        narrator.subtitles.clear()
        narrator.bars.clear()

        self.connect(self.args.clients - len(self.clients))
        if not self.until(
            lambda: all(client.joined for client in self.clients), 90.0, "every client in play"
        ):
            self.check(False, "%d clients reached the world" % self.args.clients)
            return

        counting = self.until(
            lambda: any(bar["title"].startswith("Starting in") for bar in narrator.bars),
            90.0,
            "the countdown to appear on the boss bar",
        )
        self.check(
            counting,
            "a full lobby puts the countdown on the boss bar: %s"
            % [bar["title"] for bar in narrator.bars][-3:],
        )
        counting_bars = [bar for bar in narrator.bars if bar["title"].startswith("Starting in")]
        if counting_bars:
            self.check(
                all(bar["colour"] == "yellow" for bar in counting_bars),
                "and it is yellow, not the lobby's blue: %s"
                % sorted({bar["colour"] for bar in counting_bars}),
            )

        # Where the countdown's own packets begin, so the retitle claim below
        # counts seconds passing and not the lobby's player count moving.
        first_countdown_op = len(narrator.bar_ops)

        # The last three seconds, then the word that starts the match.
        started = self.until(lambda: "GO!" in narrator.titles, 120.0, "the match to start")
        if not self.check(started, "the start of the match is a title: %s" % narrator.titles):
            return

        # The bar drains, checked over the whole countdown now that it has run
        # out. Only the tail after the last reset: every join shortens the
        # countdown and hands it a fresh full bar, which is the lobby doing its
        # job rather than the bar going backwards.
        counting_bars = [bar for bar in narrator.bars if bar["title"].startswith("Starting in")]
        full = [
            index for index, bar in enumerate(counting_bars) if bar["progress"] > 1.0 - 1e-6
        ]
        tail = counting_bars[(full[-1] if full else 0) :]
        drained = [bar["progress"] for bar in tail]
        self.check(
            len(drained) > 4 and drained == sorted(drained, reverse=True) and drained[-1] < 0.3,
            "the countdown bar drains towards the start rather than filling: %s"
            % [round(value, 3) for value in drained],
        )

        # An update sends only the update, read off the one bar in the game
        # whose label moves to a clock the server owns. `whole_seconds` is a
        # `ceil`, so the title changes on the tick the timer crosses a whole
        # second, while the fill is quantised to sixty-fourths of the span and
        # steps at its own rate: most of those seconds move the title and
        # nothing else, and each has to cost one `UpdateName` carrying one
        # field. The seconds where a fill step lands on the same tick are a
        # two-field change and are allowed to be an `Add`, which is why this
        # counts the label-only ones rather than requiring every second to be
        # one.
        #
        # This claim used to be read off the CPU bar, where it needed the
        # *host* to move a whole percent of a core inside twenty seconds. See
        # `load_check` for why that made the gate a measurement of the builder.
        #
        # Not redundant with `diff_check`, though the two overlap on one
        # failure and it is worth saying why before somebody deletes this as a
        # duplicate. `diff_check` is four rules of the form "no packet did X",
        # so it can only judge packets that exist; a packet that was never sent
        # is invisible to every one of them. Measured, by making `operation`
        # return `None` for a title-only change: every countdown froze on every
        # screen, and `diff_check` passed all four rules over 780 packets
        # (0 empty, 0 wasteful, 0 orphaned) while this check was the only thing
        # in the suite that went red. The two claims are opposites -- that one
        # says no packet carried a field that did not move, this one says a
        # field that moved did get a packet -- and each is blind exactly where
        # the other looks.
        countdown = {bar["uuid"] for bar in counting_bars}
        retitles = [
            moved
            for uuid, operation, moved, _ in narrator.bar_ops[first_countdown_op:]
            if uuid in countdown and operation == "update_name"
        ]
        label_only = [moved for moved in retitles if moved == {"title"}]
        self.check(
            len(label_only) >= 3 and len(label_only) == len(retitles),
            "a second passing that moves only the countdown's label costs one "
            "packet carrying only the label: %d of %d retitle(s) carried the "
            "title and nothing else" % (len(label_only), len(retitles)),
        )

        digits = [text for text in narrator.titles if text in COUNTDOWN_DIGITS]
        self.check(
            digits == COUNTDOWN_DIGITS,
            "the countdown's last three seconds are titles, in order: %s" % digits,
        )
        self.check(
            "Get ready" in narrator.subtitles,
            "each digit carries a subtitle, which this server had never sent: %s"
            % narrator.subtitles,
        )
        self.check(
            "Smash them off the map" in narrator.subtitles,
            "and so does the start: %s" % narrator.subtitles,
        )
        # A title's own animation, which is what keeps one digit from fading
        # across the next.
        self.check(
            (0, 20, 0) in narrator.animations,
            "the countdown digits are timed to exactly one second: %s" % narrator.animations,
        )

        # The ordering, which is the whole reason the game carries a title and
        # its subtitle as one value. `ClientboundSetSubtitleTextPacket` only
        # stores a line; the title packet is what draws both and restarts the
        # animation. A subtitle sent after its title therefore appears under the
        # *next* one, and a title sent with no subtitle at all inherits whatever
        # was left over. This is the only place either mistake is visible: from
        # the server both look identical to doing it right.
        kinds = [kind for kind, _ in narrator.screen]
        triples = [
            narrator.screen[index - 2 : index + 1]
            for index, kind in enumerate(kinds)
            if kind == "title" and index >= 2
        ]
        wrong = [
            triple
            for triple in triples
            if [kind for kind, _ in triple] != ["times", "subtitle", "title"]
        ]
        self.check(
            triples and not wrong,
            "every title arrives as times, then its subtitle, then itself: "
            "%d title(s), %d out of order %s" % (len(triples), len(wrong), wrong[:2]),
        )

        percent = self.until(
            lambda: any(re.fullmatch(r"\d+%", bar["title"]) for bar in narrator.bars),
            30.0,
            "the percentage to reach the boss bar",
        )
        self.check(
            percent,
            "during a match the boss bar is the player's knockback percentage: %s"
            % [bar["title"] for bar in narrator.bars][-4:],
        )
        readings = [bar for bar in narrator.bars if re.fullmatch(r"\d+%", bar["title"])]
        if readings:
            fresh = readings[0]
            self.check(
                fresh["title"] == "0%" and fresh["colour"] == "green",
                "a fresh player reads 0%% and is green: %s" % fresh,
            )
            self.check(
                math.isclose(fresh["progress"], 1.0, abs_tol=1e-6),
                "with a full bar under it, because the bar is the health the "
                "percentage is derived from: %.3f" % fresh["progress"],
            )
            self.check(
                fresh["flags"] == 0,
                "and it darkens nothing, plays nothing and fogs nothing: flags=%d"
                % fresh["flags"],
            )

    # --- the server's own load ------------------------------------------

    def load_check(self):
        """The two bars carrying the host process's CPU and memory.

        Also the three lifecycle claims the boss bar API rests on, because
        these are the only bars in the game with an audience of more than one
        person: a second viewer joining an already-running bar gets the *same*
        id the first one holds, and a viewer leaving the audience gets a
        `Remove` for it. A per-player bar can demonstrate neither.
        """
        admin = self.clients[0]
        second = self.clients[1]

        # `/serverload` is Admin, and these two clients hold it because an
        # operator said so before the server started: they logged in under
        # profile ids named in `HYPERION_PERMISSIONS`, which is the ordinary
        # way to run this server with administrators
        # (`hyperion_permission::seed`).
        #
        # This gate used to promote itself with `/perms set`, which worked only
        # because that command was gated at `Normal` -- a privilege escalation
        # any player could use, ENG-10871. Reading the group back with the
        # `Normal` half of the same command is what proves the seeding, rather
        # than the bar appearing proving it by side effect.
        for client in (admin, second):
            client.chat.clear()
            client.command("perms get %s" % client.name)
        self.pump(1.0)
        seeded = [
            client.name
            for client in (admin, second)
            if any("group is Admin" in uncoloured(line) for line in client.chat)
        ]
        if not self.check(
            len(seeded) == 2,
            "an account named in the server's configuration joins holding the "
            "group it was given, with nothing asked for: %s" % seeded,
        ):
            return

        # And the hole itself, from a client that has none of this: a `Normal`
        # player asking for `Admin` is refused, and the group does not move.
        plain = self.clients[2]
        plain.chat.clear()
        plain.command("perms set %s admin" % plain.name)
        self.pump(1.0)
        refused = any("do not have permission" in uncoloured(line) for line in plain.chat)
        plain.chat.clear()
        plain.command("perms get %s" % plain.name)
        self.pump(1.0)
        self.check(
            refused
            and any("group is Normal" in uncoloured(line) for line in plain.chat),
            "and a player who was not named cannot name themselves: %s"
            % [uncoloured(line) for line in plain.chat],
        )

        admin.bar_ops.clear()
        admin.command("serverload")
        shown = self.until(
            lambda: len(admin.bars_titled(CPU_LABEL)) == 1
            and len(admin.bars_titled(MEMORY_LABEL)) == 1,
            20.0,
            "the load bars, with a reading on each",
        )
        if not self.check(
            shown,
            "/serverload puts the host's CPU and memory on two boss bars: %s"
            % [bar["title"] for bar in admin.bar_state.values()],
        ):
            return
        cpu = admin.bars_titled(CPU_LABEL)[0]
        memory = admin.bars_titled(MEMORY_LABEL)[0]

        # The reading is what `top` would say and the ceiling is the machine.
        used, ceiling = (int(value) for value in CPU_LABEL.match(admin.bar_state[cpu]["title"]).groups())
        self.check(
            ceiling > 100,
            "the CPU label is unnormalised, so its ceiling is every core and "
            "not one: %s" % admin.bar_state[cpu]["title"],
        )
        self.check(
            0.0 <= admin.bar_state[cpu]["progress"] <= 1.0,
            "and the fill stays a fraction however far past a core the "
            "reading goes: %.4f" % admin.bar_state[cpu]["progress"],
        )
        # The property the whole choice of ceiling is defensible under. A
        # ceiling nobody printed cannot satisfy it and neither can an invented
        # one, because both numbers are read off the bar's own label here.
        self.check(
            abs(admin.bar_state[cpu]["progress"] - used / ceiling) <= 1.0 / LOAD_STEPS,
            "the CPU fill is the quotient of the two numbers in its own "
            "label: %.4f against %d/%d" % (admin.bar_state[cpu]["progress"], used, ceiling),
        )
        resident, total = (
            float(value) for value in MEMORY_LABEL.match(admin.bar_state[memory]["title"]).groups()
        )
        self.check(
            0.0 < resident < total and 0.0 <= admin.bar_state[memory]["progress"] <= 1.0,
            "the memory bar is this process's resident set against the "
            "machine's: %s" % admin.bar_state[memory]["title"],
        )
        self.check(
            abs(admin.bar_state[memory]["progress"] - resident / total) <= 1.0 / LOAD_STEPS,
            "and its fill is the quotient of its own two numbers too: %.4f "
            "against %.2f/%.1f" % (admin.bar_state[memory]["progress"], resident, total),
        )

        # The colour and the effects never move, so after the `Add` that
        # carried them they are never on the wire again.
        for name, uuid in (("CPU", cpu), ("memory", memory)):
            operations = admin.ops_for(uuid)
            self.check(
                operations[:1] == ["add"],
                "the %s bar arrives as one packet carrying the whole bar: %s"
                % (name, operations[:3]),
            )
            self.check(
                "update_style" not in operations
                and "update_properties" not in operations,
                "and its colour and effects, which never move, are never on "
                "the wire again after it: %s" % operations,
            )
        # Neither bar is asserted on for how many packets it sent, because
        # neither count is a property of the software. The CPU label is a whole
        # percent of one core, so whether this second's reading differs from
        # the last one is a fact about the host: this check used to wait twenty
        # seconds for two of them and went red on a 128-core builder that idled
        # at `CPU 1% of 12800%` and held that label the whole time, which is
        # the gate measuring the machine. The memory bar is the same shape
        # against how much memory the box has. What is checked instead is that
        # whatever they sent obeyed the rule, which `diff_check` does over
        # every packet of the whole run, and that a label-only change costs one
        # `UpdateName`, which `match_check` reads off the countdown bar because
        # the server's own clock moves that label rather than the host's load.
        # What that gives up is a wire-level witness for these two bars
        # specifically; `egress::boss_bar`'s own truth-table test covers the
        # transition itself.
        self.log(
            "the memory bar sent %d packet(s) against the CPU bar's %d"
            % (len(admin.ops_for(memory)), len(admin.ops_for(cpu)))
        )

        # A viewer who joins an audience that already exists is told about the
        # bars under the ids everybody else already holds. This is the whole
        # reason the sent state is `(Sent, viewer)` pair data: the second
        # viewer has none, so the next tick sends them `Add`, and there is no
        # join handler anywhere.
        second.bar_ops.clear()
        second.command("serverload")
        joined = self.until(
            lambda: cpu in second.bar_state and memory in second.bar_state,
            20.0,
            "the second viewer to be given the running bars",
        )
        self.check(
            joined,
            "a viewer joining a running bar's audience is sent it under the "
            "same id the others hold: %s" % sorted(second.bar_state),
        )
        if joined:
            self.check(
                second.ops_for(cpu)[0] == "add" and second.ops_for(memory)[0] == "add",
                "as an Add and not as an update to a bar they never had: %s"
                % [second.ops_for(cpu)[:2], second.ops_for(memory)[:2]],
            )

        # And leaving that audience takes them away, which is the same
        # `(Sent, viewer)` pair going away as a viewer disconnecting.
        second.bar_ops.clear()
        second.command("serverload")
        gone = self.until(
            lambda: cpu not in second.bar_state and memory not in second.bar_state,
            20.0,
            "the second viewer's bars to be taken away",
        )
        self.check(
            gone,
            "leaving a bar's audience removes it from that screen and nobody "
            "else's: %s" % sorted(second.bar_state),
        )
        self.check(
            sorted(second.ops_for(cpu)) == ["remove"]
            and cpu in admin.bar_state
            and memory in admin.bar_state,
            "with a Remove, while the viewer who is still watching keeps "
            "theirs: %s / %s" % (second.ops_for(cpu), sorted(admin.bar_state)),
        )

        # A viewer disconnecting while a bar is on their screen is the third
        # way a bar stops being shown, and the one with nothing to observe on
        # the wire: the packet that must *not* be written is one to a socket
        # that has gone. What is observable is that the server carries on.
        leaver = Screen(
            self.args.host, self.args.port, "HX", self.started, uuid=self.admin_uuid(2)
        )
        leaver.handshake(self.args.host, self.args.port, 2)
        leaver.login()
        leaver.configuration()
        leaver.enter_play()
        self.clients.append(leaver)
        self.until(lambda: leaver.joined, 60.0, "the disconnecting viewer to join")
        leaver.command("serverload")
        self.until(lambda: cpu in leaver.bar_state, 20.0, "the leaver's own load bars")
        leaver.sock.close()
        leaver.alive = False
        # Carrying on is proved by a change this check causes rather than by
        # one it waits for. Waiting was what this did -- two more CPU readings
        # inside twenty seconds -- and a reading only reaches the wire when the
        # host's load crosses a whole percent of a core, so on a quiet builder
        # the survivor's screen was correct and silent and the gate read that
        # as the server having stopped. Toggling the audience puts the same
        # drive system through the same work on demand.
        #
        # What that gives up is the unprompted case: this no longer notices a
        # server that draws only when asked. Nothing else here can, without
        # asking the machine to be busy.
        #
        # STATUS: UNPROVEN. Read this before deleting it as redundant, and
        # before trusting it. It is deterministic and it is green, which is
        # more than the assertion it replaced could say, but nobody has yet
        # shown it catches anything the three earlier toggle checks do not.
        # Two attempts to break the game so that only this check notices:
        #
        #  1. Breaking `/serverload`'s re-add. The gate died at the *first*
        #     `/serverload` and `load_check` returned before reaching here, so
        #     this check never ran. Too blunt.
        #  2. `add_if_new` matching `(Sent, Wildcard)` instead of
        #     `(Sent, viewer)`, so a bar anybody holds is never sent to anyone
        #     else. Two neighbouring checks went red and this one PASSED: the
        #     admin already held the bars, so their toggle still worked.
        #
        # The defect only this could catch is one that needs a stale
        # `(Sent, dead_viewer)` pair to exist before it shows, because this is
        # the only bar check that runs after a socket has died -- the earlier
        # toggle checks all run while every viewer is still connected. That is
        # an argument, not evidence, and it is labelled as one deliberately.
        # If you want to finish the job, that is the defect to build.
        admin.bar_ops.clear()
        admin.command("serverload")
        off = self.until(
            lambda: cpu not in admin.bar_state and memory not in admin.bar_state,
            20.0,
            "the surviving viewer's bars to come off",
        )
        admin.command("serverload")
        on = self.until(
            lambda: cpu in admin.bar_state and memory in admin.bar_state,
            20.0,
            "and to come back",
        )
        # Only the lifecycle operations, because a CPU reading that did happen
        # to move in between is a packet this check has no opinion about.
        lifecycle = {
            name: [op for op in admin.ops_for(uuid) if op in ("remove", "add")]
            for name, uuid in (("CPU", cpu), ("memory", memory))
        }
        self.check(
            off and on and all(ops == ["remove", "add"] for ops in lifecycle.values()),
            "a viewer disconnecting with a bar on their screen leaves the "
            "server drawing everybody else's: %s" % lifecycle,
        )

    # --- a player who arrives after the match started --------------------

    def joiner_check(self):
        """Somebody who was not there when the bar was made still gets one.

        The bar this server draws for a match is per player, so a joiner's own
        bar is new; what the claim is about is that nothing in the server had
        to notice they arrived. They have no `(Sent, bar)` pair, so the next
        tick sends them an `Add`, and the join case is the absence of state
        rather than a code path.
        """
        late = Screen(self.args.host, self.args.port, "H9", self.started)
        late.handshake(self.args.host, self.args.port, 2)
        late.login()
        late.configuration()
        late.enter_play()
        self.clients.append(late)
        if not self.until(lambda: late.joined, 60.0, "the late client in play"):
            self.check(False, "a client joined after the match started")
            return
        arrived = self.until(lambda: late.bar_state, 20.0, "the late client's own bar")
        self.check(
            arrived,
            "a player who joins after a match started is given the bar for "
            "it: %s" % [bar["title"] for bar in late.bar_state.values()],
        )
        if not arrived:
            return
        uuid = next(iter(late.bar_state))
        self.check(
            late.ops_for(uuid)[0] == "add",
            "as an Add carrying the whole bar, not as an update to one they "
            "never had: %s" % late.ops_for(uuid)[:3],
        )
        held = {bar for client in self.clients[:8] for bar in client.bar_state}
        self.check(
            uuid not in held,
            "and under its own id, because this bar carries a number that is "
            "only true of them: %s against %d others" % (uuid[:8], len(held)),
        )    # --- the diffing, over every packet of the whole run ------------------

    def diff_check(self):
        """No packet ever carried a field that did not change.

        This is the one claim that cannot be made from any single moment, and
        it is the claim the whole API is for. Every boss bar packet every
        client received in this run is checked against the state that client
        was in when it arrived, so a diff that leaked one field on one
        transition fails here even though every screen looked right.

        Three rules, and they are the whole of `egress::boss_bar::operation`
        and the observer that tears a bar down:

        * A packet that changes nothing must not exist. An unchanged bar is
          silence, and a narrow operation can only carry its own field, so
          this one rule covers "an update sends only the update" outright: an
          `UpdateName` whose title the client already had is a packet with
          nothing in it.
        * A repeat `Add` is only allowed where it is *cheaper* than the narrow
          operations it replaces, which is two or more fields at once, and it
          is what makes a multi-field change atomic on a protocol that has no
          other way to be.
        * Nothing arrives for a bar the client was never given. A `Sent` pair
          recorded without the `Add` that goes with it, or one left behind
          after a `Remove`, both surface here and nowhere else -- from the
          server both look exactly like doing it right.
        """
        packets = 0
        empty = []
        wasteful = []
        orphans = []
        for client in self.clients:
            for uuid, operation, moved, known in client.bar_ops:
                packets += 1
                if not known:
                    if operation != "add":
                        orphans.append((client.name, uuid[:8], operation))
                    continue
                if operation == "remove":
                    continue
                if operation == "add":
                    if len(moved) < 2:
                        wasteful.append((client.name, uuid[:8], sorted(moved)))
                elif not moved:
                    empty.append((client.name, uuid[:8], operation))

        self.check(
            packets > 200,
            "%d boss bar packet(s) went out this run, which is enough of them "
            "for the rules below to mean something" % packets,
        )
        self.check(
            not empty,
            "no boss bar packet said anything the client already knew: "
            "%d of %d %s" % (len(empty), packets, empty[:3]),
        )
        self.check(
            not wasteful,
            "and a whole bar was only ever resent where two or more fields "
            "moved at once: %d of %d %s" % (len(wasteful), packets, wasteful[:3]),
        )
        self.check(
            not orphans,
            "and nothing arrived for a bar the client had never been given: "
            "%d of %d %s" % (len(orphans), packets, orphans[:3]),
        )

    def run(self):
        self.connect(1)
        if not self.until(lambda: self.clients[0].joined, 90.0, "the first client in play"):
            self.check(False, "a client reached the world")
            return self.report()
        self.experience_check()
        self.match_check()
        self.joiner_check()
        self.load_check()
        self.diff_check()
        return self.report()

    def report(self):
        print("", flush=True)
        self.log(
            "RESULT: %s (%d check(s) failed)"
            % ("ok" if not self.failures else "failure", len(self.failures))
        )
        for failure in self.failures:
            print("  failed: %s" % failure, file=sys.stderr)
        return 1 if self.failures else 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=25565)
    parser.add_argument(
        "--admin-uuid",
        action="append",
        metavar="UUID",
        help="a profile id the server was told holds Admin, through "
        "HYPERION_PERMISSIONS. This gate needs three of them, because "
        "/serverload is an Admin command and nothing lets a client give "
        "itself the group (ENG-10871). flake.nix builds both this argument "
        "and the server's configuration from one list",
    )
    parser.add_argument(
        "--clients",
        type=int,
        default=8,
        help="at or above `LobbyConfig::full_players`, which is what makes the "
        "countdown its ten second one rather than its sixty second one. Eight "
        "and not the four that now fills a lobby, because `diff_check` wants "
        "more than one client's worth of traffic to count and the two joiner "
        "checks want a match already under way to walk into",
    )
    args = parser.parse_args()
    return Run(args).run()


if __name__ == "__main__":
    sys.exit(main())
