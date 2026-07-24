#!/usr/bin/env python3
"""One-time migration: convert raw ride_length to human-readable metres.

Before the report builder was fixed, report.json stored ride_length as the
game's raw 16.16 fixed-point total (e.g. 25539696). The game itself shows the
value >> 16 (ToHumanReadableRideLength in src/openrct2/core/UnitConversion.cpp),
so 25539696 is really 389 metres. New runs write metres directly; this rewrites
the raw values already committed in old report.json files.

Surgical by design: it regex-replaces only the `"ride_length": N` number, so
every other byte of each report stays identical and git diffs stay clean.

Idempotent: a real coaster's raw length is always well above 2^16 (even a
one-metre ride is 65536 raw), and a converted length is always well below it
(no ride is 65000 metres), so the > 2^16 guard both selects raw values and
refuses to shift an already-converted file twice.

Usage:
    uv run evals/migrate_ride_length.py            # dry run, shows changes
    uv run evals/migrate_ride_length.py --apply    # write the changes
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

# 2^16: the boundary between raw fixed-point (millions) and metres (hundreds).
RAW_THRESHOLD = 1 << 16
RIDE_LENGTH = re.compile(r'("ride_length":\s*)(\d+)')


def convert_text(text: str) -> tuple[str, list[tuple[int, int]]]:
    """Return the rewritten text and the (raw, metres) pairs it changed."""
    changes: list[tuple[int, int]] = []

    def repl(match: re.Match[str]) -> str:
        raw = int(match.group(2))
        if raw <= RAW_THRESHOLD:
            return match.group(0)  # already metres (or zero); leave it
        metres = raw >> 16
        changes.append((raw, metres))
        return f"{match.group(1)}{metres}"

    return RIDE_LENGTH.sub(repl, text), changes


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=Path, default=Path("evals/runs"))
    parser.add_argument("--apply", action="store_true", help="write changes (default: dry run)")
    args = parser.parse_args()

    reports = sorted(args.runs.rglob("report.json"))
    changed_files = 0
    total_changes = 0
    for report in reports:
        text = report.read_text()
        new_text, changes = convert_text(text)
        if not changes:
            continue
        changed_files += 1
        total_changes += len(changes)
        rel = report.relative_to(args.runs.parent.parent) if args.runs.is_absolute() else report
        for raw, metres in changes:
            print(f"  {rel}: {raw} -> {metres} m")
        if args.apply:
            report.write_text(new_text)

    verb = "converted" if args.apply else "would convert"
    print(f"\n{verb} {total_changes} ride_length value(s) across {changed_files} file(s)"
          f" (of {len(reports)} report.json scanned)")
    if not args.apply and changed_files:
        print("re-run with --apply to write")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
