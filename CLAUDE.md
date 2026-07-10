# Platypus

Platypus is a **Rust engine first**, a zero-dependency core (`platypus-core`) plus a small
C FFI, that also ships a **native macOS (SwiftUI) app**; the same core can back a
Linux or Windows front-end. It's a location-first programming manager for **radios**, with a
pluggable per-model device-profile system (a new radio is a profile, not a rewrite). The
product story (what "location-first" buys) is in [`README.md`](README.md).

Supported radios live in [`docs/radios/`](docs/radios/) — an index plus one device reference
per radio (today: the Uniden **SDS150** SD-card scanner and the Yaesu **FT-60R** clone-image
HT). This file is **model-agnostic**: device specifics (file-format/codec details, the
favorites dialect, write rules, limits, spec URLs) live in the per-device doc, enforced in
code by the active profile (an `SdCardProfile` or `CloneImageProfile`).

This file is the **cold-start brief + doc router**. Read it, skim the linked docs for
detail, and you can begin work without re-deriving the decisions below.

## Documentation map

| Doc | What's in it |
|---|---|
| [`README.md`](README.md) | Public intro, the name story, architecture, build, license. |
| [`docs/architecture.md`](docs/architecture.md) | The crate/layer breakdown (core → FFI → app, + `platypus-serial`), the **device-profile class split** (`RadioProfile` base + `SdCardProfile`/`CloneImageProfile`), and the core principles. |
| [`docs/capabilities.md`](docs/capabilities.md) | **What Platypus can do today, per device** — the living capability list (SDS150, FT-60R, cross-device). |
| [`TODO.md`](TODO.md) | Tracked future work (providers, store, per-device gaps, serial live control). |
| [`docs/sources.md`](docs/sources.md) | Data sources (Sentinel files, RadioReference, FCC…), how each maps to the canonical model, RR auth, and the updates/freshness strategy. |
| [`docs/respecting-sources.md`](docs/respecting-sources.md) | How we stay a good API citizen — cache-first, throttled, per-query, facts-only; the short reader-facing version of the etiquette posture. |
| [`docs/radios/`](docs/radios/) | **Per-radio docs.** The folder `README.md` indexes supported radios + how to add one; each `<model>.md` is that radio's device reference (file format/codec, favorites dialect, write rules, limits, spec URLs). Today: [`sds150.md`](docs/radios/sds150.md) (+ [`sds150-display.md`](docs/radios/sds150-display.md)) and [`ft60.md`](docs/radios/ft60.md). |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | How to build/test, the one gate (`just check`), the ground rules (facts-only/GPL, privacy, generic core, round-trip gate), and how to add a radio. |
| [`samples/README.md`](samples/README.md) | The byte-exact fixtures and the recon/round-trip tools in [`tools/`](tools/). |
| [`assets/`](assets/) | Source art — `icon.jpg` (app icon, embedded by `just app::bundle`), `logo.jpg` (README). |
| [`LICENSE`](LICENSE) | **GPL-2.0-only** (GNU GPL v2). Copyleft: derivatives (incl. front-ends on the core) stay GPLv2. |
| [`CREDITS.md`](CREDITS.md) | Credit for external docs/specs + references, and the **facts-only** sourcing policy (facts only, spec-derived, no code copied from any reference). |

## Architecture

```
platypus-core (Rust, zero-dep)  →  platypus-ffi (C ABI)  →  PlatypusMac (SwiftUI)
```

A zero-dep Rust core, a hand-rolled C FFI, a serial-transport sibling
(`platypus-serial`), and the SwiftUI app. The crate/layer breakdown + principles live in
[`docs/architecture.md`](docs/architecture.md).

**Build system.** The Rust engine is a standalone Cargo workspace — it builds/tests with
`cargo` alone (no extra tooling). The macOS app has its own front driven by
[`just`](https://just.systems): `just app::build` / `app::run` / `app::bundle` (the root
`justfile` + `apps/PlatypusMac/justfile`). The C ABI header
(`crates/platypus-ffi/include/platypus.h`) is a **cbindgen product** — after changing the
FFI surface, run `just gen-header` and commit it (CI has a freshness gate).

**Quality gate:** `just check` runs the whole thing — `cargo fmt` / `clippy -D warnings` /
`test`, `reuse lint` (the repo is REUSE-compliant, GPL-2.0-only), `cargo deny check`
(`deny.toml`), an offline doc-link check, the cbindgen header-freshness gate, and the macOS
app build + `--libtest` smoke. The same fronts run in CI (`.github/workflows/ci.yml`).

## The two device interfaces (both officially documented)

1. **File format — the SD card (bulk programming, the heart of the app).** A relational,
   **geo-tagged** card layout (Country→State→County and Trunk/Conventional→Site/Group→
   Channel, with lat/lon/range on the geo records), which is what makes location-first
   essentially free. The exact syntax, record vocabulary, and layout are per-model
   (see the device doc); validated byte-exact against real hardware.
2. **Serial protocol — live control (optional).** Status/control only; bulk
   programming is the file format above.

Per-model specifics (format version, model folder, serial id, the favorites dialect)
live in the device doc, e.g. [`docs/radios/sds150.md`](docs/radios/sds150.md).

## Critical rules (don't violate these)

- **Eject before reconnect.** macOS buffers FAT writes — an `Ok` ≠ bytes on the card.
  Every write `fsync`s, and the app drives an eject and never reports success until it
  confirms. The card-write gotcha #1.
- **Byte-exact round-trip is the writer safety gate** (`tests/roundtrip.rs`,
  `tools/roundtrip_hpd.py`). Selection-only output round-trips; synthesis is gated.
- **Device-specific write rules are enforced by the active `SdCardProfile`** and
  documented per radio (e.g. the SDS150 must delete `app_data.cfg` after a write, uses
  the misspelled `discvery.cfg`, and has a favorites *dialect* — not a subset). Don't
  hard-code any of this outside the profile; see [`docs/radios/sds150.md`](docs/radios/sds150.md).
- **Privacy.** Nothing in the codebase (code *or* tests) may reference the owner's
  real home location (their state, county, or city). Value-asserting tests use
  **fictional synthetic fixtures**; the real card/radio dumps are opaque round-trip inputs
  only (gitignored; the round-trip walk globs files, names none). Discussing the owner's data in
  conversation is fine; hard-coding it in the repo is not. See the memory note
  `generic-core-no-usecase-hardcoding`.
- **Generic core.** The core hard-codes no use case and no model. Filters are UI-composed
  from generic primitives (`Dataset::select`, `Extraction::select`); the model is
  detected from each file's header via `ProfileRegistry` — **adding a scanner is a new
  `device/<model>.rs` + a `register()` line**, no FFI/app changes.
- **Facts-only sourcing → implementation stays GPLv2.** From any external reference we take
  only **facts** — hardware specs, file/record layouts, memory offsets, protocol constants.
  Facts aren't copyrightable, so referencing them creates no derivative work and **Platypus
  stays GPL-2.0-only.** Never copy a reference's *expression* (source code, its struct/enum
  definitions, string literals, algorithm structure), whatever its license. Codecs are
  **spec-derived**. See [`CREDITS.md`](CREDITS.md).
- **Logical, atomic commits.** Each commit is one self-contained, understandable change —
  small enough to review at a glance, with a message that explains it. Don't batch unrelated
  edits together; split a large change into a sequence of focused commits. Every commit should
  build and pass `just check` on its own. Commit (and push) only when asked; sign off with
  `git commit -s`. **Message format:** a concise subject line, then a blank line, then a
  bulleted body — one `- ` bullet per notable change:

  ```
  <subject: what this commit does>

  - change X
  - change Y
  ```

Current capabilities are listed in [`docs/capabilities.md`](docs/capabilities.md); planned
work is tracked in [`TODO.md`](TODO.md).
