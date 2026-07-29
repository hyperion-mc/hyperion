#!/usr/bin/env python3
"""What every other player sees of a player who put on a kit.

The two questions a skin has to answer on the wire, driven against a live smash
server with one player who selects a kit and one who only watches:

  1. The watcher's tab-list profile for the wearer carries a `textures`
     property equal to the payload committed for that kit, WITH its Mojang
     signature. An unsigned property looks identical from the server and is
     silently dropped by every client but the wearer's own, so the signature is
     the half that decides whether anyone else sees the skin at all.

  2. The watcher's entity metadata for the wearer carries the skin-overlay mask
     with the hat bit set. The hat, jacket, sleeves and trouser overlays are
     the second skin layer; a zero mask renders a player as a bald base model
     however good the texture is. `metadata::show_all` sends 0xFF as a floor
     and a client that announced its own parts refines it, so the wearer here
     should arrive wearing every overlay.

  3. Putting on a kit does not cost the wearer a Respawn. A skin change used to
     respawn the wearer, which threw their world away and left a real client on
     "Loading terrain..." forever; this asserts the cheap path stayed cheap.

Exits non-zero on the first thing that is not true, after printing what it saw.
This is the regression gate for the skin and hat bugs: break `roster::entry_of`
so it drops the property, or `metadata::show_all` so the mask is zero, and this
goes red.
"""

import argparse
import base64
import importlib.util
import json
import pathlib
import struct
import sys
import time
from urllib.parse import urlparse

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import padding
from cryptography.hazmat.primitives.serialization import load_der_public_key

TOOLS = pathlib.Path(__file__).resolve().parent
ROOT = TOOLS.parent


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


match = _load("smash_match", "smash-match.py")
monitor = _load("packet_monitor", "packet_monitor.py")

# `TextureUrlChecker.ALLOWED_DOMAINS` on the client is exactly this one host;
# authlib throws the whole payload away, signature and all, for any other.
ALLOWED_TEXTURE_DOMAINS = {"textures.minecraft.net"}


def _mojang_keys():
    """Mojang's committed profile-property public keys, DER-decoded."""
    data = json.loads(
        (ROOT / "events" / "smash" / "skins" / "mojang-profile-keys.json").read_text()
    )
    return [load_der_public_key(base64.b64decode(e["publicKey"])) for e in data["profilePropertyKeys"]]


def yggdrasil_signed(value, signature):
    """Whether `signature` verifies over the base64 `value` string's bytes under
    one of Mojang's keys, RSA with SHA-1.

    This is exactly what `YggdrasilServicesKeyInfo.validateProperty` runs before
    an online client keeps a skin for anyone but its wearer. Byte-equality to a
    committed file is a weaker claim: a committed-but-invalid signature passes
    that and still renders as Steve for every other player.
    """
    if not signature:
        return False
    try:
        blob = base64.b64decode(signature, validate=True)
    except (ValueError, base64.binascii.Error):
        return False
    for key in _mojang_keys():
        try:
            key.verify(blob, value.encode(), padding.PKCS1v15(), hashes.SHA1())
            return True
        except InvalidSignature:
            continue
    return False


def skin_url(value):
    """The SKIN texture url inside a base64 textures payload, or ''."""
    try:
        payload = json.loads(base64.b64decode(value))
    except (ValueError, base64.binascii.Error):
        return ""
    return ((payload.get("textures") or {}).get("SKIN") or {}).get("url", "")

base = match.base

take_var_int = base.take_var_int
var_int = base.var_int


class Probe(match.MatchClient):
    """A scripted player that feeds everything it reads to a `Monitor`."""

    def __init__(self, host, port, name, started):
        super().__init__(host, port, name, started)
        self.joined = False
        self.entity_id = None
        self.monitor = monitor.Monitor()

    def absorb(self, packet_id, payload):
        # Keep-alives and teleport acks keep the connection from being culled;
        # everything else is the monitor's business.
        if packet_id == match.S2C_LOGIN:
            self.entity_id = struct.unpack(">i", payload[:4])[0]
            self.joined = True
        elif packet_id == match.S2C_PLAYER_POSITION:
            teleport_id, offset = take_var_int(payload)
            x, y, z = struct.unpack(">ddd", payload[offset : offset + 24])
            self.position = (x, y, z)
            self.send(match.C2S_ACCEPT_TELEPORTATION, var_int(teleport_id))
        elif packet_id == match.S2C_KEEP_ALIVE:
            self.send(match.C2S_KEEP_ALIVE, payload[:8])
        self.monitor.feed(packet_id, payload)


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
    parser.add_argument("--kit", default="Zombie")
    args = parser.parse_args()

    started = time.monotonic()
    failures = []

    def check(ok, message):
        print("%s %s" % ("PASS" if ok else "FAIL", message), flush=True)
        if not ok:
            failures.append(message)

    clients = []
    for name in ("Wearer", "Watcher"):
        client = Probe(args.host, args.port, name, started)
        client.handshake(args.host, args.port, 2)
        client.login()
        client.configuration()
        client.enter_play()
        clients.append(client)
        pump(clients, 0.5)
    wearer, watcher = clients

    check(
        wearer.profile_id is not None and watcher.profile_id is not None,
        "both players finished login",
    )

    # Both have to see each other before a skin change means anything.
    ok = wait_until(
        clients,
        lambda: wearer.profile_id in watcher.monitor.entity_of_profile,
        30.0,
        "the watcher to be sent the wearer's entity",
    )
    check(ok, "the watcher was sent the wearer's entity")

    respawns_before = wearer.monitor.respawns
    wearer.command("kit %s" % args.kit)

    skins = ROOT / "events" / "smash" / "skins"
    value_path = skins / (args.kit.lower() + ".value")
    sig_path = skins / (args.kit.lower() + ".sig")
    if not value_path.exists():
        check(False, "no committed skin for kit %r at %s" % (args.kit, value_path))
        return _verdict(failures)
    expected_value = value_path.read_text().strip()
    expected_sig = sig_path.read_text().strip()

    ok = wait_until(
        clients,
        lambda: watcher.monitor.texture_of(wearer.profile_id) is not None,
        30.0,
        "the watcher to receive a textures property for the wearer",
    )
    check(ok, "the watcher's profile for the wearer carries a textures property")

    view = watcher.monitor.view_of(wearer.profile_id)
    print("MONITOR_VIEW " + json.dumps(view), flush=True)

    check(
        view["textures_value"] == expected_value,
        "that texture is the one committed for %s" % args.kit,
    )
    check(
        view["textures_signature"] == expected_sig,
        "and it carries its Mojang signature, without which only the wearer "
        "would see it",
    )

    # The signed-skins crux: verify the signature the wire carried the way a
    # vanilla online client does, not merely that it equals a committed file.
    wire_value = view["textures_value"] or ""
    wire_sig = view["textures_signature"] or ""
    check(
        yggdrasil_signed(wire_value, wire_sig),
        "the wire signature verifies under a Mojang profile key (SIGNED), so a "
        "real online client keeps this skin for other players and does not fall "
        "back to Steve",
    )
    check(
        urlparse(skin_url(wire_value)).hostname in ALLOWED_TEXTURE_DOMAINS,
        "the skin url is on textures.minecraft.net, the only host a client will "
        "load a skin from",
    )
    # A guard is not a guard until it has failed: a one-byte tamper must not
    # verify, or the check above would pass for any bytes at all.
    _blob = bytearray(base64.b64decode(wire_sig)) if wire_sig else bytearray(b"\x00")
    _blob[0] ^= 0x01
    check(
        not yggdrasil_signed(wire_value, base64.b64encode(bytes(_blob)).decode()),
        "a one-byte-tampered signature is rejected, so the verification above is "
        "real and not vacuous",
    )

    ok = wait_until(
        clients,
        lambda: watcher.monitor.skin_parts_of(wearer.profile_id) is not None,
        30.0,
        "the watcher to receive the wearer's skin-overlay mask",
    )
    check(ok, "the watcher was sent the wearer's skin-overlay mask")
    view = watcher.monitor.view_of(wearer.profile_id)
    print("MONITOR_VIEW " + json.dumps(view), flush=True)
    check(
        view["hat_shown"],
        "the hat overlay bit is set, so the wearer is not rendered bald: mask=%s"
        % _hex(view["skin_parts"]),
    )
    check(
        view["all_parts_shown"],
        "every skin overlay is on (cape, jacket, sleeves, trousers, hat): "
        "mask=%s" % _hex(view["skin_parts"]),
    )

    check(
        wearer.monitor.respawns == respawns_before,
        "putting on a kit did not respawn the wearer (respawns: %d -> %d)"
        % (respawns_before, wearer.monitor.respawns),
    )

    return _verdict(failures)


def _hex(value):
    return "None" if value is None else "0x%02X" % value


def _verdict(failures):
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
