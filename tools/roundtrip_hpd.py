#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
# Copyright (C) 2026 The Platypus Authors
"""
Platypus — round-trip fidelity harness.

Throwaway Python (NOT the real backend — that's Rust). Purpose: prove that our
serialization assumptions about the SDSx00 file format are LOSSLESS before any
writer is trusted to touch the SD card. CLAUDE.md calls this the core safety
net: read a real file -> deconstruct into our model -> regenerate -> diff
byte-for-byte. The writer is only trustworthy once the diff is clean.

This is the line/field-level gate (the level a typed Rust model must also clear):
    - ASCII decode <-> encode is identity
    - splitting a line on TAB and re-joining on TAB is identity
      (so empty fields / blanked MyId/ParentId columns survive untouched)
    - every line terminator is reproduced EXACTLY: CRLF, a stray bare-LF, or a
      missing final newline. No normalization.

It deliberately does NOT yet round-trip through typed structs — that lands in
Rust. But it locks down the gotchas (the profile.cfg bare-LF, trailing-newline
presence, blank trailing fields) that silently corrupt a scanner file.

Usage:
    python3 roundtrip_hpd.py samples/synthetic/s_000090.hpd
    python3 roundtrip_hpd.py samples/synthetic     # walks dir, all .hpd/.cfg/.avd

Exit code is non-zero if any file fails round-trip, so it can gate CI later.
"""

import sys
import os


def split_terminator(segment: bytes):
    """Split a raw line segment into (payload, terminator).

    terminator is one of b'\\r\\n', b'\\n', or b'' (final line, no newline).
    Everything before it is the payload. A lone b'\\r' with no following b'\\n'
    is treated as payload, not a terminator (the format uses CRLF, not bare CR).
    """
    if segment.endswith(b"\r\n"):
        return segment[:-2], b"\r\n"
    if segment.endswith(b"\n"):
        return segment[:-1], b"\n"
    return segment, b""


def regenerate(raw: bytes) -> bytes:
    """Deconstruct into (payload, terminator) records, pass payload through the
    TAB split/join model, and reassemble. Should be byte-identical to `raw`."""
    out = bytearray()
    start = 0
    n = len(raw)
    while start < n:
        nl = raw.find(b"\n", start)
        if nl == -1:
            segment = raw[start:]
            start = n
        else:
            segment = raw[start:nl + 1]
            start = nl + 1

        payload, term = split_terminator(segment)

        # --- the model under test: ASCII <-> str, TAB split <-> join ---
        text = payload.decode("ascii")            # raises on non-ASCII -> caught below
        fields = text.split("\t")
        rebuilt = "\t".join(fields).encode("ascii")
        # ---------------------------------------------------------------

        out += rebuilt
        out += term
    return bytes(out)


def first_diff(a: bytes, b: bytes):
    """Return (offset, context) of the first differing byte, or None if equal."""
    if a == b:
        return None
    m = min(len(a), len(b))
    i = 0
    while i < m and a[i] == b[i]:
        i += 1
    lo = max(0, i - 20)
    ctx_a = a[lo:i + 20]
    ctx_b = b[lo:i + 20]
    return i, ctx_a, ctx_b


def check_file(path: str) -> bool:
    with open(path, "rb") as f:
        raw = f.read()
    try:
        regen = regenerate(raw)
    except UnicodeDecodeError as e:
        print(f"FAIL {path}\n     non-ASCII byte breaks the ASCII model: {e}")
        return False

    diff = first_diff(raw, regen)
    if diff is None:
        # also surface the terminator profile so a clean pass is informative
        crlf = raw.count(b"\r\n")
        lf = raw.count(b"\n") - crlf
        final = "newline" if raw.endswith(b"\n") else "NO final newline"
        print(f"PASS {path}  ({len(raw)} bytes, {crlf} CRLF, {lf} bare-LF, {final})")
        return True

    off, ctx_a, ctx_b = diff
    print(f"FAIL {path}\n     first diff at byte {off}")
    print(f"       original : {ctx_a!r}")
    print(f"       regenerated: {ctx_b!r}")
    return False


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    target = sys.argv[1]
    paths = []
    if os.path.isdir(target):
        for root, _, files in os.walk(target):
            for name in sorted(files):
                if name.lower().endswith((".hpd", ".cfg", ".avd")):
                    paths.append(os.path.join(root, name))
        if not paths:
            print(f"No .hpd/.cfg/.avd files found under {target}")
            sys.exit(1)
    else:
        paths = [target]

    ok = 0
    for p in sorted(paths):
        if check_file(p):
            ok += 1

    total = len(paths)
    print(f"\nround-trip: {ok}/{total} clean")
    sys.exit(0 if ok == total else 1)


if __name__ == "__main__":
    main()
