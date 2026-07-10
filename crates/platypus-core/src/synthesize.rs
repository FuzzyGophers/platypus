// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Synthesize a scanner favorites file from a **source-agnostic programmable model**.
//!
//! The location-first flow lets a user browse many sources (RadioReference today,
//! RepeaterBook/HPDB later) and program them onto many radios. The reusable seam:
//!
//! ```text
//! source (RR, RepeaterBook, …) → ProgramSystem (this module, lossless + hierarchical)
//!     → per-radio-class emitter → device format
//! ```
//!
//! [`ProgramSystem`] is the neutral, **lossless** hierarchy (system → sites with their
//! control/voice frequencies → groups with their channels), unlike the flat
//! [`provider::Dataset`](crate::provider) view used for browse/filter (which drops the
//! site frequencies a trunk needs to tune). Each import source maps *into* it; each radio
//! class emits *from* it. [`synthesize_favorites`] is the **SD-card scanner** emitter — it
//! builds a full-dialect [`Document`] per the model's [`RecordSchema`] and runs it through
//! the existing, device-validated [`favorites::build_favorites`]. A clone-image (FT-60)
//! emitter consuming the same [`ProgramSystem`] is the natural next radio.

use crate::device::{Modulation, ProgramSupport, SdCardProfile};
use crate::favorites::build_favorites;
use crate::format::{Document, Line, LineEnding};
use crate::provider::SystemKind;
use crate::rr::{mhz_to_hz, RrConventionalSystem, RrSystem, RrTrunkedSystem};
use std::collections::BTreeSet;

/// A geographic coverage point (a site or a group), source-agnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramGeo {
    pub lat: f64,
    pub lon: f64,
    pub range_mi: f64,
}

/// One control/voice frequency a trunked site transmits (→ an SD-card `T-Freq`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramSiteFreq {
    pub freq_hz: u64,
    /// Control channel (RR `use` = "d"/"a"); voice otherwise.
    pub control: bool,
}

/// A trunked site: where it is + the frequencies it uses (→ an SD-card `Site` + `T-Freq`s).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramSite {
    pub name: String,
    pub geo: Option<ProgramGeo>,
    pub frequencies: Vec<ProgramSiteFreq>,
}

/// One programmable channel — a conventional frequency or a trunked talkgroup.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramChannel {
    pub name: String,
    /// Conventional frequency (Hz); `None` for a talkgroup.
    pub freq_hz: Option<u64>,
    /// Repeater **input** (TX) frequency (Hz), when the source carries it — lets a clone-image
    /// emitter recover duplex/offset. `None` = simplex or a talkgroup.
    pub input_hz: Option<u64>,
    /// Talkgroup decimal id; `None` for a conventional frequency.
    pub tgid: Option<String>,
    /// Modulation/audio type as the device stores it (`NFM`/`AM`, or `DIGITAL`/`ANALOG`).
    pub mode: Option<String>,
    /// Squelch tone as the device stores it (`TONE=C156.7`); conventional only.
    pub tone: Option<String>,
    /// RadioReference service-type code.
    pub service_type: Option<u16>,
}

/// A talkgroup category / conventional subcategory (→ an SD-card `T-Group` / `C-Group`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramGroup {
    pub name: String,
    pub geo: Option<ProgramGeo>,
    pub channels: Vec<ProgramChannel>,
}

/// One programmable system, source-agnostic + lossless (unlike the flat browse `Dataset`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramSystem {
    pub name: String,
    pub kind: SystemKind,
    /// Raw technology string (`P25Standard`, `MotoTrbo`, `Conventional`, …).
    pub tech: Option<String>,
    pub county_ids: Vec<u64>,
    pub state_ids: Vec<u64>,
    /// Trunked sites (empty for a conventional system).
    pub sites: Vec<ProgramSite>,
    /// Talkgroup categories (trunked) or subcategories (conventional).
    pub groups: Vec<ProgramGroup>,
}

// ---------------------------------------------------------------------------
// Source mappings (RadioReference now; other sources map into the same model).
// ---------------------------------------------------------------------------

/// Normalize an RR conventional `mode` (`FM`/`FMN`/`AM`/…) to the SD-card vocabulary.
fn conventional_mode(rr_mode: &str) -> Option<String> {
    match rr_mode.to_ascii_uppercase().as_str() {
        "" => None,
        "FMN" | "NFM" => Some("NFM".into()),
        "FM" => Some("FM".into()),
        "AM" => Some("AM".into()),
        other => Some(other.to_string()),
    }
}

/// Normalize an RR talkgroup `tgMode` (`A`/`D`/`M`/`T`/`E`) to the SD-card `TGID` audio type.
fn talkgroup_mode(tg_mode: &str) -> Option<String> {
    match tg_mode.to_ascii_uppercase().as_str() {
        "A" => Some("ANALOG".into()),
        "D" | "T" | "M" | "E" => Some("DIGITAL".into()),
        _ => None,
    }
}

fn first_service_type(tags: &[crate::rr::Tag]) -> Option<u16> {
    tags.first().map(|t| t.tag_id)
}

impl From<&RrTrunkedSystem> for ProgramSystem {
    fn from(sys: &RrTrunkedSystem) -> Self {
        let sites = sys
            .sites
            .iter()
            .map(|s| ProgramSite {
                name: s.site_descr.clone(),
                geo: Some(ProgramGeo {
                    lat: s.lat,
                    lon: s.lon,
                    range_mi: s.range,
                }),
                frequencies: s
                    .frequencies
                    .iter()
                    .filter_map(|f| {
                        mhz_to_hz(f.freq_mhz).map(|freq_hz| ProgramSiteFreq {
                            freq_hz,
                            // RR `use`: "d" primary / "a" alternate control; empty = voice.
                            control: matches!(f.use_.as_str(), "d" | "a"),
                        })
                    })
                    .collect(),
            })
            .collect();
        // RR trunked talkgroups arrive as one flat list; put them under a single group for now
        // (category grouping is a later refinement).
        let channels = sys
            .talkgroups
            .iter()
            .map(|tg| ProgramChannel {
                name: if !tg.tg_descr.is_empty() {
                    tg.tg_descr.clone()
                } else {
                    tg.tg_alpha.clone()
                },
                freq_hz: None,
                input_hz: None,
                tgid: Some(tg.tg_dec.clone()),
                mode: talkgroup_mode(&tg.tg_mode),
                tone: None,
                service_type: first_service_type(&tg.tags),
            })
            .collect();
        ProgramSystem {
            name: sys.trs.s_name.clone(),
            kind: SystemKind::Trunk,
            tech: crate::rr::tech_from_rr(
                sys.trs.type_name.as_deref(),
                sys.trs.flavor_name.as_deref(),
            ),
            county_ids: sys.trs.county_ids.clone(),
            state_ids: sys.trs.state_ids.clone(),
            sites,
            groups: vec![ProgramGroup {
                name: sys.trs.s_name.clone(),
                geo: Some(ProgramGeo {
                    lat: sys.trs.lat,
                    lon: sys.trs.lon,
                    range_mi: sys.trs.range,
                }),
                channels,
            }],
        }
    }
}

impl From<&RrConventionalSystem> for ProgramSystem {
    fn from(sys: &RrConventionalSystem) -> Self {
        let groups = sys
            .subcats
            .iter()
            .map(|(subcat, freqs)| ProgramGroup {
                name: subcat.sc_name.clone(),
                geo: Some(ProgramGeo {
                    lat: subcat.lat,
                    lon: subcat.lon,
                    range_mi: subcat.range,
                }),
                channels: freqs
                    .iter()
                    .map(|f| ProgramChannel {
                        name: if !f.descr.is_empty() {
                            f.descr.clone()
                        } else {
                            f.alpha.clone()
                        },
                        freq_hz: mhz_to_hz(f.out_mhz),
                        // RR carries the repeater input as `in` — keep it when it's a real,
                        // distinct TX freq so a clone-image radio can recover the duplex/offset.
                        input_hz: (f.in_mhz > 0.0 && f.in_mhz != f.out_mhz)
                            .then(|| mhz_to_hz(f.in_mhz))
                            .flatten(),
                        tgid: None,
                        mode: conventional_mode(&f.mode),
                        tone: (!f.tone.is_empty()).then(|| f.tone.clone()),
                        service_type: first_service_type(&f.tags),
                    })
                    .collect(),
            })
            .collect();
        ProgramSystem {
            name: sys.name.clone(),
            kind: SystemKind::Conventional,
            tech: Some("Conventional".into()),
            county_ids: sys.county_ids.clone(),
            state_ids: sys.state_ids.clone(),
            sites: Vec::new(),
            groups,
        }
    }
}

impl From<&RrSystem> for ProgramSystem {
    fn from(sys: &RrSystem) -> Self {
        match sys {
            RrSystem::Trunked(t) => t.into(),
            RrSystem::Conventional(c) => c.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Capability filter: the one thing that differs between radios.
// ---------------------------------------------------------------------------

/// Flatten the programmable **channels** from `systems`, keeping only what a radio with this
/// [`ProgramSupport`] can store, de-duplicated. This capability gate is the *only* per-radio
/// difference in synthesis: a trunk-less radio drops trunked systems (and any talkgroup); a channel
/// survives only if its modulation — [`Modulation::classify`] of the system tech + channel mode — is
/// one the radio supports (e.g. an analog-only handheld keeps FM conventional, drops P25/DMR). The
/// dedupe key `(freq_hz, tgid, tone)` collapses the same repeater listed in two counties into one
/// memory; first occurrence keeps its name. A clone-image emitter builds device channels from this.
pub fn programmable_channels(
    systems: &[ProgramSystem],
    support: &ProgramSupport,
) -> Vec<ProgramChannel> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for sys in systems {
        // A trunked system needs a trunk-tracking radio.
        if matches!(sys.kind, SystemKind::Trunk) && !support.trunking {
            continue;
        }
        for group in &sys.groups {
            for ch in &group.channels {
                // A talkgroup needs trunking; a conventional memory needs a frequency.
                if ch.tgid.is_some() && !support.trunking {
                    continue;
                }
                let modulation = Modulation::classify(sys.tech.as_deref(), ch.mode.as_deref());
                if !support.supports(modulation) {
                    continue;
                }
                if seen.insert((ch.freq_hz, ch.tgid.clone(), ch.tone.clone())) {
                    out.push(ch.clone());
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SD-card emitter: ProgramSystem -> full-dialect Document -> build_favorites.
// ---------------------------------------------------------------------------

/// Sequential id generator (`SiteId=8201` etc.). Values only need internal referential
/// integrity: the favorites dialect blanks the id/parent columns, so the scanner reads the
/// hierarchy positionally — the numbers just keep the full-dialect `Document` well-formed.
struct Ids {
    next: u64,
}

impl Ids {
    fn new() -> Self {
        Ids { next: 1 }
    }
    fn next(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}={}", self.next);
        self.next += 1;
        id
    }
}

/// Format a coverage geo the way the card stores it (lat/lon to 6 dp, range to 1 dp).
fn fmt_geo(g: &ProgramGeo) -> (String, String, String) {
    (
        format!("{:.6}", g.lat),
        format!("{:.6}", g.lon),
        format!("{:.1}", g.range_mi),
    )
}

/// Build a line for `command` with `field_count` fields (from the profile schema), then set
/// specific columns. Extra fields are empty — matching the full-dialect records on a real card.
fn record(command: &str, field_count: usize, set: &[(usize, String)]) -> Line {
    let mut fields = vec![String::new(); field_count];
    fields[0] = command.to_string();
    for (col, val) in set {
        if *col < fields.len() {
            fields[*col] = val.clone();
        }
    }
    Line {
        fields,
        ending: LineEnding::Crlf,
    }
}

/// Emit one system's full-dialect records (header + sites/groups/channels) into `out`, keyed
/// off the model's `RecordSchema` field counts + column maps. Area tagging is omitted — the
/// favorites dialect strips it anyway.
fn emit_system(
    out: &mut Document,
    sys: &ProgramSystem,
    profile: &dyn SdCardProfile,
    ids: &mut Ids,
) {
    // Column maps + field counts from the profile schema (model-specific, not hard-coded here).
    let count = |cmd: &str| profile.record_schema(cmd).map(|s| s.field_count as usize);
    let name_col = |cmd: &str| {
        profile
            .record_schema(cmd)
            .and_then(|s| s.name_col)
            .unwrap_or(3)
    };
    let tech_col = |cmd: &str| profile.record_schema(cmd).and_then(|s| s.tech_col);
    let geo_cols = |cmd: &str| profile.record_schema(cmd).and_then(|s| s.geo);
    let chan_cols = |cmd: &str| profile.record_schema(cmd).and_then(|s| s.channel);

    match sys.kind {
        SystemKind::Conventional => {
            let Some(n) = count("Conventional") else {
                return;
            };
            let mut set = vec![
                (1, ids.next("CountyId")),
                (2, ids.next("StateId")),
                (name_col("Conventional"), sys.name.clone()),
            ];
            if let Some(tc) = tech_col("Conventional") {
                set.push((
                    tc,
                    sys.tech.clone().unwrap_or_else(|| "Conventional".into()),
                ));
            }
            out.lines.push(record("Conventional", n, &set));
            let sys_id = ids.next("SysRef"); // logical parent for groups (blanked in favorites)
            for group in &sys.groups {
                emit_group(
                    out, "C-Group", "C-Freq", &sys_id, group, geo_cols, chan_cols, count, name_col,
                    ids,
                );
            }
        }
        SystemKind::Trunk | SystemKind::Other => {
            let Some(n) = count("Trunk") else { return };
            let mut set = vec![
                (1, ids.next("TrunkId")),
                (2, ids.next("StateId")),
                (name_col("Trunk"), sys.name.clone()),
            ];
            if let (Some(tc), Some(tech)) = (tech_col("Trunk"), sys.tech.clone()) {
                set.push((tc, tech));
            }
            out.lines.push(record("Trunk", n, &set));
            for site in &sys.sites {
                emit_site(out, site, geo_cols, chan_cols, count, name_col, ids);
            }
            for group in &sys.groups {
                emit_group(
                    out, "T-Group", "TGID", "", group, geo_cols, chan_cols, count, name_col, ids,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_group(
    out: &mut Document,
    group_cmd: &str,
    chan_cmd: &str,
    _parent: &str,
    group: &ProgramGroup,
    geo_cols: impl Fn(&str) -> Option<crate::device::GeoColumns>,
    chan_cols: impl Fn(&str) -> Option<crate::device::ChannelColumns>,
    count: impl Fn(&str) -> Option<usize>,
    name_col: impl Fn(&str) -> usize,
    ids: &mut Ids,
) {
    if let Some(n) = count(group_cmd) {
        let mut set = vec![
            (1, ids.next("GroupId")),
            (2, ids.next("SysRef")),
            (name_col(group_cmd), group.name.clone()),
        ];
        if let (Some(g), Some(geo)) = (geo_cols(group_cmd), group.geo.as_ref()) {
            let (lat, lon, range) = fmt_geo(geo);
            set.push((g.lat, lat));
            set.push((g.lon, lon));
            set.push((g.range, range));
            set.push((g.shape, "Circle".into()));
        }
        out.lines.push(record(group_cmd, n, &set));
    }
    let Some(n) = count(chan_cmd) else { return };
    for ch in &group.channels {
        let mut set = vec![
            (1, ids.next("ChanId")),
            (2, ids.next("GroupRef")),
            (name_col(chan_cmd), ch.name.clone()),
        ];
        if let Some(c) = chan_cols(chan_cmd) {
            if let (Some(col), Some(hz)) = (c.freq, ch.freq_hz) {
                set.push((col, hz.to_string()));
            }
            if let (Some(col), Some(tg)) = (c.tgid, ch.tgid.as_ref()) {
                set.push((col, tg.clone()));
            }
            if let (Some(col), Some(m)) = (c.mode, ch.mode.as_ref()) {
                set.push((col, m.clone()));
            }
            if let (Some(col), Some(t)) = (c.tone, ch.tone.as_ref()) {
                set.push((col, t.clone()));
            }
            if let (Some(col), Some(st)) = (c.service_type, ch.service_type) {
                set.push((col, st.to_string()));
            }
        }
        out.lines.push(record(chan_cmd, n, &set));
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_site(
    out: &mut Document,
    site: &ProgramSite,
    geo_cols: impl Fn(&str) -> Option<crate::device::GeoColumns>,
    chan_cols: impl Fn(&str) -> Option<crate::device::ChannelColumns>,
    count: impl Fn(&str) -> Option<usize>,
    name_col: impl Fn(&str) -> usize,
    ids: &mut Ids,
) {
    if let Some(n) = count("Site") {
        let mut set = vec![
            (1, ids.next("SiteId")),
            (2, ids.next("TrunkRef")),
            (name_col("Site"), site.name.clone()),
        ];
        if let (Some(g), Some(geo)) = (geo_cols("Site"), site.geo.as_ref()) {
            let (lat, lon, range) = fmt_geo(geo);
            set.push((g.lat, lat));
            set.push((g.lon, lon));
            set.push((g.range, range));
            set.push((g.shape, "Circle".into()));
        }
        out.lines.push(record("Site", n, &set));
    }
    let Some(n) = count("T-Freq") else { return };
    for f in &site.frequencies {
        let mut set = vec![(1, ids.next("TFreqId")), (2, ids.next("SiteRef"))];
        if let Some(c) = chan_cols("T-Freq") {
            if let Some(col) = c.freq {
                set.push((col, f.freq_hz.to_string()));
            }
        }
        out.lines.push(record("T-Freq", n, &set));
    }
}

/// Synthesize a **favorites** `Document` from programmable systems, via the model's full-dialect
/// records + the existing, device-validated [`build_favorites`] (dialect + `DQKs_Status` + field
/// defaults, incl. the control-channel lock). The result is ready to write (or merge into an open
/// list with [`crate::favorites::merge_favorites`]).
pub fn synthesize_favorites(
    systems: &[ProgramSystem],
    profile: &dyn SdCardProfile,
    departments_on: bool,
) -> Document {
    let mut doc = Document::default();
    // The header the profile's `matches`/detection expects.
    let key = profile.model_key();
    doc.lines
        .push(record("TargetModel", 2, &[(1, key.target_model.clone())]));
    doc.lines.push(record(
        "FormatVersion",
        2,
        &[(1, key.format_version.clone())],
    ));

    let mut ids = Ids::new();
    for sys in systems {
        emit_system(&mut doc, sys, profile, &mut ids);
    }
    build_favorites(&doc, profile, departments_on)
}

/// Convenience: synthesize from RadioReference systems directly.
pub fn synthesize_favorites_from_rr(
    systems: &[RrSystem],
    profile: &dyn SdCardProfile,
    departments_on: bool,
) -> Document {
    let program: Vec<ProgramSystem> = systems.iter().map(ProgramSystem::from).collect();
    synthesize_favorites(&program, profile, departments_on)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::sds150::Sds150;
    use crate::rr::{
        Freq, RrConventionalSystem, RrTrunkedSystem, SiteFrequency, Subcat, Tag, Talkgroup, Trs,
        TrsSite,
    };

    fn p25() -> RrSystem {
        RrSystem::Trunked(RrTrunkedSystem {
            trs: Trs {
                s_name: "Statewide P25".into(),
                s_type: 5,
                s_flavor: 2,
                s_voice: 1,
                type_name: Some("Project 25".into()),
                flavor_name: Some("Phase II".into()),
                county_ids: vec![9001],
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
                frequencies: vec![
                    SiteFrequency {
                        freq_mhz: 851.0125,
                        use_: "d".into(),
                        lcn: Some(1),
                    },
                    SiteFrequency {
                        freq_mhz: 851.2625,
                        use_: "".into(),
                        lcn: Some(2),
                    },
                ],
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

    fn conventional() -> RrSystem {
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
                    tone: "TONE=C131.8".into(),
                    mode: "FMN".into(),
                    tags: vec![Tag {
                        tag_id: 8,
                        tag_descr: "Fire-Tac".into(),
                    }],
                }],
            )],
        })
    }

    #[test]
    fn synthesized_favorites_round_trip_and_are_well_formed() {
        let profile = Sds150::new();
        let doc = synthesize_favorites_from_rr(&[p25(), conventional()], &profile, false);

        // The serialized form is stable + parseable (the writer's round-trip contract).
        let bytes = doc.to_bytes();
        let reparsed = Document::parse(&bytes).expect("synthesized favorites parse");
        assert_eq!(reparsed.to_bytes(), bytes);

        // The right records got emitted (trunk + conventional vocabulary).
        let counts = doc.command_counts();
        for cmd in [
            "Trunk",
            "Site",
            "T-Freq",
            "T-Group",
            "TGID",
            "Conventional",
            "C-Group",
            "C-Freq",
        ] {
            assert!(counts.get(cmd).copied().unwrap_or(0) > 0, "missing {cmd}");
        }
        // One DQKs_Status synthesized per system (2 systems).
        assert_eq!(counts.get("DQKs_Status").copied(), Some(2));
        // Favorites carry no area tagging.
        assert_eq!(counts.get("AreaState"), None);

        // Favorites dialect: hierarchical records blank id@1 / parent@2.
        for line in &doc.lines {
            if matches!(
                line.command(),
                "Trunk"
                    | "Conventional"
                    | "Site"
                    | "T-Group"
                    | "C-Group"
                    | "TGID"
                    | "C-Freq"
                    | "T-Freq"
            ) {
                assert_eq!(line.field(1), Some(""), "{} id not blanked", line.command());
                assert_eq!(
                    line.field(2),
                    Some(""),
                    "{} parent not blanked",
                    line.command()
                );
            }
        }

        // build_favorites applied the control-channel lock defaults on T-Freq (col4=Off, col6=0).
        let tfreq = doc.lines.iter().find(|l| l.command() == "T-Freq").unwrap();
        assert_eq!(tfreq.field(4), Some("Off"));
        assert_eq!(tfreq.field(6), Some("0"));

        // A conventional channel kept its frequency + tone + normalized mode + service type.
        let cfreq = doc.lines.iter().find(|l| l.command() == "C-Freq").unwrap();
        assert_eq!(cfreq.field(5), Some("154265000")); // 154.265 MHz -> Hz
        assert_eq!(cfreq.field(6), Some("NFM")); // FMN -> NFM
        assert_eq!(cfreq.field(7), Some("TONE=C131.8"));
        assert_eq!(cfreq.field(8), Some("8"));

        // A talkgroup kept its decimal id + digital audio type + service type.
        let tgid = doc.lines.iter().find(|l| l.command() == "TGID").unwrap();
        assert_eq!(tgid.field(5), Some("12345"));
        assert_eq!(tgid.field(6), Some("DIGITAL"));
        assert_eq!(tgid.field(7), Some("2"));
    }
}
