<!--
SPDX-License-Identifier: GPL-2.0-only
SPDX-FileCopyrightText: The Platypus Authors
-->

# Architecture

Platypus is a **Rust engine first**, with a thin C FFI and a native macOS UI on top:

```
platypus-core (Rust, zero-dep)  →  platypus-ffi (C ABI)  →  PlatypusMac (SwiftUI)
                                          ↑
                             platypus-serial (nix / termios)
```

## Crates & layers

- **`crates/platypus-core`** — the open backend: byte-exact file `format`, per-model `device`
  profiles (the `SdCardProfile` / `CloneImageProfile` traits on a small cross-class
  `RadioProfile` base, plus `ProfileRegistry`), the typed `model`, `extract`ion + filters,
  `county_geo` placement, `favorites` build/dialect, `card` I/O, and the `provider`/`Dataset`
  canonical model. **No UI, no platform assumptions, dependency-free.** A Linux or Windows
  front-end can build on it.
- **`crates/platypus-ffi`**: a small, hand-rolled C ABI over the core (opaque handles; results
  cross as JSON strings). The cross-platform contract the app links against.
- **`crates/platypus-serial`**: the serial-transport sibling (termios via `nix`, MIT) that
  drives the FT-60 clone read/write, so the core itself stays dependency-free.
- **`apps/PlatypusMac`**: the SwiftUI macOS app. `swift run PlatypusMac --libtest <dir>` is a
  headless check of the whole Swift → FFI → core path.

## Device profiles (the class split)

Radios are described by profiles in one of two device *classes* that converge at the canonical
typed model — the fork is only at the edges (a parser/codec in, a transport out):

- **`RadioProfile`** — the small cross-class base: identity (`id`, `product_name`, `maker`,
  `transport`) + `class()`, and `as_sd_card()` / `as_clone_image()` accessors that hand back the
  class-specific trait, so the registry can hold every radio as one `dyn RadioProfile`.
- **`SdCardProfile`** — a database scanner programmed by an SD-card file format: layout, record
  schemas, the favorites dialect + field defaults, write rules, limits. *(Uniden SDS150.)*
- **`CloneImageProfile`** — a radio programmed by cloning a fixed EEPROM image over serial: the
  clone-transport spec, image detection, capacity, and editable-field options; the binary image
  codec is model-specific. *(Yaesu FT-60R.)*

`RadioClass` is `{ SdCardScanner, CloneImage }`. The `ProfileRegistry` holds one ordered set of
`Box<dyn RadioProfile>` and detects the right profile per input: `detect()` on a file header,
`detect_clone_image()` on the image magic. Both classes produce/consume the same
`provider::Dataset`, so extraction, filters, the map lens, and the round-trip discipline sit
**above** the fork, unchanged. Adding a radio is a new `device/<model>.rs` + a `register()` line
(a codec too, for a clone-image radio).

## Principles

- **Generic core**: the core hard-codes no use case and no model; filters are UI-composed from
  generic primitives, and each file's model is detected via `ProfileRegistry`. Adding a radio is
  a new `crates/platypus-core/src/device/<model>.rs` + a `register()` line.
- **Zero-dep core**: heavier dependencies live in siblings (`platypus-serial`) or future glue
  crates, never in `platypus-core`.
- **Byte-exact round-trip**: any code that writes to a device round-trips
  (`decode → encode == input`) before it ships; it's the writer safety gate.

See [`../CLAUDE.md`](../CLAUDE.md) for the cold-start brief + doc router.
