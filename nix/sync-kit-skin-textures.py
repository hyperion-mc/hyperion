"""Repin every kit's skin image by hash, into events/smash/skins/textures.lock.json.

The image check (`verify-kit-skin-images.py`) is offline: it validates images
already in the store, fetched from pinned (url, sha256) pairs. This regenerates
those pairs from the committed `.value` payloads, so replacing a skin is still a
two-file change plus one command rather than a hand-edited hash. Impure: it
reaches Mojang's texture host, which is why it is a `nix run` and not a check.
"""

import base64
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SKINS = ROOT / "events" / "smash" / "skins"


def main():
    lock = {}
    for value_path in sorted(SKINS.glob("*.value")):
        mob = value_path.stem
        payload = json.loads(base64.b64decode(value_path.read_text().strip()))
        url = (payload.get("textures") or {}).get("SKIN", {}).get("url")
        if not url:
            raise SystemExit("%s carries no SKIN url" % mob)
        result = subprocess.run(
            ["nix", "store", "prefetch-file", "--json", url],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise SystemExit("prefetch failed for %s (%s): %s" % (mob, url, result.stderr))
        lock[mob] = {"url": url, "sha256": json.loads(result.stdout)["hash"]}
        print("pinned %-16s %s" % (mob, lock[mob]["sha256"]), file=sys.stderr)

    out = SKINS / "textures.lock.json"
    out.write_text(json.dumps(lock, indent=2, sort_keys=True) + "\n")
    print("wrote %s with %d entries" % (out, len(lock)), file=sys.stderr)


if __name__ == "__main__":
    main()
