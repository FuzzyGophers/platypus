# PlatypusMac — native macOS client

A SwiftUI app over the Rust `platypus-core`, via the C ABI in `platypus-ffi`.
The core stays the single source of truth; Swift only renders and calls in.

```
platypus-core (Rust)  →  platypus-ffi (C ABI, staticlib)  →  PlatypusMac (SwiftUI)
```

## Build & run

Driven by [`just`](https://just.systems) from the repo root (or `just` in this dir):

```sh
just app::build       # sync the header, build the Rust staticlib, then swift build
just app::run         # build + launch the app (needs a desktop session)
just app::bundle      # assemble Platypus.app (signed if CODESIGN_IDENTITY is set)
```

Headless smoke test (no window, verifies the whole Swift→FFI→core path):

```sh
just app::smoke       # runs `PlatypusMac --libtest ../../samples/synthetic`
```

## What it does today

Location-first programming across two radio classes, all through the Rust core:

- **Browse the catalog** from a loaded HPDB library — filter systems by service type,
  technology, county, or a point + radius (lat/lon/miles) on a map, with the geo work done
  in core.
- **Program an SDS150 SD card** — auto-detect a mounted card, read its favorites lists, and
  edit them: build/append lists from the catalog, per-channel settings (avoid, priority,
  alerts, delay…), reorder, and write back, with backup/restore and a driven eject.
- **Program a Yaesu FT-60R** — read the radio over a serial clone cable, edit channels
  (frequency, mode, tone, power, step, duplex, banks), and write back; edits are byte-exact
  round-trip-gated in core before anything touches the radio.
- **Manage "my radios"** — a persisted owned-radios set with a neutral first run.

## Layout

`Sources/PlatypusMac/` groups by concern:

- `App/`: entry, app, root UI, theme, the `RadioModel`/owned-radios store.
- `Bridge/`: thin typed Swift wrappers over the C ABI (`PlatypusCore`, `Library`, `Radios`,
  `Ft60`, `Ft60Options`, `Write`); each owns a Rust handle and decodes the JSON results.
- `Card/`: SDS150 card detect, favorites lists, backup/restore, eject.
- `Catalog/`: the browse/filter/map UI, service-type presentation, data-source credits.
- `FT60/`: the clone-image editor (read/edit/write sheets + the memory model).
- `Sources/CPlatypusFFI/`: C module exposing `platypus.h` (a cbindgen product, synced from
  the crate by `just app::build`) so Swift can `import CPlatypusFFI`.
- `Info.plist`: committed bundle manifest; `just app::bundle` copies it into the `.app` and
  stamps `CFBundleVersion`/`CFBundleShortVersionString` from the workspace Cargo version.

## Notes / next

- Linking uses `-L../../target/release` (release staticlib — the full-USA parse is
  CPU-bound). `just app::bundle` assembles a `.app`; signing/notarization is a seam
  (`CODESIGN_IDENTITY`) wired but not yet enabled.
- The app is **unsandboxed and unsigned by default** — it needs raw file/serial access to
  program devices. Distribution builds will be signed (see the bundle seam).
