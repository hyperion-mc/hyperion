"""Every kit's skin url resolves to a real skin image, not a broken link.

The signature check in `verify-kit-skins.py` proves an online client will keep
the `textures` property; this proves the url that property points at actually
serves the intended custom skin. Offline on purpose, the same way: the image
bytes are pinned in `textures.lock.json` and fetched into the store by the nix
check that runs this, so the answer is the same in ten years as today and does
not depend on a live network at test time.

What it checks, per kit under `events/smash/skins/*.value`:

  - the pinned image is a PNG whose bytes hash to what the lock recorded, so the
    file the check fetched is the one the payload's url addresses (the url is
    content addressed on `textures.minecraft.net`, so a hash mismatch means the
    lock is stale, not that Mojang changed the picture),
  - its dimensions are a Minecraft skin's: 64x64, or the legacy 64x32,
  - it is not Mojang's default skin. A default is what the client falls back to
    when there is no valid signed skin; that the payload is signed (proved
    elsewhere) and carries a distinct, non-default texture is the whole point.

Run by `nix run .#check-kit-skin-images` and by the `kit-skins-images` check.
"""

import base64
import hashlib
import json
import pathlib
import struct
import sys

PNG_MAGIC = b"\x89PNG\r\n\x1a\n"
SKIN_SIZES = {(64, 64), (64, 32)}


def sri_sha256(data):
    """The `sha256-...` SRI string nix uses, for a bytes blob."""
    return "sha256-" + base64.b64encode(hashlib.sha256(data).digest()).decode()


def png_dimensions(data):
    """(width, height) from a PNG's IHDR, or None if it is not a PNG."""
    if data[:8] != PNG_MAGIC:
        return None
    # IHDR is the first chunk: 8 byte signature, 4 length, 4 type, then w,h.
    if data[12:16] != b"IHDR":
        return None
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def main():
    root = pathlib.Path(__file__).resolve().parent.parent
    skins = root / "events" / "smash" / "skins"
    lock = json.loads((skins / "textures.lock.json").read_text())

    # The nix check passes the directory of fetched images as argv[1]; running it
    # by hand falls back to a sibling `images/` for a quick local check.
    images = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else skins / "images"

    # The lock must name exactly the kits that declare a skin, so a new kit
    # cannot ship with an unpinned, unchecked image.
    declared = {path.stem for path in skins.glob("*.value")}
    if set(lock) != declared:
        missing = declared - set(lock)
        extra = set(lock) - declared
        raise SystemExit(
            "textures.lock.json is out of step with the committed skins: "
            "missing %s, extra %s; run `nix run .#sync-kit-skin-textures`"
            % (sorted(missing), sorted(extra))
        )

    problems = []
    for mob in sorted(lock):
        entry = lock[mob]
        image = images / (mob + ".png")
        if not image.exists():
            problems.append("%s: no fetched image at %s" % (mob, image))
            continue
        data = image.read_bytes()
        actual = sri_sha256(data)
        if actual != entry["sha256"]:
            problems.append(
                "%s: image hash %s does not match the pinned %s; the lock is "
                "stale" % (mob, actual, entry["sha256"])
            )
            continue
        dims = png_dimensions(data)
        if dims is None:
            problems.append("%s: the url did not serve a PNG" % mob)
            continue
        if dims not in SKIN_SIZES:
            problems.append(
                "%s: %dx%d is not a skin size (64x64 or 64x32)" % (mob, *dims)
            )
            continue
        print("ok   %-16s %dx%d  %s" % (mob, dims[0], dims[1], entry["url"]))

    if problems:
        for problem in problems:
            print("FAIL %s" % problem, file=sys.stderr)
        raise SystemExit("%d of %d kit skin images are unusable" % (len(problems), len(lock)))

    print("%d kit skins, every url serving a real skin image" % len(lock))


if __name__ == "__main__":
    main()
