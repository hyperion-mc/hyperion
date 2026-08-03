#!/usr/bin/env python3
"""What the fleet cost in the last 24 hours, in dollars, per VM.

Why this exists: `ix billing usage` answers for the whole account, and the
account also carries eval runners and scratch VMs. The question during an
event is narrower: what are these four machines costing right now. The
server already attributes every usage event to a VM (`resource_spend` in
the summary), so this is a filter and a sum, not a new data source.

Run it from any machine where `ix` is logged in:

    python3 nix/fleet/spend.py            # last 24h, VMs matching hyperion-
    python3 nix/fleet/spend.py --since 7d
    python3 nix/fleet/spend.py --prefix pumpkin-

A billing account must exist for the user; without one, `ix billing usage`
fails with "billing account has not been provisioned" and this script
repeats that error and exits nonzero rather than printing zeros that would
read as "the fleet is free".
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys

# crates/ix/db/src/billing/money.rs: MICROCREDITS_PER_USD = 1_000_000.
_MICROCREDITS_PER_USD = 1_000_000


def _usage_json(since: str) -> dict[str, object]:
    """Fetch the account usage summary, failing loudly with ix's own error."""
    result = subprocess.run(
        ["ix", "billing", "usage", "--since", since, "--json"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        raise SystemExit(result.returncode)
    return json.loads(result.stdout)


def _dollars(microcredits: int) -> str:
    return f"${microcredits / _MICROCREDITS_PER_USD:,.4f}"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--since", default="24h", help="window, e.g. 24h, 7d (default 24h)")
    parser.add_argument(
        "--prefix",
        default="hyperion-",
        help="count VMs whose name starts with this (default hyperion-)",
    )
    args = parser.parse_args()

    summary = _usage_json(args.since)
    rows = summary.get("resource_spend")
    if not isinstance(rows, list):
        raise SystemExit(
            "no resource_spend in `ix billing usage --json`; the CLI schema moved"
        )

    fleet: list[tuple[str, int, int]] = []
    other_total = 0
    for row in rows:
        name = row.get("resource_name") or row.get("resource_id") or "?"
        cost = int(row["cost_microcredits"])
        if str(name).startswith(args.prefix):
            fleet.append((str(name), cost, int(row["event_count"])))
        else:
            other_total += cost

    fleet.sort(key=lambda entry: -entry[1])
    labels = ("fleet total", "rest of account")
    width = max(*(len(label) for label in labels), *(len(name) for name, _, _ in fleet or [("", 0, 0)]))
    print(f"fleet spend, last {args.since} (prefix {args.prefix!r})")
    for name, cost, events in fleet:
        print(f"  {name:<{width}}  {_dollars(cost):>12}  {events} events")
    if not fleet:
        # A prefix that matches nothing looks identical to a free fleet;
        # say which it is.
        print(f"  (no resources matching {args.prefix!r} in this window)")
    fleet_total = sum(cost for _, cost, _ in fleet)
    print(f"  {'fleet total':<{width}}  {_dollars(fleet_total):>12}")
    print(f"  {'rest of account':<{width}}  {_dollars(other_total):>12}")


if __name__ == "__main__":
    main()
