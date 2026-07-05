# Data Sources

Every way radio data gets *into* Platypus. The architecture is a **hub**: each
source is a [`Provider`](../crates/platypus-core/src/provider.rs) that loads its data
into one **canonical model** (`Dataset` of `SystemRecord`s), after which filters,
the writer, and the UI are 100% source-agnostic. Adding a source = implementing the
provider + its mapping; nothing downstream changes.

```
Sentinel/HPDB files ─┐
RadioReference SOAP ─┤
FCC ULS            ─┼─►  Provider.load()  ─►  Dataset (canonical)  ─►  filter ─► writer ─► card
OpenMHz / CSV      ─┘                          systems · sites ·
                                               channels · geo · service types
```

**Universal rule:** the core is **I/O-light**: a provider's *mapping* (source
shape → canonical) lives in dependency-free `platypus-core` and is unit-tested with
fixtures; the *acquisition* (file read, HTTP/SOAP, archive download) is platform
**glue** that hands the core already-fetched data. This is why `platypus-core` has
zero dependencies and every mapping is offline-testable.

The canonical target every source maps onto:

| Field | Meaning |
|---|---|
| `SystemRecord` | name · kind (Conventional/Trunk) · `tech` · `county_ids` · `state_ids` · `locations[]` · `channels[]` · `service_types` |
| `Location` | name · `Geo { lat, lon, range_mi, shape }` — sites & coverage groups |
| `Channel` | name · kind (Frequency/Talkgroup) · `freq_hz` · `tgid` · `mode` · `tone` · `service_type` (RR code) |

---

## 1. Sentinel / HPDB card files — **implemented (provider #1)**

- **What:** the tab-delimited ASCII files Uniden **Sentinel** writes to the SD card
  (`s_<state>.hpd`, `hpdb.cfg`, favorites `f_*.hpd`, …). Sentinel itself pulls from
  RadioReference, so this is *RR data, already packed*. See `CLAUDE.md` for the byte
  format.
- **Acquire:** read files off the card / a backup folder (caller does the I/O).
- **Shape:** the relational HPDB model: `Conventional`/`Trunk` → `Site`/`C-Group`/
  `T-Group` → `C-Freq`/`TGID`, with `AreaState`/`AreaCounty` geo tags and per-record
  lat/lon/range. Service type is a **numeric RR code** (we map common codes to names
  in `model::service_type_name`).
- **Auth / cost:** none (local files). Implies the user once ran Sentinel (Windows) +
  has an RR sub.
- **Mapping:** `provider::HpdbProvider` (parse `Document` → segment systems →
  `SystemRecord`s).
- **Status:** ✅ done, byte-exact round-trip, hardware-validated read **and** write.
- **Gotchas:** misspelled `discvery.cfg`; must delete `app_data.cfg` on write; favorites
  is a distinct *dialect* (see `CLAUDE.md`). Strength: works fully **offline**, and it's
  the only source proven end-to-end on real hardware.

## 2. RadioReference Web Service — **schema + mapping sketched (provider #2)**

- **What:** the curated upstream database, queried **directly** over its SOAP API:
  the Mac-native unlock (no Windows/Sentinel).
- **Acquire:** SOAP/XML over HTTP. Location-first call chain: `getZipcodeInfo`
  (zip → county/lat/lon) → `getCountyInfo` / county freq search → `getTrsDetails` +
  `getSites` + `getTalkgroups` per system. **Per-query, not bulk**: fits "fetch my
  area"; a full-USA pull means iterating counties/states.
- **Shape:** typed RR structs (`Trs`, `TrsSite`, `Talkgroup`, `freq`, `subcat`, `Tag`,
  `Bandplan`, …), modeled field-for-field in `rr.rs` from the **public WSDL**
  (`api.radioreference.com/soap2/?wsdl`) + the `DSheirer/radio-reference-api` reference
  client. Near-identical to the HPDB (Sentinel proves the transform). **Two upgrades
  over the card:** service types arrive as **names** (`Tag.tagDescr`), and **band plans
  are explicit** (`Trs.bandplan`).
- **Auth / cost:** `authInfo` = our **appKey** (one per app, **free**, issued by RR
  support: email `support@radioreference.com`) + the **end user's** RR username/
  password, whose account must be **Premium** (paid, ~$30/yr). Free for us, costs the
  user their existing sub, gated on RR approving the app key. No published rate limits;
  don't redistribute (per-user auth only).
- **Mapping:** `rr::RadioReferenceProvider` + `From<&RrSystem> for SystemRecord` —
  done & tested offline with fixtures. **The network call (`rr::fetch_systems`) is a
  documented stub** returning `Error::NotInCore`; the real SOAP/HTTP/XML client is glue
  for a future `platypus-rr` crate (would add `reqwest` + an XML/SOAP dep) or the app
  layer, then feeds `RrSystem`s into `from_systems`.
- **Status:** 🟡 types + canonical mapping in code (`rr.rs`, 5 tests). **Blocked on a
  you-action:** request the app key. Then build the SOAP client behind the stub.
- **Gotchas:** confirm current ToS/limits before shipping; `sType`/`sFlavor`/`sVoice`
  are ids into `getTypes`/`getFlavors`/`getVoices` taxonomies (resolve to names to drive
  `tech_from_rr`); RR `out`/`in` are **MHz** (we store **Hz**).

## 3. FCC ULS — **future (provider #3)**

- **What:** the FCC Universal Licensing System: the authoritative raw license DB
  (frequencies, callsigns, locations, license holders). Public-domain government data.
- **Acquire:** bulk weekly DB dumps (pipe-delimited `.dat` archives) and/or a query API.
- **Shape:** license-centric, **not** scanner-system-centric: frequencies + geocoded
  license locations, no talkgroup labels or trunked-system structure. RR's own
  `FccGetCallsign*` calls overlay this onto systems.
- **Auth / cost:** free, no account.
- **Mapping:** TODO — `FccProvider`: licenses → conventional `SystemRecord`s (likely
  grouped by licensee/location); little to no trunked structure.
- **Status:** ⚪ not started. Value: fills gaps RR lacks (raw/obscure licenses) and is
  fully free/offline. Weak on the curation (names, talkgroups) that is RR's moat — best
  as a **supplement**, not a primary.

## 4. OpenMHz / Broadcastify / CSV / manual — **future (providers #4+)**

- **OpenMHz / Broadcastify Calls:** community trunk-recorder data, useful for *observed*
  talkgroup activity, not authoritative system structure. Has its own API (`rr.rs`
  neighborhood; see Broadcastify-Calls-API). ⚪ not started.
- **CSV / manual entry:** let users paste/import a spreadsheet or hand-add a system —
  the universal escape hatch. Maps a column layout → `SystemRecord`. ⚪ not started.
- **Auth / cost:** varies (OpenMHz free-ish; CSV none).

---

## Updates & freshness (Sentinel bulk vs RR delta, and the hybrid)

Radio data goes stale; how each source refreshes differs sharply:

- **Sentinel** = coarse **full-DB replace**. Re-downloads a fresh HPDB snapshot from RR
  and overwrites the `s_*.hpd` set, all-or-nothing per state, on Uniden's schedule.
- **RR Web Service** = fine-grained **delta sync**. Every record carries a
  `lastUpdated` timestamp (`Trs.lastUpdated`, `freq.lastUpdated`, …). Store it at import;
  on refresh, re-query the scope and replace only records whose `lastUpdated` advanced.
  Location-first keeps that scope tiny (your counties / saved areas, not all ~15k
  systems), so a refresh is "freshen my area," on your schedule and granularity.

**Hybrid model (recommended): Sentinel base + RR overlay.** Bulk-import the Sentinel DB
once (free, offline, full-USA breadth), then use RR as the incremental freshener for
monitored areas — best of both. Requirements:

- **Identity/key join.** The source HPDB carries **RR's own ids**: e.g. real data shows
  `CountyId`/`StateId` (= RR `ctid`/`stid`) and `CFreqId`/`CGroupId`/`Tid`/`SiteId`
  (RR record ids). So a card record matches its RR record by id, with geo+name+freq/TGID
  as a fallback. **Caveat:** the *favorites* dialect blanks `MyId`/`ParentId`, so updates
  must run off the imported **source HPDB**, not a favorites file. **Verify on first RR
  access** that HPDB ids are identical to RR ids (near-certain; confirm on one system).
- **Precedence:** RR is the curated authority → RR wins on conflict (the card is a stale
  RR snapshot); user edits are a third layer on top.
- **Provenance:** tag every record with `(source, lastUpdated)` so the UI can show
  "from card · 3 mo old" vs "RR · today" and offer a per-area refresh.

**This is the real mandate for the SQLite store** — not filtering speed (microseconds in
memory, benchmarked). Delta-merging two sources with identity, precedence, and
provenance, persisted across launches, is relational-store work; design its schema around
`(source, rr_id, lastUpdated)` keys when it's built.

## Cross-source concerns (when >1 source is live)

- **Merge / dedupe:** the same system can arrive from multiple sources (RR + a Sentinel
  card). Needs identity keys (RR system id, geo+name) and a precedence rule (curated RR
  > FCC raw). This is the **SQLite store** track in the roadmap — the persistence layer
  where multi-source data is reconciled, edited, and saved-filtered.
- **Provenance & freshness:** track which source each record came from and "last updated";
  surface staleness. Don't silently mix.
- **Don't redistribute RR data:** per-user auth; the app is a client, never a shared store.
- **Bulk vs on-demand:** full-USA offline browse wants a bulk import (Sentinel files now,
  or an iterated RR sync); the common case is on-demand "fetch my area." UI should make
  the cheap path effortless and the heavy path possible-but-clearly-heavier.

## Status at a glance

| # | Source | Acquire | Auth | Mapping | Status |
|---|---|---|---|---|---|
| 1 | Sentinel/HPDB files | local files | none | `HpdbProvider` | ✅ hardware-validated |
| 2 | RadioReference SOAP | per-query HTTP | appKey + user Premium | `rr::RadioReferenceProvider` | 🟡 mapping done; need app key + SOAP client |
| 3 | FCC ULS | bulk dumps / API | none | `FccProvider` (TODO) | ⚪ not started |
| 4 | OpenMHz / CSV / manual | API / file / UI | varies | TODO | ⚪ not started |
