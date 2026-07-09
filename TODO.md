# TODO

Tracked future work. Run `just check` before pushing (see `CLAUDE.md`).

## Release / distribution

- **Sign + notarize the macOS app.** `just app::bundle` has a `_sign` seam (runs `codesign`
  only when `CODESIGN_IDENTITY` is set; a no-op otherwise, so dev builds stay unsigned). For a
  distributable build, wire a Developer ID: `codesign --options runtime`, then
  `xcrun notarytool submit … --wait` + `xcrun stapler staple` (stub commands are already in
  `apps/PlatypusMac/justfile`). Needs an Apple Developer account + a notary credential profile.

## UI / data

- **Display customization — hardware verification.** Confirm the `DisplayOption` col-11 label and
  the `Backlight` fields against a real card, and hardware-confirm the per-element area↔`DispColors`
  group pairing (spec-derived + example-confirmed, not yet verified on-device). Record in
  [`docs/radios/sds150-display.md`](docs/radios/sds150-display.md).

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

- **Location-first map: widen a networked source beyond one county.** The map is coordinate-scoped
  for local sources but county-scoped for the SOAP web service (its API resolves a place to one county
  and has no radius query), so panning re-anchors county-by-county. Widening a networked source to a
  true radius needs per-county centroids the API doesn't provide; a bundled county-centroid table would
  enable it but requires reliable name matching. Revisit if county-granular coverage isn't enough.

- **Portable geocoding for non-macOS front-ends.** Place/coordinate/ZIP geocoding — and the "locate
  me" device fix — is Apple `CLGeocoder`/`CLLocationManager` today, so it's macOS-only and lives
  entirely in the app layer; the core/FFI stay geocoder-agnostic (they take a coordinate + radius or a
  ZIP string). A Linux/Windows front-end needs its own: an OS location service, a networked geocoder
  (e.g. OSM Nominatim — honor its usage policy + a descriptive `User-Agent`), or a **bundled offline
  ZCTA/county-centroid table** for network-free forward (ZIP→coord) and reverse (coord→nearest
  ZIP/county). `platypus-rr`'s `getZipcodeInfo` already covers forward-by-ZIP portably; **reverse**
  (coord→ZIP) is the real gap — the map's pan-to-load depends on it. The same offline centroid table
  would also unlock the multi-county widening above (see *"Location-first map: widen a networked source
  beyond one county"*). See the geocoding principle in [`docs/architecture.md`](docs/architecture.md).

- **Extract a shared networked-source cache/transport layer.** The per-account, OS-appropriate on-disk
  cache + throttle + TTL / refresh / upstream-`lastUpdated` freshness (and atomic writes) currently live
  in `platypus-rr`. When a second networked provider lands, lift that into a reusable crate so each
  provider reuses one gentle, well-behaved cache instead of reimplementing it.

- **SQLite store (cross-source merge + refresh).** A persistent store keyed on
  `(source, id, lastUpdated)` to merge a Sentinel base with RR / RepeaterBook deltas and refresh
  them. Its mandate is **freshness**, not speed; browse is already fast in-memory.

## FT-60 (Yaesu clone-image HT)

Read / write / edit-and-write and the full standard-memory record are **done and byte-exact**
(name, freq, mode FM/NFM/AM, tone + sub-kind, duplex ±, offset, split tx-freq, power, step,
skip, banks). PMS band-edge memories decode, edit, and round-trip (the pair editor is in;
interleaved lower/upper pairing confirmed on hardware). Remaining FT-60 capabilities to map:

- **Split duplex + tx_freq** is implemented but **unvalidated** — the FT-60 front panel can't set
  an odd-split, so there's no way to program a hardware sample. Round-trips defensively; confirm if
  a split image ever turns up. (Power Mid/Low and the 70 cm 5 MHz offset are now hardware-validated
  via a write-back round-trip — see [`docs/radios/ft60.md`](docs/radios/ft60.md).)
- **Non-channel radio config (extend the radio settings editor).** The set-mode/menu settings
  (APO, squelch, lamp, beep, ARTS, busy-lockout…) are now modeled + editable via the gear dialog.
  Still preserved-verbatim-only, not yet modeled: home channels (5), NOAA weather (10), DTMF
  autodialer memories, and VFO / one-touch / EPCS paging — a distinct feature from channel
  programming. Facts live in
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
