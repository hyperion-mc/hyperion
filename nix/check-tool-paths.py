"""Fail when a script names a repository path that does not exist.

A Python tool reaching into `crates/.../src/generated/registry.rs` is a
dependency the compiler cannot see and `cargo` cannot break. When that file
became a directory, two scripted clients kept building, kept passing review and
failed only inside four end-to-end gates, twenty minutes into CI, with a
`FileNotFoundError` that named a path nobody had grepped for because it was in
a string in a language nothing type-checks against the tree.

This is the cheap half of the fix. The real half is that those two now read
`protocol.json`, which is a data interface rather than somebody else's source
file. But the class outlives the instance: any script may name any path, and
this makes a stale one fail in seconds rather than in a gate.

Only paths anchored at a top-level directory of this repository are checked, so
`"target/debug"` and `"a/b.rs"` in prose are not mistaken for claims.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

# A string literal is treated as a claim about the tree when it starts with one
# of these. Anchoring on real directories is what keeps a doc string mentioning
# `some/path` out of the results.
ANCHORS = ("crates/", "events/", "nix/", "tools/", "docs/", "assets/")

# `crates/hyperion/src/lib.rs`, but not `crates/{}/src` or `crates/%s/x`.
CANDIDATE = re.compile(
    r"""["']((?:%s)[A-Za-z0-9_./-]+)["']""" % "|".join(re.escape(a) for a in ANCHORS)
)

# Where a path is deliberately not a file: a glob, or a name built at runtime.
PLACEHOLDER = re.compile(r"[{}%*?]|\$\{")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", required=True, type=Path)
    args = ap.parse_args()

    listing = subprocess.run(
        ["git", "ls-files", "-z", "*.py", "*.sh", "*.nix"],
        cwd=args.root, check=True, capture_output=True, text=True,
    ).stdout

    broken: list[tuple[str, int, str]] = []
    checked = 0
    for name in listing.split("\0"):
        if not name:
            continue
        try:
            text = (args.root / name).read_text()
        except (OSError, UnicodeDecodeError):
            continue
        for number, line in enumerate(text.splitlines(), 1):
            for claim in CANDIDATE.findall(line):
                if PLACEHOLDER.search(claim):
                    continue
                checked += 1
                if not (args.root / claim).exists():
                    broken.append((name, number, claim))

    print(
        "{} repository paths named in scripts, {} of them missing".format(
            checked, len(broken)
        ),
        file=sys.stderr,
    )
    for name, number, claim in broken:
        print("MISSING {}:{}  {}".format(name, number, claim), file=sys.stderr)
    if broken:
        print(
            "\nA script names a path this tree does not have. Nothing else catches "
            "this: a path in a\nstring is invisible to cargo and to every Rust "
            "grep, so it fails inside whichever gate runs\nthat script. Either fix "
            "the path, or -- better -- stop reaching into another component's\n"
            "source and read its committed data instead.",
            file=sys.stderr,
        )
    return 1 if broken else 0


if __name__ == "__main__":
    raise SystemExit(main())
