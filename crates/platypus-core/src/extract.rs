// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Extraction engine: segment a full HPDB file (`s_*.hpd`) into systems and
//! select an arbitrary subset of them.
//!
//! A **system** is a `Conventional` or `Trunk` line plus everything under it
//! (its `AreaState`/`AreaCounty` tags and its child Group/Freq/Site/TGID records)
//! up to the next system.
//!
//! The core API is generic: [`Extraction::select`] takes *any* predicate over a
//! [`System`], and a `System` exposes the metadata a caller needs to build one —
//! [`System::county_ids`], [`System::state_ids`], [`System::geos`],
//! [`System::name`], [`System::kind`]. Which filters exist (county, radius, state,
//! service type, free-text, and combinations) is a **UI** concern composed from
//! these primitives; this layer hard-codes no particular selection. The
//! [`by_county`] / [`within_radius`] free functions are just thin convenience
//! wrappers and examples — not the only way in.
//!
//! Output stays in the **full HPDB dialect** — selection only ever *keeps*
//! existing lines, it never synthesizes, so the result round-trips byte-for-byte.
//! Turning a selection into a card-ready favorites file is a separate step (see
//! [`crate::favorites`]) and carries an unresolved synthesis gap.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::device::SdCardProfile;
use crate::format::{Document, Line};
use crate::model::{self, Geo, RecordKind};

/// A contiguous run of lines making up one system (its header line first).
pub struct System<'a> {
    lines: &'a [Line],
    profile: &'a dyn SdCardProfile,
}

impl<'a> System<'a> {
    /// All lines in the system, header first.
    pub fn lines(&self) -> &'a [Line] {
        self.lines
    }

    /// The `Conventional` / `Trunk` line that starts the system.
    pub fn header(&self) -> &'a Line {
        &self.lines[0]
    }

    pub fn kind(&self) -> RecordKind {
        RecordKind::from_command(self.header().command())
    }

    /// System technology (`P25Standard`, `MotoTrbo`, `Conventional`, …), if known.
    pub fn tech(&self) -> Option<&'a str> {
        model::Record::new(self.header(), self.profile)?.tech()
    }

    /// True if this is a P25 trunked system (the band-plan-bearing kind).
    pub fn is_p25(&self) -> bool {
        self.tech().is_some_and(|t| t.contains("P25"))
    }

    /// System name as shown in the scanner UI.
    pub fn name(&self) -> Option<&'a str> {
        model::Record::new(self.header(), self.profile)?.name()
    }

    /// County ids this system is tagged with (`AreaCounty`). A statewide / multi-
    /// county system carries many — which is why it's correctly pulled in for any
    /// of its counties.
    ///
    /// `AreaCounty` is `[owner-id, county-tag]`: field 1 is the system's *own* id
    /// (e.g. `AgencyId=…`), field 2 is the actual county. Validated against a real
    /// multi-county/agency state file; single-county files can coincide on both
    /// columns and hide the distinction.
    pub fn county_ids(&self) -> BTreeSet<u64> {
        self.lines
            .iter()
            .filter(|l| l.command() == "AreaCounty")
            .filter_map(|l| model::keyed_id(l.field(2)?))
            .collect()
    }

    /// State ids this system is tagged with (`AreaState` field 2). Lets the UI
    /// filter by state as readily as by county.
    pub fn state_ids(&self) -> BTreeSet<u64> {
        self.lines
            .iter()
            .filter(|l| l.command() == "AreaState")
            .filter_map(|l| model::keyed_id(l.field(2)?))
            .collect()
    }

    /// Locations of this system's geo-bearing child records (Site/T-Group/C-Group).
    pub fn geos(&self) -> impl Iterator<Item = Geo> + '_ {
        self.lines
            .iter()
            .filter_map(|l| model::Record::new(l, self.profile))
            .filter_map(|r| r.geo())
    }

    pub fn is_in_county(&self, county_id: u64) -> bool {
        self.county_ids().contains(&county_id)
    }

    pub fn is_in_state(&self, state_id: u64) -> bool {
        self.state_ids().contains(&state_id)
    }

    /// True if any of the system's geo points is within `radius_mi` of the point.
    pub fn is_within(&self, lat: f64, lon: f64, radius_mi: f64) -> bool {
        self.geos()
            .any(|g| model::haversine_miles(lat, lon, g.lat, g.lon) <= radius_mi)
    }
}

/// A segmented HPDB document: the header preamble plus its systems.
pub struct Extraction<'a> {
    doc: &'a Document,
    profile: &'a dyn SdCardProfile,
    /// Header lines before the first system (`TargetModel`, `FormatVersion`, …).
    preamble: Range<usize>,
    /// Line range of each system.
    ranges: Vec<Range<usize>>,
}

impl<'a> Extraction<'a> {
    /// Segment a parsed HPDB document into systems.
    pub fn segment(doc: &'a Document, profile: &'a dyn SdCardProfile) -> Self {
        let is_start = |l: &Line| matches!(l.command(), "Conventional" | "Trunk");

        let starts: Vec<usize> = doc
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| is_start(l))
            .map(|(i, _)| i)
            .collect();

        let preamble = 0..starts.first().copied().unwrap_or(doc.lines.len());

        let mut ranges = Vec::with_capacity(starts.len());
        for (k, &start) in starts.iter().enumerate() {
            let end = starts.get(k + 1).copied().unwrap_or(doc.lines.len());
            ranges.push(start..end);
        }

        Extraction {
            doc,
            profile,
            preamble,
            ranges,
        }
    }

    pub fn systems(&self) -> impl Iterator<Item = System<'_>> + '_ {
        self.ranges.iter().map(move |r| System {
            lines: &self.doc.lines[r.clone()],
            profile: self.profile,
        })
    }

    pub fn system_count(&self) -> usize {
        self.ranges.len()
    }

    pub fn preamble_lines(&self) -> &'a [Line] {
        &self.doc.lines[self.preamble.clone()]
    }

    /// Build a new full-dialect document from the preamble plus the systems that
    /// match `pred`. The result round-trips (it is a subset of real lines).
    pub fn select(&self, pred: impl Fn(&System) -> bool) -> Document {
        let mut lines = self.preamble_lines().to_vec();
        for sys in self.systems().filter(|s| pred(s)) {
            lines.extend_from_slice(sys.lines());
        }
        Document { lines }
    }

    /// Channel-level subset: keep only the named voice channels within the named
    /// systems. `keep` maps a **system index** (as enumerated by [`Self::systems`])
    /// to the set of **voice-channel indices** to keep (index = order of voice
    /// channels — `TGID`/`C-Freq`, see [`is_voice_channel`] — within that system,
    /// matching the catalog/FFI enumeration the UI's cart ids are built from).
    ///
    /// Each kept channel's scaffolding is preserved: the system header + area tags,
    /// every `Site` with its control freqs and band plans, and the parent
    /// `T-Group`/`C-Group` (plus its `Rectangle` bounds) of any kept channel. A
    /// group with no kept child is dropped. Output stays full-dialect (selection
    /// only, with the preamble) — feed it to [`crate::favorites::build_favorites`].
    pub fn subset_channels(&self, keep: &BTreeMap<usize, BTreeSet<usize>>) -> Document {
        let mut lines = self.preamble_lines().to_vec();
        lines.extend(self.subset_system_lines(keep));
        Document { lines }
    }

    /// The pruned system lines only (no preamble) — lets a caller merge subsets from
    /// several source documents under one preamble (favorites span states).
    pub fn subset_system_lines(&self, keep: &BTreeMap<usize, BTreeSet<usize>>) -> Vec<Line> {
        let mut out = Vec::new();
        for (si, sys) in self.systems().enumerate() {
            if let Some(channels) = keep.get(&si) {
                push_system_subset(sys.lines(), channels, &mut out);
            }
        }
        out
    }
}

/// True for the user-selectable **voice channels** — talkgroups (`TGID`) and
/// conventional frequencies (`C-Freq`). Excludes the control-channel `T-Freq`,
/// which is trunk scaffolding (a frequency, but not a listenable selection): it
/// must NOT be counted as a channel by the catalog or the selection cart.
pub fn is_voice_channel(command: &str) -> bool {
    matches!(command, "TGID" | "C-Freq")
}

/// Prune one system's lines to the kept voice channels plus the scaffolding they
/// need. `keep` is the set of voice-channel indices (within this system) to retain.
fn push_system_subset(sys_lines: &[Line], keep: &BTreeSet<usize>, out: &mut Vec<Line>) {
    let n = sys_lines.len();
    let is_group = |c: &str| matches!(c, "T-Group" | "C-Group");

    // Mark which channel lines are kept (by voice-channel index), and which groups
    // therefore must survive (a group with at least one kept child).
    let mut kept_channel = vec![false; n];
    let mut group_keep = vec![false; n];
    let mut ci = 0usize;
    let mut current_group: Option<usize> = None;
    for (i, line) in sys_lines.iter().enumerate() {
        let cmd = line.command();
        if is_group(cmd) {
            current_group = Some(i);
        } else if is_voice_channel(cmd) {
            if keep.contains(&ci) {
                kept_channel[i] = true;
                if let Some(g) = current_group {
                    group_keep[g] = true;
                }
            }
            ci += 1;
        }
    }

    // Emit: scaffolding always; groups/rectangles only if the group survives;
    // channels only if kept.
    current_group = None;
    for (i, line) in sys_lines.iter().enumerate() {
        let cmd = line.command();
        if is_group(cmd) {
            current_group = Some(i);
            if group_keep[i] {
                out.push(line.clone());
            }
        } else if is_voice_channel(cmd) {
            if kept_channel[i] {
                out.push(line.clone());
            }
        } else if cmd == "Rectangle" {
            if current_group.is_some_and(|g| group_keep[g]) {
                out.push(line.clone());
            }
        } else {
            // header / area tags / site / t-freq / band plan — scaffolding.
            current_group = None;
            out.push(line.clone());
        }
    }
}

/// Convenience example: extract every system tagged with `county_id`. Equivalent
/// to `Extraction::segment(..).select(|s| s.is_in_county(county_id))` — the UI is
/// free to build its own predicate instead.
pub fn by_county(doc: &Document, profile: &dyn SdCardProfile, county_id: u64) -> Document {
    Extraction::segment(doc, profile).select(|s| s.is_in_county(county_id))
}

/// Convenience example: extract every system with a geo point within `radius_mi`
/// of the point. A thin wrapper over [`Extraction::select`].
pub fn within_radius(
    doc: &Document,
    profile: &dyn SdCardProfile,
    lat: f64,
    lon: f64,
    radius_mi: f64,
) -> Document {
    Extraction::segment(doc, profile).select(|s| s.is_within(lat, lon, radius_mi))
}

/// Prune a document to only the geo-bearing children (`Site`, `T-Group`,
/// `C-Group`) within `radius_mi` of a point, dropping each out-of-range one along
/// with its children (`T-Freq`/`BandPlan_P25` for a site, `TGID` for a talkgroup
/// group, `C-Freq`/`Rectangle` for a conventional group). System headers,
/// `DQKs_Status`, and area tags pass through.
///
/// This is the heart of location-first selection at the **site and talkgroup**
/// level: out of a statewide trunk's hundreds of sites and talkgroup groups, keep
/// the few near you — replacing the manual deselection of 100+ entries in Sentinel.
/// Records without a location are kept. The result round-trips (only whole real
/// lines are dropped).
pub fn filter_within_radius(
    doc: &Document,
    profile: &dyn SdCardProfile,
    lat: f64,
    lon: f64,
    radius_mi: f64,
) -> Document {
    let mut lines = Vec::with_capacity(doc.lines.len());
    let mut dropping = false;
    for line in &doc.lines {
        match line.command() {
            // Geo-bearing parents: keep or drop based on distance.
            "Site" | "T-Group" | "C-Group" => {
                let keep = model::Record::new(line, profile)
                    .and_then(|r| r.geo())
                    .is_none_or(|g| model::haversine_miles(lat, lon, g.lat, g.lon) <= radius_mi);
                dropping = !keep;
                if keep {
                    lines.push(line.clone());
                }
            }
            // Children of the parent above.
            "T-Freq" | "TGID" | "C-Freq" | "BandPlan_P25" | "Rectangle" => {
                if !dropping {
                    lines.push(line.clone());
                }
            }
            // Trunk/Conventional headers, DQKs, area tags, etc. — always kept.
            _ => {
                dropping = false;
                lines.push(line.clone());
            }
        }
    }
    Document { lines }
}

#[cfg(test)]
mod subset_tests {
    use super::*;
    use crate::device::Sds150;
    use crate::favorites;
    use crate::format::LineEnding;

    fn line(fields: &[&str]) -> Line {
        Line {
            fields: fields.iter().map(|s| s.to_string()).collect(),
            ending: LineEnding::Crlf,
        }
    }

    // System 0: P25 trunk — a site (with control freq) + two groups of two
    // talkgroups. System 1: conventional — one group of two frequencies.
    fn doc() -> Document {
        Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&["FormatVersion", "1.00"]),
                line(&[
                    "Trunk",
                    "TrunkId=1",
                    "StateId=1",
                    "SysA",
                    "Off",
                    "",
                    "P25Standard",
                ]),
                line(&["AreaCounty", "TrunkId=1", "CountyId=5"]),
                line(&[
                    "Site",
                    "SiteId=1",
                    "TrunkId=1",
                    "SiteOne",
                    "Off",
                    "45.0",
                    "-100.0",
                    "10",
                ]),
                line(&["T-Freq", "TFreqId=1", "SiteId=1", "", "", "851000000"]),
                line(&["T-Group", "TGroupId=1", "TrunkId=1", "PoliceGrp"]),
                line(&["TGID", "Tid=1", "TGroupId=1", "AlphaDisp", "Off", "100"]), // ci 0
                line(&["TGID", "Tid=2", "TGroupId=1", "AlphaTac", "Off", "101"]),  // ci 1
                line(&["T-Group", "TGroupId=2", "TrunkId=1", "FireGrp"]),
                line(&["TGID", "Tid=3", "TGroupId=2", "BravoFire", "Off", "200"]), // ci 2
                line(&["TGID", "Tid=4", "TGroupId=2", "BravoGround", "Off", "201"]), // ci 3
                line(&["Conventional", "CountyId=5", "StateId=1", "SysB"]),
                line(&["C-Group", "CGroupId=1", "CountyId=5", "EmsGrp"]),
                line(&[
                    "C-Freq",
                    "CFreqId=1",
                    "CGroupId=1",
                    "EmsAmb",
                    "Off",
                    "154000000",
                ]), // ci 0
                line(&[
                    "C-Freq",
                    "CFreqId=2",
                    "CGroupId=1",
                    "EmsHosp",
                    "Off",
                    "155000000",
                ]), // ci 1
            ],
        }
    }

    #[test]
    fn keeps_only_selected_channels_with_scaffolding() {
        let profile = Sds150::new();
        let d = doc();
        let ext = Extraction::segment(&d, &profile);
        let mut keep = BTreeMap::new();
        keep.insert(0usize, BTreeSet::from([0usize, 2])); // AlphaDisp + BravoFire
        keep.insert(1usize, BTreeSet::from([1usize])); // EmsHosp
        let s = String::from_utf8(ext.subset_channels(&keep).to_bytes()).unwrap();

        // kept channels present, dropped ones absent
        for kept in ["AlphaDisp", "BravoFire", "EmsHosp"] {
            assert!(s.contains(kept), "missing {kept}");
        }
        for dropped in ["AlphaTac", "BravoGround", "EmsAmb"] {
            assert!(!s.contains(dropped), "should have dropped {dropped}");
        }
        // scaffolding kept: preamble, site, control freq, and the parent groups of
        // kept channels (both trunk groups, the conventional group).
        for scaffold in [
            "TargetModel",
            "SiteOne",
            "851000000",
            "PoliceGrp",
            "FireGrp",
            "EmsGrp",
        ] {
            assert!(s.contains(scaffold), "missing scaffold {scaffold}");
        }
    }

    #[test]
    fn drops_groups_with_no_kept_child_and_unselected_systems() {
        let profile = Sds150::new();
        let d = doc();
        let ext = Extraction::segment(&d, &profile);
        let mut keep = BTreeMap::new();
        keep.insert(0usize, BTreeSet::from([0usize])); // only AlphaDisp (PoliceGrp)
        let s = String::from_utf8(ext.subset_channels(&keep).to_bytes()).unwrap();

        assert!(s.contains("PoliceGrp"));
        assert!(s.contains("AlphaDisp"));
        // FireGrp has no kept child -> dropped entirely.
        assert!(!s.contains("FireGrp"));
        assert!(!s.contains("BravoFire"));
        // System 1 wasn't selected at all.
        assert!(!s.contains("SysB"));
        assert!(!s.contains("EmsHosp"));
    }

    #[test]
    fn subset_feeds_a_complete_favorites_build() {
        let profile = Sds150::new();
        let d = doc();
        let ext = Extraction::segment(&d, &profile);
        let mut keep = BTreeMap::new();
        keep.insert(0usize, BTreeSet::from([0usize]));
        let sub = ext.subset_channels(&keep);
        let fav = favorites::build_favorites(&sub, &profile, false);

        // area tag dropped, DQKs synthesized, and byte round-trips.
        assert!(favorites::is_favorites_dialect(&fav));
        assert!(favorites::has_synthesized_records(&fav, &profile));
        assert_eq!(
            fav.to_bytes(),
            Document::parse(&fav.to_bytes()).unwrap().to_bytes()
        );
    }
}
