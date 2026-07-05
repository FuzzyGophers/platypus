# SPDX-License-Identifier: GPL-2.0-only
# Copyright (C) 2026 The Platypus Authors
#
# Top-level build/orchestration for Platypus. The Rust engine builds and tests with plain
# `cargo` alone (no `just` needed) — this file only ties the cross-language steps together:
# the cbindgen-generated C header, the umbrella quality gate, and the macOS app (delegated to
# apps/PlatypusMac/justfile). `just` and `cbindgen` are optional dev tools, like reuse /
# cargo-deny / lychee: absent tools are detected and skipped, never hard-failed.

# The macOS app has its own build front — `just app::build`, `just app::bundle`, `just app::run`.
mod app "apps/PlatypusMac/justfile"

root := justfile_directory()

# The canonical, committed header (a cbindgen product). The Swift package consumes a copy the
# app build syncs from it (that copy is a gitignored build product — see app::_sync-header).
header := "crates/platypus-ffi/include/platypus.h"
swift_header := "apps/PlatypusMac/Sources/CPlatypusFFI/include/platypus.h"

default:
    @just --list

# Regenerate the C ABI header from the FFI crate (cbindgen) and sync it into the Swift package.
# The single source is the Rust signatures; the header is a committed build product.
gen-header:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cbindgen >/dev/null 2>&1; then
      echo "cbindgen not installed — 'cargo install cbindgen'"; exit 1
    fi
    cd "{{root}}"
    cbindgen --config crates/platypus-ffi/cbindgen.toml --output "{{header}}" crates/platypus-ffi
    cp "{{header}}" "{{swift_header}}"
    echo "generated {{header}} (+ synced Swift copy)"

# Fail if the committed header isn't what cbindgen produces (drift guard for CI + local).
# Skips gracefully when cbindgen isn't installed.
header-check:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cbindgen >/dev/null 2>&1; then
      echo "· skip header-check (cbindgen not installed)"; exit 0
    fi
    cd "{{root}}"
    just gen-header >/dev/null
    # Only the canonical header is version-controlled; the Swift copy is a build product.
    if ! git diff --quiet -- "{{header}}"; then
      echo "ERROR: generated header is stale — run 'just gen-header' and commit the result."
      git --no-pager diff -- "{{header}}"
      exit 1
    fi
    echo "header up to date"

# The Rust-side gate: fmt / clippy / test, then license / deps / doc-links (detect-and-skip
# the optional external tools reuse / cargo-deny / lychee if absent).
rust-check:
    #!/usr/bin/env bash
    set -euo pipefail
    have() { command -v "$1" >/dev/null 2>&1; }
    cd "{{root}}"
    echo "▶ rustfmt";  cargo fmt --all -- --check
    echo "▶ clippy";   cargo clippy --workspace --all-targets --all-features -- -D warnings
    echo "▶ test";     cargo test --workspace --all-features
    if have reuse; then echo "▶ reuse"; reuse lint
    else echo "· skip reuse (pipx install reuse)"; fi
    if have cargo-deny; then echo "▶ cargo-deny"; cargo deny check
    else echo "· skip cargo-deny (brew install cargo-deny)"; fi
    if have lychee; then
      echo "▶ links"
      mds=$(find . -name '*.md' -not -path './target/*' -not -path '*/.build/*' -not -path './.git/*')
      # shellcheck disable=SC2086
      lychee --offline --no-progress $mds
    else echo "· skip lychee (brew install lychee)"; fi

# The full local gate: Rust checks + header freshness + (macOS only) the app build & smoke.
check: rust-check header-check
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{root}}"
    if [ "$(uname)" = "Darwin" ]; then
      just app::build
      just app::test
      just app::smoke
    else
      echo "· skip swift build + test + smoke (macOS only)"
    fi

# The CI entrypoint for the Linux gate job (the macOS job calls `just app::bundle` + smoke).
ci: rust-check header-check
