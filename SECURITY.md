# Security Policy

## Supported versions

Only the **latest release** is supported. A security fix ships as its own **point (patch)
release** on the current release line (and lands on `master`); older versions aren't
back-patched — upgrade to the newest release to get the fix.

| Version | Supported |
|---|---|
| Latest release | ✅ |
| Older | ❌ |

## Reporting a vulnerability

Please report security issues **privately** — do not open a public issue for a vulnerability.

Use GitHub's **private vulnerability reporting** for this repository:
**Security → Report a vulnerability**
(<https://github.com/FuzzyGophers/platypus/security/advisories/new>).

Include what you'd expect: affected version/commit, a description, reproduction steps, and the
impact. We'll acknowledge the report, work on a fix, and coordinate disclosure with you. As a
small volunteer project we can't promise a fixed timeline, but we take reports seriously.

## Scope notes

Platypus programs real hardware, so a few things are inherent rather than vulnerabilities:

- **It writes raw SD cards and drives serial radios**, and the macOS app is **unsandboxed** by
  design to do so. Only point it at your own devices and cards.
- **Device writes are gated by a byte-exact round-trip** in the core; selection-only output is
  reproduced verbatim, and synthesis is gated. A write that could corrupt a card/radio image
  bypassing that gate *is* in scope.
- The core parses untrusted external files (card dumps, clone images); a parser crash or memory
  unsafety on malformed input **is** in scope.
