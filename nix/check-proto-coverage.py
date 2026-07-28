"""Hold the protocol extractor's coverage gap against a committed baseline.

`protocol.json` carries a `coverage.incomplete` list: every packet and data
component whose byte layout the extractor could not recover in full, each with
the field path that stopped it. Those entries are the reason a packet has to be
hand-written or does not exist, and until this check there was nothing anywhere
that reported them. Sixty-five had accumulated before anybody counted.

So this is a ratchet, not a target of zero. It fails when the list grows, which
is a regression somebody just introduced, and it also fails when the list
shrinks, because a baseline nobody tightens stops being a bound. Both are fixed
by the same command, and the count can only go down.

The histogram is printed on every run, pass or fail. A number alone says a gap
exists; the grouping says whether the answer is one generator change or forty
separate ones.
"""

from __future__ import annotations

import argparse
import collections
import json
import re
import sys
from pathlib import Path

# Reasons grouped by what a fix would have to change, rather than by their text.
# The order matters: the first pattern that matches wins, so the specific
# `branching` and `unmodelled statement` shapes come before their catch-alls.
CAUSES: list[tuple[str, str]] = [
    (r"dispatched codec", "runtime-dispatched union"),
    (r"branching encode body: if \(\w*[iI]temStack\.isEmpty\(\)\)", "optional ItemStack"),
    (r"branching encode body: if \(patch\.isEmpty\(\)\)", "DataComponentPatch"),
    (r"branching encode body: (?:if \(this\.\w+\) \{\s*(?:bitfield|flags)|.*flags \|= )", "packed bitfield"),
    (r"branching encode body: for ", "loop over a sequence"),
    (r"branching encode body", "conditional presence"),
    (r"unmodelled factory CustomPacketPayload\.codec", "CustomPacketPayload"),
    (r"unmodelled factory", "unmodelled codec factory"),
    (r"unmodelled statement", "unmodelled statement"),
    (r"", "other"),
]


def cause(reason: str) -> str:
    # The field path is prefixed to the reason, so the reason itself starts
    # after the first ": " that a path could have introduced.
    text = reason.split(": ", 1)[1] if re.match(r"^[\w\[\]?.|<>]+: ", reason) else reason
    for pattern, label in CAUSES:
        if re.search(pattern, text):
            return label
    return "other"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--protocol", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--write", action="store_true", help="rewrite the baseline instead of checking it")
    args = parser.parse_args()

    coverage = json.loads(args.protocol.read_text())["coverage"]
    entries = coverage["incomplete"]
    found = {entry["id"]: entry["reasons"] for entry in entries}

    histogram = collections.Counter(cause(entry["reasons"][0]) for entry in entries)
    print(f"{len(found)} layouts not recovered in full, by cause:", file=sys.stderr)
    for label, count in sorted(histogram.items(), key=lambda kv: (-kv[1], kv[0])):
        print(f"  {count:3d}  {label}", file=sys.stderr)

    if args.write:
        body = {
            "comment": (
                "Layouts nix/extract-protocol.py cannot recover in full, held by "
                "nix flake check .#minecraft-proto-coverage. Regenerate with "
                "nix run .#sync-minecraft-proto."
            ),
            "count": len(found),
            "incomplete": sorted(found),
        }
        args.baseline.write_text(json.dumps(body, indent=2) + "\n")
        print(f"wrote {args.baseline} with {len(found)} entries", file=sys.stderr)
        return 0

    baseline = set(json.loads(args.baseline.read_text())["incomplete"])
    added = sorted(set(found) - baseline)
    removed = sorted(baseline - set(found))

    for entry_id in added:
        print(f"NEW GAP {entry_id}", file=sys.stderr)
        for reason in found[entry_id]:
            print(f"          {reason}", file=sys.stderr)
    for entry_id in removed:
        print(f"CLOSED  {entry_id}", file=sys.stderr)

    if added:
        print(
            f"\n{len(added)} layout(s) stopped being recoverable. Each one is a packet that now "
            "has to be hand-written or does not exist, so fix the extractor rather than widening "
            "the baseline.",
            file=sys.stderr,
        )
        return 1
    if removed:
        print(
            f"\n{len(removed)} layout(s) are now recovered, so the baseline is looser than the "
            "code. Tighten it with: nix run .#sync-minecraft-proto",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
