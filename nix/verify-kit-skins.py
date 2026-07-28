"""Every kit's skin must be one the client will actually show other players.

Offline on purpose. The rule this enforces is a property of the payload and of
Mojang's public key, both of which are committed, so the check has an answer
without a network and gives the same answer in ten years as today.

What it checks, per kit under `events/smash/src/module/kits/`:

  - the kit declares a skin, and the pair of files it names both exist
  - the `.value` is base64 of a Minecraft textures payload with a SKIN entry
  - that skin URL is on `textures.minecraft.net`, the only host
    `TextureUrlChecker.ALLOWED_DOMAINS` contains
  - the `.sig` verifies over the `.value` bytes under one of Mojang's profile
    property keys, RSA with SHA-1, which is what
    `YggdrasilServicesKeyInfo.validateProperty` does

The last one is the whole point. `SkinManager.createLookup` keeps a skin only
when `!requireSecure || skin.secure()`, and `PlayerInfo.createSkinLookup` passes
`requireSecure = !isLocalPlayer`, so an unsigned property is a skin the wearer
sees and nobody else does. That failure is invisible from the server: the
packets go out, the wearer's own client is happy, and every other player quietly
sees Steve. Nothing but this check notices.
"""

import base64
import json
import pathlib
import re
import sys
from urllib.parse import urlparse

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import padding
from cryptography.hazmat.primitives.serialization import load_der_public_key
from cryptography.exceptions import InvalidSignature

ALLOWED_TEXTURE_DOMAINS = {"textures.minecraft.net"}
DECLARATION = re.compile(r'kit_skin!\("([a-z_]+)"\)')


def fail(problems, kit, message):
    problems.append("%s: %s" % (kit, message))


def check_payload(problems, kit, value):
    try:
        raw = base64.b64decode(value, validate=True)
    except Exception as error:
        fail(problems, kit, "value is not base64: %s" % error)
        return
    try:
        payload = json.loads(raw)
    except ValueError as error:
        fail(problems, kit, "value is not JSON: %s" % error)
        return
    skin = (payload.get("textures") or {}).get("SKIN")
    if not skin:
        fail(problems, kit, "payload carries no SKIN texture")
        return
    url = skin.get("url", "")
    host = urlparse(url).hostname
    if host not in ALLOWED_TEXTURE_DOMAINS:
        # authlib throws the whole payload away, signature and all, so this is
        # not a lesser failure than a bad signature.
        fail(problems, kit, "skin url host %r is not a Mojang texture host" % host)


def main():
    root = pathlib.Path(__file__).resolve().parent.parent
    kits_dir = root / "events" / "smash" / "src" / "module" / "kits"
    skins_dir = root / "events" / "smash" / "skins"

    keys = [
        load_der_public_key(base64.b64decode(entry["publicKey"]))
        for entry in json.loads((skins_dir / "mojang-profile-keys.json").read_text())[
            "profilePropertyKeys"
        ]
    ]
    if not keys:
        raise SystemExit("mojang-profile-keys.json lists no profile property keys")

    kits = sorted(path for path in kits_dir.glob("*.rs"))
    if not kits:
        raise SystemExit("found no kits under %s" % kits_dir)

    problems = []
    checked = 0
    for path in kits:
        kit = path.stem
        declared = DECLARATION.findall(path.read_text())
        if not declared:
            fail(problems, kit, "declares no skin; add .skin(crate::kit_skin!(...))")
            continue
        if len(declared) > 1:
            fail(problems, kit, "declares %d skins" % len(declared))
            continue
        mob = declared[0]

        value_path = skins_dir / (mob + ".value")
        sig_path = skins_dir / (mob + ".sig")
        if not value_path.exists() or not sig_path.exists():
            fail(problems, kit, "names %r but %s.{value,sig} is missing" % (mob, mob))
            continue

        value = value_path.read_text().strip()
        signature = sig_path.read_text().strip()
        check_payload(problems, kit, value)

        if not signature:
            # Distinguished from a wrong signature because the fix differs: an
            # empty one means nobody ever attached Mojang's, and hyperion sends
            # the property unsigned rather than not at all.
            fail(problems, kit, "has no signature, so only the wearer would see this skin")
            continue

        try:
            blob = base64.b64decode(signature, validate=True)
        except Exception as error:
            fail(problems, kit, "signature is not base64: %s" % error)
            continue

        # `validateProperty` signs the base64 `value` string itself, not the
        # JSON it decodes to.
        signed = False
        for key in keys:
            try:
                key.verify(blob, value.encode(), padding.PKCS1v15(), hashes.SHA1())
                signed = True
                break
            except InvalidSignature:
                continue
        if not signed:
            fail(
                problems,
                kit,
                "signature does not verify under any Mojang profile property key, "
                "so only the wearer would see this skin",
            )
            continue
        checked += 1
        print("ok   %-16s %s" % (kit, mob))

    if problems:
        for problem in problems:
            print("FAIL %s" % problem, file=sys.stderr)
        raise SystemExit("%d of %d kits have an unusable skin" % (len(problems), len(kits)))

    print("%d kits, every skin signed by Mojang and hosted where the client will load it" % checked)


if __name__ == "__main__":
    main()
