"""Fail when a vanilla registry name appears as a raw string in Rust.

A `"minecraft:..."` literal is an unproven claim. Nothing checks that the name
exists, that this version still spells it that way, or that the registry it
belongs to is the one being indexed, so a typo or a Mojang rename becomes a
`None` on whichever tick that value was next needed -- or worse, a silent
fallback. Every static registry is now a closed Rust enum, so the name can be a
variant instead, and the compiler answers all three questions at the line that
asked.

This is what stops the class from growing back. It is deliberately not a
regex over lines: `//! minecraft:pig` in a doc comment and `"minecraft:pig"` in
code look identical to one, and this repo's doc comments name registry entries
constantly. The scan below walks each file the way Rust lexes it and only ever
looks inside string literals.

Two kinds of exemption, and they are different on purpose.

`ALLOWED` is a path with a reason: a file where these literals are correct and
are expected to stay. It is short, and adding to it should feel like a decision.

The baseline is a path with a *list*: literals that have not been migrated yet.
It is debt with a name on it, it ratchets in both directions, and the intended
end state is that it is empty and this script stops needing it.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

# Namespaces a registry entry can carry. `brigadier:` is the six command
# argument types Mojang did not move into their own namespace.
NAMESPACE = re.compile(r"\b(?:minecraft|brigadier):[a-z0-9_./]+")

# The generator's own output, recognised by the header every generator writes
# rather than by a path list, so a new generated file is covered the day it
# appears and a file that stops being generated stops being exempt.
GENERATED_MARKER = "@generated"

# Files where a raw registry name is the right thing, by path, with the reason.
#
# Not a pattern. A pattern loose enough to cover these would also cover the
# next mistake.
ALLOWED: dict[str, str] = {
    "crates/hyperion/src/net/protocol/registries.rs": (
        "the element names of the 29 *dynamic* registries, which protocol.json "
        "does not carry and so have no enum. This file is a name table of the "
        "same kind as generated output and its own header says it should be "
        "deleted once the proto crate generates dynamic registry contents; "
        "until then the names have to be written down somewhere"
    ),
    "crates/hyperion-minecraft-proto/tests/registry_enum.rs": (
        "the test that proves the enums cover the registries has to name "
        "entries as strings to check that the string form resolves, and names "
        "deliberate non-entries to check that they do not"
    ),
}


def literals(src: str):
    """Yield (line, text) for every string literal in `src`.

    Comments, char literals and lifetimes are stepped over. Raw strings keep
    their contents verbatim, which is what a `r"minecraft:x"` needs.
    """
    i, n, line = 0, len(src), 1
    while i < n:
        char = src[i]
        if char == "\n":
            line += 1
            i += 1
            continue
        if src.startswith("//", i):
            end = src.find("\n", i)
            i = n if end < 0 else end
            continue
        if src.startswith("/*", i):
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif src.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    line += src[i] == "\n"
                    i += 1
            continue
        raw = re.match(r'(b?r)(#*)"', src[i:])
        if raw:
            start = i + raw.end()
            close = '"' + raw.group(2)
            end = src.find(close, start)
            end = n if end < 0 else end
            yield line, src[start:end]
            line += src.count("\n", i, end)
            i = end + len(close)
            continue
        if char == '"':
            j, out = i + 1, []
            while j < n:
                if src[j] == "\\":
                    out.append(src[j:j + 2])
                    j += 2
                    continue
                if src[j] == '"':
                    break
                out.append(src[j])
                j += 1
            text = "".join(out)
            yield line, text
            line += text.count("\n")
            i = j + 1
            continue
        if char == "'":
            i += 1
            continue
        i += 1


def scan(root: Path) -> dict[str, list[str]]:
    """Every registry name in a string literal, by file, excluding generated."""
    listing = subprocess.run(
        ["git", "ls-files", "-z", "*.rs"],
        cwd=root, check=True, capture_output=True, text=True,
    ).stdout
    found: dict[str, list[str]] = {}
    for name in listing.split("\0"):
        if not name:
            continue
        path = root / name
        try:
            src = path.read_text()
        except (OSError, UnicodeDecodeError):
            continue
        # The marker has to be near the top; a mention of the word further down
        # is prose rather than a claim about the file.
        if GENERATED_MARKER in src[:200]:
            continue
        if name in ALLOWED:
            continue
        hits = [
            hit
            for _, text in literals(src)
            for hit in NAMESPACE.findall(text)
        ]
        if hits:
            found[name] = sorted(hits)
    return found


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", required=True, type=Path)
    ap.add_argument("--baseline", required=True, type=Path)
    ap.add_argument("--write", action="store_true", help="rewrite the baseline")
    args = ap.parse_args()

    found = scan(args.root)
    total = sum(len(v) for v in found.values())

    if args.write:
        args.baseline.write_text(
            json.dumps({"literals": found}, indent=2, sort_keys=True) + "\n"
        )
        print(
            "{} raw registry names in {} files".format(total, len(found)),
            file=sys.stderr,
        )
        return 0

    baseline = json.loads(args.baseline.read_text())["literals"]
    allowed_total = sum(len(v) for v in baseline.values())

    print(
        "{} raw registry names in Rust string literals, {} allowed by the "
        "baseline".format(total, allowed_total),
        file=sys.stderr,
    )

    added: list[tuple[str, str]] = []
    removed: list[tuple[str, str]] = []
    for name in sorted(set(found) | set(baseline)):
        was = list(baseline.get(name, []))
        now = list(found.get(name, []))
        for hit in now:
            if hit in was:
                was.remove(hit)
            else:
                added.append((name, hit))
        removed.extend((name, hit) for hit in was)

    for name, hit in added:
        print("NEW     {}  {}".format(name, hit), file=sys.stderr)
    for name, hit in removed:
        print("GONE    {}  {}".format(name, hit), file=sys.stderr)

    if added:
        print(
            "\n{} raw registry name(s) that were not there before. Every static "
            "registry is a closed enum in\nhyperion_minecraft_proto::generated::registry, "
            "so name the variant instead: a typo or a\nMojang rename is then a compile "
            "error here rather than a None at run time. If the name\nbelongs to a "
            "dynamic registry and genuinely has no enum, add the file to ALLOWED in\n"
            "nix/check-minecraft-literals.py with the reason.".format(len(added)),
            file=sys.stderr,
        )
    if removed:
        print(
            "\n{} raw registry name(s) are gone, so the baseline is looser than the "
            "code and has\nstopped bounding anything. Tighten it with: "
            "nix run .#sync-minecraft-literals".format(len(removed)),
            file=sys.stderr,
        )
    return 1 if added or removed else 0


if __name__ == "__main__":
    raise SystemExit(main())
