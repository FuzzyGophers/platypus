# Credits & third-party references

Platypus is **GPL-2.0-only** (see [`LICENSE`](LICENSE)). This file credits the external
documentation and references we consulted.

## Sourcing policy: facts only

**From every external reference, Platypus takes only *facts***: hardware specifications,
file and record layouts, memory-map offsets, and protocol constants (baud rates, block sizes,
ACK bytes, field meanings). Facts and the methods they describe are **not protected by
copyright** (only a particular creative *expression* is), so referencing them creates no
derivative work and imposes no license obligation; Platypus's own implementation is and
stays GPL-2.0-only.

The discipline that follows: **never copy a reference's expression** (its source code,
struct/enum definitions, string literals, table orderings, or algorithm structure), whatever
that reference's own license. Every codec is **derived from the facts, not any reference's code**,
cross-checked against the manufacturer spec and real hardware.

## Radio documentation & specifications

Manufacturer documentation, referenced for factual specifications and file/format details
(their prose and artwork remain the manufacturers' copyright; we quote only small factual
excerpts, with attribution):

- **Uniden** — SDS100/200 & `BCDx36HP` File Specifications (V1.03, V2.00), Remote Command
  Specification (V2.00), and the Operating & Service Manuals. Cited throughout
  [`docs/radios/sds150.md`](docs/radios/sds150.md).
- **Yaesu** — FT-60R/E Operating Manual (EH017M209) and Service Manual. Cited in
  [`docs/radios/ft60.md`](docs/radios/ft60.md).

## Reverse-engineering references

- **CHIRP** — © Dan Smith and the CHIRP contributors (<https://chirpmyradio.com>). Consulted
  only as a **factual cross-reference** for Yaesu FT-60 memory-map offsets and clone-protocol
  constants; no code is copied or ported.

## Data sources

How each data provider maps to the canonical model is documented in
[`docs/sources.md`](docs/sources.md). Use of any provider's data is subject to its terms and
the user's own account.

- **RadioReference** — the upstream curated database. Platypus is a *client*, not a rival
  dataset, and never redistributes RR data; each user authenticates with their own RR
  subscription.
- **RepeaterBook** — the community amateur-repeater directory
  (<https://www.repeaterbook.com>). Platypus accesses their free JSON API as a *client*; their
  terms require **attribution** ("Data courtesy of RepeaterBook.com") and prohibit bulk
  redistribution or re-serving. Each user authenticates with their own RepeaterBook app token.
- **Uniden Sentinel / SD-card database** — the scanner's on-card database is Uniden's data,
  obtained by the user through Uniden's Sentinel app (or already present on their card).
  Platypus reads only what is on the user's own card; it **does not bundle, download, or
  redistribute** this data.
- **FCC** public data: see [`docs/sources.md`](docs/sources.md).

## Dependency licenses

`cargo deny` / [`deny.toml`](deny.toml) gates the licenses of our actual *dependency* graph — a
separate concern from the documentation references above.
