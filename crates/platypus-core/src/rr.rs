// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! RadioReference (RR) Web Service — typed data model + mapping to the canonical
//! [`Dataset`](crate::provider::Dataset).
//!
//! RR is the curated upstream database that Sentinel reads to pack the HPDB card
//! we already parse. This module is the **schema sketch + mapping** for talking to
//! RR *directly* (provider #2), so a Mac client needs no Windows/Sentinel.
//!
//! ## What lives here vs. the glue layer
//!
//! Following the same split as [`HpdbProvider`](crate::provider::HpdbProvider) —
//! *the core is I/O-light; the caller does the I/O* — this module owns only the
//! **pure, dependency-free, fully-tested mapping** from RR's structures into the
//! canonical model. The **network** (SOAP envelope, auth, HTTP via `reqwest`, XML
//! deserialization) is platform glue and lives in a future `platypus-rr` crate or
//! the FFI/app layer; here, [`fetch_systems`] is an explicit stub that returns
//! [`Error::NotInCore`]. That keeps `platypus-core` zero-dependency and lets the
//! mapping be validated offline against fixtures.
//!
//! ## RR's shape (from the public WSDL — no account needed to know the schema)
//!
//! The field names below mirror the WSDL members verbatim (in doc comments) so the
//! mapping is auditable. Source: `api.radioreference.com/soap2/?wsdl` and the
//! reference client `DSheirer/radio-reference-api`. The data is a near-identical
//! relational model to the HPDB — because Sentinel performs exactly this RR→HPDB
//! transform — so mapping is largely a field rename, with two upgrades over the
//! card data:
//!
//! - **Service types arrive as names** (`Tag.tagDescr`), not just the numeric
//!   `tagId` code — so RR grounds the icon taxonomy the HPDB only hints at.
//! - **Band plans are explicit** (`Trs.bandplan`) — the thing we had to guess at
//!   when writing P25 favorites.

use std::collections::BTreeSet;

use crate::model::{Geo, Shape};
use crate::provider::{
    Channel, ChannelKind, Dataset, Location, Provider, SystemKind, SystemRecord,
};
use crate::{Error, Result};

// ---------------------------------------------------------------------------
// RR data structures (WSDL complexTypes). snake_case Rust names; the RR member
// each maps from is noted in the doc comment.
// ---------------------------------------------------------------------------

/// RR `tag` — one entry of the service-type taxonomy. `tag_id` is the **same**
/// numeric service-type code carried in the HPDB (see
/// [`service_type_name`](crate::model::service_type_name)); `tag_descr` is its
/// human label, which the card data omits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// RR `tagId`
    pub tag_id: u16,
    /// RR `tagDescr`
    pub tag_descr: String,
}

/// RR `Talkgroup` (from `getTalkgroups`). Maps to an HPDB `TGID` channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Talkgroup {
    /// RR `tgDec` — the decimal talkgroup id the radio displays.
    pub tg_dec: String,
    /// RR `tgAlpha` — short alpha tag (the channel name).
    pub tg_alpha: String,
    /// RR `tgDescr` — long description.
    pub tg_descr: String,
    /// RR `tgMode` — `A` analog, `D` digital (P25), `M` mixed, `T` TDMA, `E` encrypted.
    pub tg_mode: String,
    /// RR `tags` — service types on this talkgroup.
    pub tags: Vec<Tag>,
}

/// RR conventional `freq` (from `getSubcatFreqs`/search). Maps to an HPDB `C-Freq`.
#[derive(Debug, Clone, PartialEq)]
pub struct Freq {
    /// RR `out` — output (receive) frequency in **MHz**.
    pub out_mhz: f64,
    /// RR `in` — input frequency in MHz (`in` is a Rust keyword).
    pub in_mhz: f64,
    /// RR `alpha` — short name.
    pub alpha: String,
    /// RR `descr` — description.
    pub descr: String,
    /// RR `tone` — CTCSS/DCS/NAC/color-code tone string.
    pub tone: String,
    /// RR `mode` — `FM`/`FMN`/`AM`/`P25`/`DMR`/…
    pub mode: String,
    /// RR `tags` — service types on this frequency.
    pub tags: Vec<Tag>,
}

/// RR `siteFreq` (within `TrsSite.siteFreqs`). The control/voice frequencies a
/// trunked site transmits — maps to the HPDB `T-Freq`.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteFrequency {
    /// Output frequency in MHz.
    pub freq_mhz: f64,
    /// RR `use` — `d` primary control, `a` alternate control, empty = voice.
    pub use_: String,
    /// Logical Channel Number (LCN), where the system defines one.
    pub lcn: Option<u32>,
}

/// RR `TrsSite` (from `getSites`). Maps to an HPDB `Site`. Geo-located.
#[derive(Debug, Clone, PartialEq)]
pub struct TrsSite {
    /// RR `siteId`
    pub site_id: u32,
    /// RR `siteNumber`
    pub site_number: u32,
    /// RR `siteDescr`
    pub site_descr: String,
    /// RR `lat`/`lon`/`range` (range in miles).
    pub lat: f64,
    pub lon: f64,
    pub range: f64,
    /// RR `nac` (P25) — present on P25 sites.
    pub nac: Option<String>,
    /// RR `siteFreqs`
    pub frequencies: Vec<SiteFrequency>,
}

/// RR `subcat` — a talkgroup/conventional category (maps to an HPDB `T-Group`/
/// `C-Group`). Geo-located.
#[derive(Debug, Clone, PartialEq)]
pub struct Subcat {
    /// RR `scName`
    pub sc_name: String,
    /// RR `lat`/`lon`/`range`.
    pub lat: f64,
    pub lon: f64,
    pub range: f64,
}

/// RR `Trs` header (from `getTrsDetails`). The trunked-system record itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Trs {
    /// RR `sName`
    pub s_name: String,
    /// RR `sType`/`sFlavor`/`sVoice` — numeric ids into the `getTypes`/
    /// `getFlavors`/`getVoices` taxonomies. Kept raw for fidelity; the human
    /// names (resolved by the caller via those calls) drive [`tech_from_rr`].
    pub s_type: i32,
    pub s_flavor: i32,
    pub s_voice: i32,
    /// Resolved type/flavor names from `getTypes`/`getFlavors` (e.g.
    /// `"Project 25"`, `"Phase II"`), if the caller looked them up.
    pub type_name: Option<String>,
    pub flavor_name: Option<String>,
    /// RR `sCounty` (ctids) / `sState` (stids).
    pub county_ids: Vec<u64>,
    pub state_ids: Vec<u64>,
    /// RR `lat`/`lon`/`range`.
    pub lat: f64,
    pub lon: f64,
    pub range: f64,
}

/// An assembled trunked system: the `Trs` header plus the `getSites` and
/// `getTalkgroups` results a caller stitches together for it.
#[derive(Debug, Clone, PartialEq)]
pub struct RrTrunkedSystem {
    pub trs: Trs,
    pub sites: Vec<TrsSite>,
    pub talkgroups: Vec<Talkgroup>,
}

/// An assembled conventional system: a county/agency grouping of subcategories,
/// each with its frequencies.
#[derive(Debug, Clone, PartialEq)]
pub struct RrConventionalSystem {
    pub name: String,
    pub county_ids: Vec<u64>,
    pub state_ids: Vec<u64>,
    pub subcats: Vec<(Subcat, Vec<Freq>)>,
}

/// One RR system, trunked or conventional — the unit the provider maps.
#[derive(Debug, Clone, PartialEq)]
pub enum RrSystem {
    Trunked(RrTrunkedSystem),
    Conventional(RrConventionalSystem),
}

// ---------------------------------------------------------------------------
// Mapping: RR types -> canonical model. Pure, dependency-free, tested offline.
// ---------------------------------------------------------------------------

/// MHz (RR's unit) -> Hz (the canonical unit). Rounds to the nearest Hz.
fn mhz_to_hz(mhz: f64) -> Option<u64> {
    if mhz > 0.0 {
        Some((mhz * 1_000_000.0).round() as u64)
    } else {
        None
    }
}

/// Map RR's `getTypes`/`getFlavors` names onto our raw tech vocabulary (the same
/// strings the HPDB header uses, e.g. `P25Standard`, `MotoTrbo`). Best-effort over
/// the names we know; unknown combinations fall back to the type name as-is so no
/// information is lost. Refine as the live taxonomy is confirmed.
pub fn tech_from_rr(type_name: Option<&str>, flavor_name: Option<&str>) -> Option<String> {
    let t = type_name?;
    let mapped = match t {
        "Project 25" => match flavor_name {
            Some("Phase II") => "P25Phase2",
            _ => "P25Standard",
        },
        "Motorola" => "Motorola",
        "DMR" | "Mototrbo" => "MotoTrbo",
        "NXDN" => "Nxdn",
        "LTR" | "LTR Standard" | "LTR Passport" => "Ltr",
        "EDACS" => "Edacs",
        other => other,
    };
    Some(mapped.to_string())
}

/// First service-type code on a list of tags (the channel's primary tag).
fn primary_service_type(tags: &[Tag]) -> Option<u16> {
    tags.first().map(|t| t.tag_id)
}

impl From<&Talkgroup> for Channel {
    fn from(tg: &Talkgroup) -> Self {
        let name = if !tg.tg_descr.is_empty() {
            tg.tg_descr.clone()
        } else {
            tg.tg_alpha.clone()
        };
        Channel {
            name,
            kind: ChannelKind::Talkgroup,
            freq_hz: None,
            tgid: Some(tg.tg_dec.clone()),
            mode: (!tg.tg_mode.is_empty()).then(|| tg.tg_mode.clone()),
            tone: None,
            service_type: primary_service_type(&tg.tags),
        }
    }
}

impl From<&Freq> for Channel {
    fn from(f: &Freq) -> Self {
        let name = if !f.descr.is_empty() {
            f.descr.clone()
        } else {
            f.alpha.clone()
        };
        Channel {
            name,
            kind: ChannelKind::Frequency,
            freq_hz: mhz_to_hz(f.out_mhz),
            tgid: None,
            mode: (!f.mode.is_empty()).then(|| f.mode.clone()),
            tone: (!f.tone.is_empty()).then(|| f.tone.clone()),
            service_type: primary_service_type(&f.tags),
        }
    }
}

fn location(name: &str, lat: f64, lon: f64, range: f64) -> Location {
    Location {
        name: name.to_string(),
        geo: Geo {
            lat,
            lon,
            range_mi: range,
            shape: Shape::Circle,
        },
    }
}

impl From<&RrTrunkedSystem> for SystemRecord {
    fn from(sys: &RrTrunkedSystem) -> Self {
        let channels: Vec<Channel> = sys.talkgroups.iter().map(Channel::from).collect();
        let service_types: BTreeSet<u16> = channels.iter().filter_map(|c| c.service_type).collect();
        SystemRecord {
            name: sys.trs.s_name.clone(),
            kind: SystemKind::Trunk,
            tech: tech_from_rr(sys.trs.type_name.as_deref(), sys.trs.flavor_name.as_deref()),
            county_ids: sys.trs.county_ids.clone(),
            state_ids: sys.trs.state_ids.clone(),
            locations: sys
                .sites
                .iter()
                .map(|s| location(&s.site_descr, s.lat, s.lon, s.range))
                .collect(),
            channels,
            service_types,
        }
    }
}

impl From<&RrConventionalSystem> for SystemRecord {
    fn from(sys: &RrConventionalSystem) -> Self {
        let mut locations = Vec::new();
        let mut channels = Vec::new();
        for (subcat, freqs) in &sys.subcats {
            locations.push(location(
                &subcat.sc_name,
                subcat.lat,
                subcat.lon,
                subcat.range,
            ));
            channels.extend(freqs.iter().map(Channel::from));
        }
        let service_types: BTreeSet<u16> = channels.iter().filter_map(|c| c.service_type).collect();
        SystemRecord {
            name: sys.name.clone(),
            kind: SystemKind::Conventional,
            tech: Some("Conventional".to_string()),
            county_ids: sys.county_ids.clone(),
            state_ids: sys.state_ids.clone(),
            locations,
            channels,
            service_types,
        }
    }
}

impl From<&RrSystem> for SystemRecord {
    fn from(sys: &RrSystem) -> Self {
        match sys {
            RrSystem::Trunked(t) => t.into(),
            RrSystem::Conventional(c) => c.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Provider + the network seam (stubbed).
// ---------------------------------------------------------------------------

/// Per-user RadioReference auth — the WSDL `authInfo`. The **app key** is ours
/// (one per app, free, issued by RR support); **username/password** are the end
/// user's, and their account must be a paid **Premium** subscriber.
#[derive(Debug, Clone)]
pub struct Credentials {
    /// `appKey`
    pub app_key: String,
    /// `username`
    pub username: String,
    /// `password`
    pub password: String,
}

/// What to fetch from RR. The location-first entry points (`getZipcodeInfo` →
/// `getCountyInfo`/`getCountyFreqs` → `getTrsDetails`) map onto these.
#[derive(Debug, Clone)]
pub enum Query {
    /// Everything in a county (the common location-first request).
    County(u64),
    /// A specific trunked system by RR system id.
    TrunkedSystem(u64),
    /// A zip code — resolves to a county first (`getZipcodeInfo`).
    Zipcode(String),
}

/// **Network seam — not implemented in the core.** The SOAP call (auth envelope,
/// HTTP, XML→[`RrSystem`] deserialization) is platform glue (`reqwest` etc.) and
/// belongs in a `platypus-rr` crate or the app layer. The core deliberately stays
/// dependency-free; it only *maps* the [`RrSystem`]s a caller hands back. Wire the
/// real client to return `Vec<RrSystem>`, then feed them to
/// [`RadioReferenceProvider::from_systems`].
pub fn fetch_systems(_creds: &Credentials, _query: &Query) -> Result<Vec<RrSystem>> {
    Err(Error::NotInCore(
        "RadioReference SOAP fetch needs an HTTP/XML client; implement it in the \
         glue layer and pass the RrSystems to RadioReferenceProvider::from_systems",
    ))
}

/// Provider #2: RadioReference Web Service. Holds RR systems a caller already
/// fetched (mirroring [`HpdbProvider`], which holds an already-read `Document`)
/// and maps them into the canonical model.
pub struct RadioReferenceProvider {
    systems: Vec<RrSystem>,
    source: String,
}

impl RadioReferenceProvider {
    /// Build from systems obtained via the glue-layer client (or fixtures).
    pub fn from_systems(systems: Vec<RrSystem>, source: impl Into<String>) -> Self {
        RadioReferenceProvider {
            systems,
            source: source.into(),
        }
    }
}

impl Provider for RadioReferenceProvider {
    fn name(&self) -> &str {
        &self.source
    }

    fn load(&self) -> Result<Dataset> {
        Ok(Dataset {
            systems: self.systems.iter().map(SystemRecord::from).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p25_system() -> RrSystem {
        RrSystem::Trunked(RrTrunkedSystem {
            trs: Trs {
                s_name: "Statewide P25".into(),
                s_type: 5,
                s_flavor: 2,
                s_voice: 1,
                type_name: Some("Project 25".into()),
                flavor_name: Some("Phase II".into()),
                county_ids: vec![9001, 9002],
                state_ids: vec![90],
                lat: 45.0,
                lon: -122.0,
                range: 30.0,
            },
            sites: vec![TrsSite {
                site_id: 1,
                site_number: 1,
                site_descr: "Hilltop".into(),
                lat: 45.1,
                lon: -122.1,
                range: 25.0,
                nac: Some("293".into()),
                frequencies: vec![SiteFrequency {
                    freq_mhz: 851.0125,
                    use_: "d".into(),
                    lcn: Some(1),
                }],
            }],
            talkgroups: vec![Talkgroup {
                tg_dec: "12345".into(),
                tg_alpha: "PD Disp".into(),
                tg_descr: "City Police Dispatch".into(),
                tg_mode: "D".into(),
                tags: vec![Tag {
                    tag_id: 2,
                    tag_descr: "Law Dispatch".into(),
                }],
            }],
        })
    }

    fn conventional_system() -> RrSystem {
        RrSystem::Conventional(RrConventionalSystem {
            name: "County Fire".into(),
            county_ids: vec![9003],
            state_ids: vec![90],
            subcats: vec![(
                Subcat {
                    sc_name: "Fire/EMS".into(),
                    lat: 44.0,
                    lon: -121.0,
                    range: 10.0,
                },
                vec![Freq {
                    out_mhz: 154.265,
                    in_mhz: 0.0,
                    alpha: "FG1".into(),
                    descr: "Fireground 1".into(),
                    tone: "131.8 PL".into(),
                    mode: "FM".into(),
                    tags: vec![Tag {
                        tag_id: 8,
                        tag_descr: "Fire-Tac".into(),
                    }],
                }],
            )],
        })
    }

    #[test]
    fn maps_trunked_system_to_canonical() {
        let provider = RadioReferenceProvider::from_systems(vec![p25_system()], "RR test");
        let ds = provider.load().unwrap();
        assert_eq!(ds.len(), 1);
        let sys = &ds.systems[0];
        assert_eq!(sys.name, "Statewide P25");
        assert_eq!(sys.kind, SystemKind::Trunk);
        assert_eq!(sys.tech.as_deref(), Some("P25Phase2"));
        assert_eq!(sys.county_ids, vec![9001, 9002]);
        // site -> location
        assert_eq!(sys.locations.len(), 1);
        assert_eq!(sys.locations[0].name, "Hilltop");
        assert_eq!(sys.locations[0].geo.lat, 45.1);
        // talkgroup -> channel, with service type aggregated
        assert_eq!(sys.channels.len(), 1);
        let ch = &sys.channels[0];
        assert_eq!(ch.kind, ChannelKind::Talkgroup);
        assert_eq!(ch.tgid.as_deref(), Some("12345"));
        assert_eq!(ch.name, "City Police Dispatch");
        assert_eq!(ch.service_type, Some(2));
        assert!(sys.has_service_type(2));
    }

    #[test]
    fn maps_conventional_freqs_with_hz_conversion() {
        let provider = RadioReferenceProvider::from_systems(vec![conventional_system()], "RR test");
        let ds = provider.load().unwrap();
        let sys = &ds.systems[0];
        assert_eq!(sys.kind, SystemKind::Conventional);
        assert_eq!(sys.tech.as_deref(), Some("Conventional"));
        assert_eq!(sys.locations[0].name, "Fire/EMS");
        let ch = &sys.channels[0];
        assert_eq!(ch.kind, ChannelKind::Frequency);
        // 154.265 MHz -> 154_265_000 Hz
        assert_eq!(ch.freq_hz, Some(154_265_000));
        assert_eq!(ch.tone.as_deref(), Some("131.8 PL"));
        assert_eq!(ch.service_type, Some(8));
    }

    #[test]
    fn canonical_filters_work_on_rr_sourced_data() {
        // The whole point of the canonical seam: the same predicate engine filters
        // RR data exactly as it filters HPDB data.
        let provider = RadioReferenceProvider::from_systems(
            vec![p25_system(), conventional_system()],
            "RR test",
        );
        let ds = provider.load().unwrap();
        assert_eq!(ds.select(|s| s.is_in_county(9001)).len(), 1);
        assert_eq!(ds.select(|s| s.has_service_type(8)).len(), 1);
        assert_eq!(ds.select(|s| s.kind == SystemKind::Trunk).len(), 1);
    }

    #[test]
    fn tech_mapping_known_and_fallback() {
        assert_eq!(
            tech_from_rr(Some("Project 25"), Some("Phase I")).as_deref(),
            Some("P25Standard")
        );
        assert_eq!(tech_from_rr(Some("DMR"), None).as_deref(), Some("MotoTrbo"));
        // unknown type name is preserved verbatim, not dropped
        assert_eq!(
            tech_from_rr(Some("OpenSky"), None).as_deref(),
            Some("OpenSky")
        );
        assert_eq!(tech_from_rr(None, None), None);
    }

    #[test]
    fn fetch_is_a_documented_stub() {
        let creds = Credentials {
            app_key: "x".into(),
            username: "u".into(),
            password: "p".into(),
        };
        let err = fetch_systems(&creds, &Query::County(9001)).unwrap_err();
        assert!(matches!(err, Error::NotInCore(_)));
    }
}
