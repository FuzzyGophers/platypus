# TODO

Tracked future work. Run `just check` before pushing (see `CLAUDE.md`).

## Engineering / hardening

- **Graceful FFI panic recovery.** The boundary is sound today: the core is written not to
  panic on malformed input, `extern "C"` aborts on unwind (Rust ≥ 1.81), and release sets
  `panic = "abort"`. A future refinement is to wrap each `extern "C"` entry point in
  `catch_unwind` so an unexpected panic degrades to the null/`0` return (a UI error) instead of
  aborting the app. Low priority while the core stays panic-free.
- **Unify the core error type.** `core::Error` (`error.rs`) covers only the parse path; card
  read/write returns `std::io::Error`. The FFI stringifies both, so callers see consistent
  messages, but a single crate error type would be cleaner.

## Release / distribution

- **Sign + notarize the macOS app.** `just app::bundle` has a `_sign` seam (runs `codesign`
  only when `CODESIGN_IDENTITY` is set; a no-op otherwise, so dev builds stay unsigned). For a
  distributable build, wire a Developer ID: `codesign --options runtime`, then
  `xcrun notarytool submit … --wait` + `xcrun stapler staple` (stub commands are already in
  `apps/PlatypusMac/justfile`). Needs an Apple Developer account + a notary credential profile.

## UI / data

- **Map: "center on me"** (follow-up to the ZIP/place jump, which is done). Add a
  current-location button using `CLLocationManager`; wire location permission by adding
  `NSLocationWhenInUseUsageDescription` to `apps/PlatypusMac/Info.plist`. (The app is
  unsandboxed, so no sandbox entitlement is needed.)

- **Display customization (SD write, new area).** The SDS format supports customizing the
  on-screen display, in the scanner's **settings config** (not HPDB/favorites): `DispOptItems`
  (which data items appear per display layout), `DispColors` (per-item text/background colors
  from a named palette), `DisplayOption` (Mot/EDACS TGID format DEC/HEX, Color Mode, 3-line
  mode), `Backlight`. A natural "manager" feature — theme the display, pick what shows. To
  build: identify + parse the settings config file; add a settings editor gated by the
  byte-exact round trip. **Spec-ready**: record formats, layout modes, item-code tables, and
  the 147-color palette are in [`docs/radios/sds150-display.md`](docs/radios/sds150-display.md).

- **RepeaterBook.com provider (import source #3).** A free JSON API of amateur repeaters —
  the natural data source for the ham/**FT-60** flow (conventional freq + offset + CTCSS/DCS
  tone), and it carries **lat/lon** so it drops straight into the map lens + location-first
  browse. Unlike RadioReference it needs **no app key**, so it's unblocked. Endpoints:
  `export.php` (North America) / `exportROW.php` (rest of world), queryable by country/state/
  county/city/callsign/frequency/mode. Build it as a new `Provider` → `Dataset` mapping,
  mirroring the `core::rr` pattern (pure, tested mapping in the core; the HTTP/JSON I/O in a
  `platypus-repeaterbook` crate or the FFI so the core stays zero-dep). Fields to map:
  frequency, input/offset, up/down tones, callsign, use, lat/lon, county/state, mode
  (FM/DMR/D-STAR/Fusion/NXDN/P25), status. **Before building:** verify the current endpoints,
  params, and fields against their live docs, and honor their terms — a descriptive
  `User-Agent`, non-commercial use, and gentle rate-limiting (cache results, don't hammer).
  Using their *data* via the API (per their terms) is distinct from copying code, so it
  doesn't affect the GPL-2.0-only license. See the provider model in
  [`crates/platypus-core/src/provider.rs`](crates/platypus-core/src/provider.rs) and the RR
  sketch in [`crates/platypus-core/src/rr.rs`](crates/platypus-core/src/rr.rs).

- **RadioReference web-service client (provider #2).** Types + canonical mapping are sketched in
  [`crates/platypus-core/src/rr.rs`](crates/platypus-core/src/rr.rs); **blocked on an RR app
  key** (free, issued by RR support). Would also fill the band-plan + service-name gaps. Same
  `Provider` → `Dataset` shape as RepeaterBook; the wire format is SOAP/XML, so the HTTP/XML I/O
  lives in a `platypus-rr` crate (or the FFI) to keep the core zero-dep.

- **SQLite store (cross-source merge + refresh).** A persistent store keyed on
  `(source, id, lastUpdated)` to merge a Sentinel base with RR / RepeaterBook deltas and refresh
  them. Its mandate is **freshness**, not speed; browse is already fast in-memory.

## FT-60 (Yaesu clone-image HT)

Read / write / edit-and-write and the full standard-memory record are **done and byte-exact**
(name, freq, mode FM/NFM/AM, tone + sub-kind, duplex ±, offset, split tx-freq, power, step,
skip, banks). PMS band-edge memories decode + round-trip. Remaining FT-60 capabilities to map:

- **PMS band-edge editing UI.** The codec reads/writes the 100 records (50 L/U pairs at
  `0x40C8`) and round-trips them; the app only **displays** them (read-only "Scan edges"
  section). Add a pair editor once a **programmed sample** confirms the interleaved lower/upper
  pairing (the owner's card has none set, so the ordering is inferred). Core support is in
  `Ft60Image::pms_edges` / `apply_pms`.
- **Power Mid/Low calibration.** Index `0` = High is confirmed on hardware; `1`/`2` = Mid/Low
  are assumed from the documented bit order and **not yet exercised** (all captured channels are
  High). Verify
  by setting one channel to Mid then Low on the radio, Read, and check the 2-bit field. Writes
  are change-gated, so unchanged channels are safe regardless.
- **Verify the single-point calibrations.** `offset` is 50 kHz-per-step, calibrated from the one
  value seen (600 kHz / 2 m); confirm with a **70 cm repeater (5 MHz)**. **Split duplex +
  tx_freq** is implemented but unvalidated (no split channel in captures) — confirm with one.
- **Tone cross-modes** (`Tone->DTCS`, `DTCS->Tone`) carry only the primary CTCSS/DCS value; the
  secondary collapses if such a channel is *edited* (it round-trips verbatim when untouched).
  Carry both values for full fidelity.
- **Name charset — symbols.** `0x00–0x24` (digits / A–Z / space) are mapped; codes above `0x24`
  (punctuation/symbols) render as space and won't round-trip if a name using them is edited. Map
  the rest of the charset (`charset_byte`/`encode_charset` in `device/ft60.rs`).
- **Non-channel radio config (a separate "radio settings editor").** The clone image also holds
  home channels (5), NOAA weather (10), DTMF autodialer memories, set-mode/menu settings (APO,
  squelch, lamp, beep, ARTS, busy-lockout…), and VFO / one-touch / EPCS paging. All are
  **preserved verbatim** by the round-trip today but not modeled or editable — a large, distinct
  feature from channel programming. Facts live in
  [`docs/radios/ft60.md`](docs/radios/ft60.md); codec in
  [`crates/platypus-core/src/device/ft60.rs`](crates/platypus-core/src/device/ft60.rs).

## Serial (SDS150 live control)

- **Live location push (`LCR`).** First serial feature: push the map's point+radius (or a
  typed ZIP) to the radio **live** via `LCR,<lat>,<lon>,<range>\r` — location-first without a
  card write. The protocol **codec is done** (`platypus-core::serial`: `Command`/`Response`
  encode+parse for MDL/VER/VOL/SQL/LCR/KEY/GCS/STS/KAL/POF, fully unit-tested, no I/O). What
  remains: the **serial transport** (reuse `platypus-serial`'s `SerialPort` to open the USB
  serial port, pump the codec, handle the `MDL`→`SDS150GBT` handshake + `KAL` keep-alive), a
  thin FFI, and the map "push to radio" button. Then extend the codec with `AVD` (live avoid)
  and `FQK`/`SQK`/`DQK` (live quick-key enable), and parse `GSI`/`PSI` XML for a live "what's it
  hearing" view. See the serial-protocol section in
  [`docs/radios/sds150.md`](docs/radios/sds150.md) and the device-class split in
  [`docs/architecture.md`](docs/architecture.md).
