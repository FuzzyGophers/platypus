// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Location-first **county placement** of channel groups.
//!
//! Statewide modernized P25 systems (SAFE-T, Minnesota ARMER, Arkansas
//! AWIN, …) live in the `_MultipleStates` file tagged `AreaCounty=CountyId=0`, so
//! they can't be placed by a system's county tags — they'd vanish from the county
//! drill-down. But their talkgroup **groups** are county-organized:
//!
//! - a conventional `C-Group` carries an explicit `CountyId` (its `parent_id`);
//! - a trunked `T-Group` is geo-tagged (`lat`/`lon`/`range`), one group per county
//!   (e.g. a statewide P25's "Alpha County (20)" @ `45.00,-100.00` r=15) plus wider
//!   statewide groups.
//!
//! This module places a group — and thus its channels — into the counties its
//! **coverage** reaches, by geography. It is general across systems and states
//! (verified against SAFE-T, ARMER/MN, AWIN/AR, …); no per-system parsing.

use std::collections::{BTreeSet, HashMap};

use crate::device::SdCardProfile;
use crate::format::Document;
use crate::model::{haversine_miles, CountyIndex, Geo, Record, RecordKind};

/// Max group range (miles) still treated as **county-scale**. Per-county talkgroup
/// groups run ~15–40 mi; a wider group is statewide/regional/national and is NOT
/// blanketed across every county it geometrically reaches (a nationwide federal
/// system would otherwise land in thousands). Such groups fall to the state-level
/// "Statewide" bucket instead. Tunable.
pub const COUNTY_SCALE_MI: f64 = 45.0;

/// One county's centroid plus the state it belongs to.
#[derive(Debug, Clone, Copy)]
struct Centroid {
    lat: f64,
    lon: f64,
    state_id: Option<u64>,
}

/// County centroids (with state), for geo-placing groups into counties.
/// Coordinates are averaged from conventional `C-Group`s — which carry both a
/// `CountyId` and geo — and the state comes from the `hpdb.cfg` county master.
#[derive(Debug, Default, Clone)]
pub struct CountyCentroids {
    map: HashMap<u64, Centroid>,
}

/// Builder: accumulate `C-Group` coordinates across the loaded files, then
/// [`finish`](CentroidBuilder::finish) with the county master for the state ids.
#[derive(Debug, Default)]
pub struct CentroidBuilder {
    // county id -> (sum_lat, sum_lon, count)
    accum: HashMap<u64, (f64, f64, f64)>,
}

impl CentroidBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accumulate every `C-Group`'s (CountyId, geo) from one parsed document.
    pub fn add_doc(&mut self, doc: &Document, profile: &dyn SdCardProfile) {
        for line in &doc.lines {
            let Some(rec) = Record::new(line, profile) else {
                continue;
            };
            if rec.kind() != RecordKind::CGroup {
                continue;
            }
            let Some(county) = rec.parent_id() else {
                continue;
            };
            if county == 0 {
                continue;
            }
            let Some(g) = rec.geo() else { continue };
            if g.lat == 0.0 && g.lon == 0.0 {
                continue;
            }
            let e = self.accum.entry(county).or_insert((0.0, 0.0, 0.0));
            e.0 += g.lat;
            e.1 += g.lon;
            e.2 += 1.0;
        }
    }

    /// Finalize into centroids, attaching each county's state from the master.
    pub fn finish(self, counties: &CountyIndex) -> CountyCentroids {
        let mut map = HashMap::with_capacity(self.accum.len());
        for (county, (slat, slon, n)) in self.accum {
            if n <= 0.0 {
                continue;
            }
            map.insert(
                county,
                Centroid {
                    lat: slat / n,
                    lon: slon / n,
                    state_id: counties.by_id(county).and_then(|c| c.state_id),
                },
            );
        }
        CountyCentroids { map }
    }
}

impl CountyCentroids {
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Counties whose centroid lies within `geo`'s coverage circle, optionally
    /// restricted to `states` (so a wide group doesn't bleed across a border). A
    /// cheap bounding-box pre-filter avoids a haversine per county.
    fn covered(&self, geo: Geo, states: &BTreeSet<u64>) -> BTreeSet<u64> {
        let mut out = BTreeSet::new();
        if geo.range_mi <= 0.0 {
            return out;
        }
        // ~miles per degree: 69 for latitude; longitude shrinks with latitude but
        // 40 is a safe lower bound across the US/Canada, so the box stays generous.
        let dlat = geo.range_mi / 69.0 + 0.5;
        let dlon = geo.range_mi / 40.0 + 0.5;
        for (&county, c) in &self.map {
            if (c.lat - geo.lat).abs() > dlat || (c.lon - geo.lon).abs() > dlon {
                continue;
            }
            if !states.is_empty() {
                if let Some(s) = c.state_id {
                    if !states.contains(&s) {
                        continue;
                    }
                }
            }
            if haversine_miles(geo.lat, geo.lon, c.lat, c.lon) <= geo.range_mi {
                out.insert(county);
            }
        }
        out
    }
}

/// The counties a group's coverage reaches (location-first placement):
/// - a `C-Group` → its explicit `CountyId` (when real, i.e. ≠ 0);
/// - a `T-Group` with geo → every county centroid within the group's range,
///   restricted to the system's `states` (`AreaState`) so coverage doesn't bleed
///   across a border;
/// - otherwise empty — the caller then falls back to placing the channel at the
///   **state** level (`states`), or unplaced if it has none.
pub fn group_counties(
    group: &Record,
    states: &BTreeSet<u64>,
    centroids: &CountyCentroids,
) -> BTreeSet<u64> {
    match group.kind() {
        RecordKind::CGroup => match group.parent_id() {
            Some(c) if c != 0 => BTreeSet::from([c]),
            _ => BTreeSet::new(),
        },
        RecordKind::TGroup => {
            let Some(g) = group.geo() else {
                return BTreeSet::new();
            };
            if (g.lat == 0.0 && g.lon == 0.0) || g.range_mi > COUNTY_SCALE_MI {
                // No usable point, or a statewide/national group — leave it for the
                // state-level fallback rather than blanketing every county.
                return BTreeSet::new();
            }
            centroids.covered(g, states)
        }
        _ => BTreeSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Sds150;
    use crate::format::{Line, LineEnding};

    fn line(fields: &[&str]) -> Line {
        Line {
            fields: fields.iter().map(|s| s.to_string()).collect(),
            ending: LineEnding::Crlf,
        }
    }

    // Synthetic, fictional coordinates — no real location.
    // Two counties in state 90: Alpha @ (45.0,-120.0), Bravo @ (45.5,-120.0)
    // (~35 mi apart). One county in state 91: Cedar @ (40.0,-100.0).
    fn master() -> Document {
        Document {
            lines: vec![
                line(&["CountyInfo", "CountyId=9001", "StateId=90", "Alpha"]),
                line(&["CountyInfo", "CountyId=9002", "StateId=90", "Bravo"]),
                line(&["CountyInfo", "CountyId=9003", "StateId=91", "Cedar"]),
            ],
        }
    }

    fn centroids() -> CountyCentroids {
        // C-Groups supply the coordinates (CountyId + geo).
        let doc = Document {
            lines: vec![
                line(&[
                    "C-Group",
                    "CGroupId=1",
                    "CountyId=9001",
                    "A",
                    "Off",
                    "45.0",
                    "-120.0",
                    "10",
                    "Circle",
                ]),
                line(&[
                    "C-Group",
                    "CGroupId=2",
                    "CountyId=9002",
                    "B",
                    "Off",
                    "45.5",
                    "-120.0",
                    "10",
                    "Circle",
                ]),
                line(&[
                    "C-Group",
                    "CGroupId=3",
                    "CountyId=9003",
                    "C",
                    "Off",
                    "40.0",
                    "-100.0",
                    "10",
                    "Circle",
                ]),
            ],
        };
        let profile = Sds150::new();
        let mut b = CentroidBuilder::new();
        b.add_doc(&doc, &profile);
        b.finish(&CountyIndex::from_hpdb(&master(), &profile))
    }

    #[test]
    fn conventional_group_uses_explicit_county() {
        let profile = Sds150::new();
        let l = line(&[
            "C-Group",
            "CGroupId=7",
            "CountyId=9002",
            "X",
            "Off",
            "0",
            "0",
            "0",
            "Circle",
        ]);
        let rec = Record::new(&l, &profile).unwrap();
        let got = group_counties(&rec, &BTreeSet::from([90]), &centroids());
        assert_eq!(got, BTreeSet::from([9002]));
    }

    #[test]
    fn small_trunk_group_lands_in_its_county_only() {
        let profile = Sds150::new();
        // 12-mile group centered on Alpha -> just Alpha (Bravo is ~35 mi away).
        let l = line(&[
            "T-Group",
            "TGroupId=1",
            "TrunkId=1",
            "Alpha Co",
            "Off",
            "45.0",
            "-120.0",
            "12",
            "Circle",
        ]);
        let rec = Record::new(&l, &profile).unwrap();
        let got = group_counties(&rec, &BTreeSet::from([90]), &centroids());
        assert_eq!(got, BTreeSet::from([9001]));
    }

    #[test]
    fn county_scale_group_covers_multiple_nearby_in_state_counties() {
        let profile = Sds150::new();
        // A 40-mile group between Alpha and Bravo (~35 mi apart) covers both, but
        // stays within the county-scale cap.
        let l = line(&[
            "T-Group",
            "TGroupId=2",
            "TrunkId=1",
            "Metro",
            "Off",
            "45.25",
            "-120.0",
            "40",
            "Circle",
        ]);
        let rec = Record::new(&l, &profile).unwrap();
        let got = group_counties(&rec, &BTreeSet::from([90]), &centroids());
        assert_eq!(got, BTreeSet::from([9001, 9002]));
    }

    #[test]
    fn statewide_range_group_is_not_county_placed() {
        let profile = Sds150::new();
        // A 165-mile statewide group is past the county-scale cap → no county (it
        // belongs at the state level), even though it geometrically covers them.
        let l = line(&[
            "T-Group",
            "TGroupId=3",
            "TrunkId=1",
            "Statewide",
            "Off",
            "45.2",
            "-120.0",
            "165",
            "Circle",
        ]);
        let rec = Record::new(&l, &profile).unwrap();
        assert!(group_counties(&rec, &BTreeSet::from([90]), &centroids()).is_empty());
    }

    #[test]
    fn state_filter_blocks_cross_border_bleed() {
        let profile = Sds150::new();
        // A county-scale group right on Cedar; restricted to state 91 it's just Cedar
        // (and the state filter would exclude any state-90 neighbor regardless).
        let l = line(&[
            "T-Group",
            "TGroupId=4",
            "TrunkId=1",
            "Edge",
            "Off",
            "40.0",
            "-100.0",
            "20",
            "Circle",
        ]);
        let rec = Record::new(&l, &profile).unwrap();
        let got = group_counties(&rec, &BTreeSet::from([91]), &centroids());
        assert_eq!(got, BTreeSet::from([9003]));
    }

    #[test]
    fn group_without_geo_or_county_is_unplaced() {
        let profile = Sds150::new();
        let l = line(&[
            "T-Group",
            "TGroupId=4",
            "TrunkId=1",
            "NoGeo",
            "Off",
            "0",
            "0",
            "0",
            "Circle",
        ]);
        let rec = Record::new(&l, &profile).unwrap();
        assert!(group_counties(&rec, &BTreeSet::from([90]), &centroids()).is_empty());
    }
}
