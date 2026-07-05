<!--
SPDX-License-Identifier: GPL-2.0-only
SPDX-FileCopyrightText: The Platypus Authors
-->

# Contributing to Platypus

Thanks for your interest. Platypus is a location-first programming manager for radios: a
zero-dependency Rust core, a small C ABI, and a SwiftUI macOS app. This page covers how to
build it, the one gate every change must pass, and a few rules that keep the project
spec-derived and license-safe.

Start with [`CLAUDE.md`](CLAUDE.md) — the cold-start brief and doc router, then
[`docs/architecture.md`](docs/architecture.md) and the per-radio docs in
[`docs/radios/`](docs/radios/).

## Dev setup

- **Rust** — the toolchain is **pinned** in `rust-toolchain.toml` (channel + `rustfmt`/`clippy`);
  `rustup` installs it automatically on first `cargo` invocation, so local and CI builds match.
  The Rust engine builds/tests on Linux/macOS with **only `cargo`** — no other tooling.
- **macOS + Xcode / Swift** for the app (`apps/PlatypusMac`), plus
  [`just`](https://just.systems) (`brew install just`) to drive its build.
- Optional gate tools: [`cbindgen`](https://github.com/mozilla/cbindgen)
  (`cargo install cbindgen`, for the C header), [`reuse`](https://reuse.software)
  (`pipx install reuse`), `cargo-deny` and `lychee` (`brew install cargo-deny lychee`). The
  gate detect-and-skips any that are absent.

```sh
cargo test --workspace                 # the Rust engine — cargo alone
just app::bundle                        # build the macOS app bundle (needs just)
open apps/PlatypusMac/Platypus.app      # run it
```

## The one gate

Before opening a PR, run the full local quality gate — the same checks CI runs:

```sh
just check
```

It runs `rustfmt`, `clippy -D warnings`, `cargo test`, REUSE license compliance,
`cargo-deny`, an offline doc-link check, a cbindgen header-freshness gate, and (on macOS)
the app build + a headless `--libtest` smoke. **PRs are expected to be gate-green.** If you
change the FFI surface, run `just gen-header` and commit the regenerated
`crates/platypus-ffi/include/platypus.h`. Keep changes small and focused.

## Ground rules

- **Facts-only, GPL-2.0-only.** Platypus is [GPL-2.0-only](LICENSE). From any external
  reference we take **only facts**: hardware specs, file/record layouts, memory offsets,
  protocol constants (facts aren't copyrightable). **Never copy a reference's *expression***
  (source code, struct/enum definitions, string literals, algorithm structure), whatever its
  license. Codecs are derived from the published spec, not its code. See [`CREDITS.md`](CREDITS.md).
- **Privacy.** Nothing in the repo (code *or* tests) may reference a real person's home
  location. Value-asserting tests use **fictional synthetic fixtures**; real card/radio
  dumps are gitignored, opaque round-trip inputs only.
- **Generic core.** `platypus-core` hard-codes no use case and no model. Filters are
  UI-composed from generic primitives; models are detected via the profile registry.
- **Byte-exact round-trip is the writer safety gate.** Any code that writes to a device
  must round-trip (`decode → encode == input`) before it ships.

## Adding a radio

A new model is a **new profile, not a rewrite**: add
`crates/platypus-core/src/device/<model>.rs` and a `register()` line — no FFI or app
changes for an SD-card scanner. See [`docs/architecture.md`](docs/architecture.md)
for how the profile traits split (SD-card vs clone-image classes) and
[`docs/radios/`](docs/radios/) for how to document it. Outstanding work is tracked in
[`TODO.md`](TODO.md).

## Commits & PRs

- **Sign your commits.** GPG-signed commits are preferred (this repo signs by default);
  include a `Signed-off-by:` line (DCO) — `git commit -s`.
- Write clear commit messages; reference an issue when one exists.
- One logical change per PR; make sure `just check` passes.

## License

By contributing, you agree that your contributions are licensed under the project's
**GNU General Public License v2.0 only** ([`LICENSE`](LICENSE)).
