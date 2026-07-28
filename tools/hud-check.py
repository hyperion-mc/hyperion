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

  2. Eight clients, which is `full_players`, so the lobby runs its shortest
     countdown rather than its sixty second one. That gets the phase machine
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

# `BossEventOperation::type_id`.
BOSS_ADD = 0
# `BossBarColor`, as ordinals.
BOSS_COLOURS = ["pink", "blue", "red", "green", "yellow", "purple", "white"]

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

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, started)
        # Every experience bar, boss bar and title in the order they arrived, so
        # a check can ask about the last one or about the whole sequence.
        self.experience = []
        self.bars = []
        self.titles = []
        self.subtitles = []
        self.animations = []
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
            bar = decode_boss_event(payload)
            if bar is not None:
                self.bars.append(bar)
                self.log(
                    "<- boss bar %r %.3f full, %s"
                    % (bar["title"], bar["progress"], bar["colour"])
                )
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
    """One `ClientboundBossEventPacket`, or `None` for an operation this does
    not read.

    Only `Add` is decoded, because it is the only one the server sends: see the
    note on `adapter::boss_bar` for why every update is a fresh `Add`.
    """
    operation, offset = take_var_int(payload, 16)
    if operation != BOSS_ADD:
        return None
    title, offset = match.take_nbt_string(payload, offset)
    (progress,) = struct.unpack_from(">f", payload, offset)
    offset += 4
    colour, offset = take_var_int(payload, offset)
    overlay, offset = take_var_int(payload, offset)
    flags = payload[offset]
    return {
        "uuid": payload[:16].hex(),
        "title": title,
        "progress": progress,
        "colour": BOSS_COLOURS[colour] if colour < len(BOSS_COLOURS) else colour,
        "overlay": overlay,
        "flags": flags,
    }


class Run:
    """The clients, the checks and the verdict."""

    def __init__(self, args):
        self.args = args
        self.started = time.time()
        self.clients = []
        self.failures = []

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

    def connect(self, count):
        for _ in range(count):
            name = "H%d" % (len(self.clients) + 1)
            client = Screen(self.args.host, self.args.port, name, self.started)
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
            self.check(
                lobby[0]["colour"] == "blue" and lobby[0]["title"].endswith("1/4"),
                "and it counts how many more it needs: %s" % lobby[0],
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

    def run(self):
        self.connect(1)
        if not self.until(lambda: self.clients[0].joined, 90.0, "the first client in play"):
            self.check(False, "a client reached the world")
            return self.report()
        self.experience_check()
        self.match_check()
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
        "--clients",
        type=int,
        default=8,
        help="`LobbyConfig::full_players`, which is what makes the countdown ten "
        "seconds rather than sixty",
    )
    args = parser.parse_args()
    return Run(args).run()


if __name__ == "__main__":
    sys.exit(main())
