<!--
SPDX-License-Identifier: GPL-2.0-only
SPDX-FileCopyrightText: The Platypus Authors
-->

# Capabilities

What Platypus can do today, per device. This is a living document — update it as
capabilities land. Planned work is tracked in [`../TODO.md`](../TODO.md); device
file-format and protocol facts live in the per-device docs under
[`radios/`](radios/).

## Across all devices

- **Location-first browse**: the whole USA + Canada held in memory (~15k systems /
  ~420k voice channels), navigated **Country → State → County → System → Channel**,
  filtered by service type / technology / mode / frequency / free-text.
- **Location-first placement**: statewide P25 systems land in the counties their
  coverage actually reaches, by geography.
- **Map lens**: a point-and-radius view of the systems near a location, with coverage
  circles, layered over the browse.
- **Manual radio selection** — the user's owned radios are chosen and remembered; the
  editor and add-targets adapt to the active radio's device *class*.

## Uniden SDS150 (SD-card scanner)

- **Read**: byte-exact parsing of the SD-card database format, validated against real
  card data.
- **Write + manage favorites** — build favorites from a location-first selection and
  commit them to the card (decode validated on a real SDS150, decoding live traffic),
  plus full **CRUD + alphabetize** on the card's existing lists (view / edit / rename /
  add / remove / delete / sort).
- **Card auto-detect** — a connected card (USB mass storage) is found by scanning
  `/Volumes`; **Open Card** opens it in one click, and it auto-opens on launch when
  exactly one card is present. A backup/library folder can be opened manually.
- **Backup / restore / eject**: full verified card backup, restore, and a guarded eject
  (writes are never reported successful until the card is safely flushed).
- **Scanner-aware limits** — model detected from each card's header; the per-list byte
  budget, list count, and quick keys come from the active profile and show in the UI.

Device reference: [`radios/sds150.md`](radios/sds150.md).

## Yaesu FT-60R (serial clone-image HT)

- **Read / write over the serial clone cable**: the full 28,617-byte memory image, with
  a byte-exact round-trip as the writer safety gate. **Every Read is backed up** — the raw
  image is saved to the managed backups folder (fsync'd, in the core FFI) as an automatic
  restore point before any editing.
- **Edit any channel** — name, RX frequency, mode (FM/NFM/AM), tone (CTCSS/DCS + the full
  tone-mode sub-kind, including the cross modes), duplex (±/split), offset, TX frequency,
  power, tuning step, skip, and multi-bank membership. Surgical, change-gated writes preserve
  everything not edited.
- **Add / delete channels and organize banks** (A–J), by hand or from the location-first
  catalog (conventional channels only — the HT can't use trunked systems).
- **Edit PMS band-edge memories** — the 50 scan-limit pairs (lower/upper + step), written
  back through the same clone-out path.
- **Edit set-mode settings** — the menu block (auto power-off, time-out timer, RF squelch,
  control lock, DTMF delay/speed, ARTS beep, lamp, bell, battery saver), change-gated so the
  reserved bits and unmodeled settings are preserved. Every Read is auto-backed-up first.

Device reference: [`radios/ft60.md`](radios/ft60.md).
