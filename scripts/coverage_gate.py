#!/usr/bin/env python3
"""Line coverage gate per crate.

CLAUDE.md, section 5, asks for at least 80 % overall (checked by
`cargo llvm-cov --fail-under-lines 80`) and "clearly higher" for the domain
and the use cases. This script checks those higher thresholds.

Usage: cargo llvm-cov report --json --summary-only | scripts/coverage_gate.py
"""

import json
import sys

# Minimum line coverage in percent, per crate directory under `crates/`.
THRESHOLDS = {
    "domain": 95.0,
    "application": 90.0,
}


def coverage_per_crate(report):
    """Sum covered and total lines of every file per crate directory."""
    totals = {}
    for file in report["data"][0]["files"]:
        parts = file["filename"].split("/crates/", 1)
        if len(parts) != 2:
            continue
        crate = parts[1].split("/", 1)[0]
        lines = file["summary"]["lines"]
        covered, count = totals.get(crate, (0, 0))
        totals[crate] = (covered + lines["covered"], count + lines["count"])
    return totals


def main():
    totals = coverage_per_crate(json.load(sys.stdin))
    failed = False
    for crate, minimum in THRESHOLDS.items():
        covered, count = totals.get(crate, (0, 0))
        percent = 100.0 * covered / count if count else 0.0
        ok = percent >= minimum
        failed |= not ok
        mark = "ok" if ok else "TOO LOW"
        print(f"{crate:12} {percent:6.2f} % (minimum {minimum:.0f} %) {mark}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
