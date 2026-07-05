# Radios

Index of supported radios. Each has its own device reference in this folder
(`<model>.md`); add a radio by dropping a new doc here plus a row in the table below.

Platypus is a location-first programming manager for **radios**, built on a pluggable
device-profile system rather than around any one model. Each radio is a
[`RadioProfile`](../../crates/platypus-core/src/device/profile.rs) registered in a
`ProfileRegistry`, in one of two device *classes*: an `SdCardProfile` (a database scanner
programmed by an SD-card file format) or a `CloneImageProfile` (a radio programmed by cloning
a fixed EEPROM image over serial). The core picks the right profile from each file's header /
image magic. Adding a radio is a new `device/<model>.rs` + a `register()` line — **no FFI or
app changes**.

## Currently supported

| Radio | Class / format | Reference | Status |
|---|---|---|---|
| **Uniden SDS150** | SD-card scanner: `BCDx36HP` DMA format (FormatVersion 1.00) | [`sds150.md`](sds150.md) | File format validated byte-exact against real hardware; favorites build + edits hardware-validated. Serial live-control is a later phase. |
| **Yaesu FT-60R** | Clone-image HT: binary EEPROM image over a serial clone cable | [`ft60.md`](ft60.md) | Read, edit, and write channels; the codec is byte-exact round-trip-gated, and edit-and-write is verified against real hardware. A different device class (no filesystem) — see the trait split in [`docs/architecture.md`](../architecture.md). |

The `BCDx36HP` model folder is shared across Uniden's DMA scanners (SDS100/SDS200/
BCD325P2/BCD436HP…), so other models in that family are largely a profile away.

## Adding a device

1. Add `crates/platypus-core/src/device/<model>.rs` implementing `RadioProfile` plus the
   sibling trait for its class: `SdCardProfile` (SD-card layout, record schemas, favorites
   dialect/field defaults, limits) or `CloneImageProfile` (clone transport spec, image codec,
   capacity).
2. `register()` it with the `ProfileRegistry` so detection can find it.

Everything above the profile (extraction, location-first placement, filters, favorites
build, card/serial I/O, the FFI, and the UI) is generic and unchanged.
