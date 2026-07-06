#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
# Copyright (C) 2026 The Platypus Authors
"""
Platypus — SD card recon tool.

Throwaway Python (NOT the real backend — that's Rust). Purpose: read the actual
.hpd / .cfg files off your SDS150's SD card and report how they line up with the
SDSx00 File Specification V1.03, BEFORE you commit to any Rust data structures.

Usage:
    python3 inspect_hpd.py /path/to/file.hpd
    python3 inspect_hpd.py /Volumes/SDS150/SDS150/HPDB/hpdb.cfg
    python3 inspect_hpd.py /Volumes/<card>            # scans a whole mounted card

What it tells you:
    - Is the file really tab-delimited ASCII, one sentence per line?
    - Which record/command types appear, and how often.
    - Field counts per record type (to spot SDS150 fields the 2019 spec lacks).
    - The header sentences (TargetModel / FormatVersion) so you can confirm the
      model + format version your unit actually writes.
"""

import sys
import os
from collections import Counter, defaultdict

# Record types defined in V1.03 (the ones we expect to see).
KNOWN_RECORDS = {
    "TargetModel", "FormatVersion", "Conventional", "Trunk", "AreaState",
    "AreaCounty", "FleetMap", "UnitIds", "AvoidTgids", "Site", "Rectangle",
    "BandPlan_Mot", "BandPlan_P25", "DQKs_Status", "C-Group", "T-Group",
    "C-Freq", "TGID", "T-Freq", "Scanner", "DateModified", "StateInfo",
    "County", "CountyInfo", "LM", "LM_Frequency",
    # profile.cfg display-customization records (see docs/radios/sds150-display.md).
    "DisplayOption", "Backlight", "DispOptItems", "DispColors",
}


def analyze_file(path):
    with open(path, "rb") as f:
        raw = f.read()

    print(f"\n=== {path} ===")
    print(f"size: {len(raw)} bytes")

    # Encoding / ASCII check
    non_ascii = [b for b in raw if b > 0x7E or (b < 0x20 and b not in (0x09, 0x0A, 0x0D))]
    if non_ascii:
        print(f"  WARNING: {len(non_ascii)} non-ASCII/control bytes (spec says ASCII).")
    else:
        print("  encoding: clean ASCII (matches spec)")

    # Line endings
    crlf = raw.count(b"\r\n")
    lf_only = raw.count(b"\n") - crlf
    print(f"  line endings: {crlf} CRLF, {lf_only} bare-LF")

    text = raw.decode("ascii", errors="replace")
    lines = [ln for ln in text.splitlines() if ln.strip()]
    print(f"  non-empty lines: {len(lines)}")

    tab_lines = sum(1 for ln in lines if "\t" in ln)
    print(f"  tab-delimited lines: {tab_lines}/{len(lines)} "
          f"({'tab format confirmed' if tab_lines else 'NO TABS — unexpected!'})")

    # Record types: first tab-separated token of each line
    record_counts = Counter()
    field_counts = defaultdict(set)
    headers = []
    for ln in lines:
        parts = ln.split("\t")
        cmd = parts[0].strip()
        record_counts[cmd] += 1
        field_counts[cmd].add(len(parts))
        if cmd in ("TargetModel", "FormatVersion", "Scanner", "DateModified"):
            headers.append(ln.replace("\t", " | "))

    if headers:
        print("  header sentences:")
        for h in headers:
            print(f"    {h}")

    print("  record types found:")
    for cmd, n in record_counts.most_common():
        known = "" if cmd in KNOWN_RECORDS else "  <-- NOT in V1.03 spec!"
        widths = sorted(field_counts[cmd])
        print(f"    {cmd:<16} x{n:<6} fields={widths}{known}")

    # Surface the first example of each record so you can eyeball real values.
    seen = set()
    print("  one example per record type:")
    for ln in lines:
        cmd = ln.split("\t")[0].strip()
        if cmd not in seen:
            seen.add(cmd)
            shown = ln if len(ln) <= 160 else ln[:157] + "..."
            print(f"    {shown.replace(chr(9), '<TAB>')}")


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    target = sys.argv[1]
    if os.path.isdir(target):
        # Walk a mounted card; analyze every .hpd / .cfg / .avd
        found = False
        for root, _, files in os.walk(target):
            for name in sorted(files):
                if name.lower().endswith((".hpd", ".cfg", ".avd")):
                    found = True
                    analyze_file(os.path.join(root, name))
        if not found:
            print(f"No .hpd/.cfg/.avd files found under {target}")
    else:
        analyze_file(target)


if __name__ == "__main__":
    main()
