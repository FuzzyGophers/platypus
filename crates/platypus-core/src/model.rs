// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Typed, **read-only** semantic views over a parsed [`Document`].
//!
//! This is the bridge from raw tab lines to domain concepts (id, parent, name,
//! geo) — and the foundation of the location-first feature. Views *borrow* the
//! underlying [`Line`]s rather than destructuring them, so the [`Document`]
//! remains the single source of truth and the byte-exact round trip is never at
//! risk. Filtering and lookups read; they never mutate.
//!
//! Column layouts come from each model's [`RecordSchema`], so this layer is
//! scanner-agnostic: point it at any [`SdCardProfile`] and it interprets that
//! model's files.

use std::collections::HashMap;

use crate::device::{RecordSchema, SdCardProfile};
use crate::format::{Document, Line};

/// Mean Earth radius in statute miles (the unit Uniden uses for Range).
const EARTH_RADIUS_MI: f64 = 3958.7613;

/// Great-circle distance between two lat/long points, in miles.
pub fn haversine_miles(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_MI * a.sqrt().asin()
}

/// Parse a `Key=Value` id field (e.g. `"SiteId=22044"`) to its numeric value.
/// Returns `None` for blank/unkeyed fields (e.g. the blanked favorites columns).
pub fn keyed_id(field: &str) -> Option<u64> {
    field.split_once('=').and_then(|(_, v)| v.parse().ok())
}

/// Coverage shape of a geo record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Circle,
    Rectangle,
    Unknown,
}

impl Shape {
    fn parse(s: &str) -> Self {
        match s {
            "Circle" => Shape::Circle,
            "Rectangle" => Shape::Rectangle,
            _ => Shape::Unknown,
        }
    }
}

/// A record's location: center point, configured range (miles), and shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geo {
    pub lat: f64,
    pub lon: f64,
    pub range_mi: f64,
    pub shape: Shape,
}

/// Coarse classification of a record, for matching in filters and UX.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Header,
    Conventional,
    CGroup,
    CFreq,
    Trunk,
    Site,
    TGroup,
    TFreq,
    Tgid,
    AreaState,
    AreaCounty,
    StateInfo,
    CountyInfo,
    Lm,
    Other,
}

impl RecordKind {
    pub fn from_command(command: &str) -> Self {
        match command {
            "TargetModel" | "FormatVersion" | "DateModified" => RecordKind::Header,
            "Conventional" => RecordKind::Conventional,
            "C-Group" => RecordKind::CGroup,
            "C-Freq" => RecordKind::CFreq,
            "Trunk" => RecordKind::Trunk,
            "Site" => RecordKind::Site,
            "T-Group" => RecordKind::TGroup,
            "T-Freq" => RecordKind::TFreq,
            "TGID" => RecordKind::Tgid,
            "AreaState" => RecordKind::AreaState,
            "AreaCounty" => RecordKind::AreaCounty,
            "StateInfo" => RecordKind::StateInfo,
            "CountyInfo" => RecordKind::CountyInfo,
            "LM" => RecordKind::Lm,
            _ => RecordKind::Other,
        }
    }
}

/// A typed view of one line, paired with its schema. Borrow-only.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    line: &'a Line,
    schema: &'a RecordSchema,
}

impl<'a> Record<'a> {
    /// Build a typed view of one line under a profile, or `None` if the profile
    /// doesn't recognize the command.
    pub fn new(line: &'a Line, profile: &'a dyn SdCardProfile) -> Option<Self> {
        profile
            .record_schema(line.command())
            .map(|schema| Record { line, schema })
    }

    /// The underlying raw line (escape hatch for fields not yet modeled).
    pub fn line(&self) -> &'a Line {
        self.line
    }

    pub fn command(&self) -> &'a str {
        self.line.command()
    }

    pub fn kind(&self) -> RecordKind {
        RecordKind::from_command(self.command())
    }

    /// This record's own id (e.g. the number in `SiteId=22044`).
    pub fn id(&self) -> Option<u64> {
        keyed_id(self.line.field(self.schema.id_col?)?)
    }

    /// The parent id linking up the hierarchy (e.g. `TrunkId=…`).
    pub fn parent_id(&self) -> Option<u64> {
        keyed_id(self.line.field(self.schema.parent_col?)?)
    }

    /// Human-readable name, if the record has one (and it's non-empty).
    pub fn name(&self) -> Option<&'a str> {
        let n = self.line.field(self.schema.name_col?)?;
        (!n.is_empty()).then_some(n)
    }

    /// Location, for records that carry one (`Site`, `T-Group`, `C-Group`).
    /// `None` if the record has no geo columns or they don't parse.
    pub fn geo(&self) -> Option<Geo> {
        let g = self.schema.geo.as_ref()?;
        let lat = self.line.field(g.lat)?.parse().ok()?;
        let lon = self.line.field(g.lon)?.parse().ok()?;
        let range_mi = self
            .line
            .field(g.range)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let shape = self
            .line
            .field(g.shape)
            .map(Shape::parse)
            .unwrap_or(Shape::Unknown);
        Some(Geo {
            lat,
            lon,
            range_mi,
            shape,
        })
    }

    /// System technology on a header line (`P25Standard`, `MotoTrbo`,
    /// `Conventional`, …).
    pub fn tech(&self) -> Option<&'a str> {
        nonempty(self.line.field(self.schema.tech_col?)?)
    }

    /// Channel frequency in Hz (conventional / control-channel records).
    pub fn frequency_hz(&self) -> Option<u64> {
        self.line
            .field(self.schema.channel.as_ref()?.freq?)?
            .parse()
            .ok()
    }

    /// Talkgroup id (trunked talkgroup records).
    pub fn talkgroup(&self) -> Option<&'a str> {
        nonempty(self.line.field(self.schema.channel.as_ref()?.tgid?)?)
    }

    /// Modulation / audio type (`NFM`/`AM`/`FM` or `ALL`/`ANALOG`/`DIGITAL`).
    pub fn mode(&self) -> Option<&'a str> {
        nonempty(self.line.field(self.schema.channel.as_ref()?.mode?)?)
    }

    /// Squelch tone (the `TONE=` prefix stripped, e.g. `C156.7`).
    pub fn tone(&self) -> Option<&'a str> {
        let v = self.line.field(self.schema.channel.as_ref()?.tone?)?;
        nonempty(v.strip_prefix("TONE=").unwrap_or(v))
    }

    /// The raw audio-option field verbatim, prefix intact: `TONE=`/`NAC=`/`ColorCode=`/
    /// `RAN=`/`Area=` (or a bare DCS code). Unlike [`tone`](Self::tone) this keeps the
    /// prefix so callers can tell a CTCSS/DCS tone from a P25 NAC or DMR color code.
    pub fn audio_option(&self) -> Option<&'a str> {
        nonempty(self.line.field(self.schema.channel.as_ref()?.tone?)?)
    }

    /// RadioReference service-type code, if the record carries one.
    pub fn service_type_code(&self) -> Option<u16> {
        self.line
            .field(self.schema.channel.as_ref()?.service_type?)?
            .parse()
            .ok()
    }
}

fn nonempty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

/// Service-type codes (`FuncTagId`) → human names, from the **Uniden File
/// Specification V2.00** "Service Type" sheet — the authoritative enumeration for the
/// SDS card format (mirrors the RadioReference service tags). This is the **single
/// source** every front-end reads (via the FFI), so no UI re-hardcodes the table.
/// Codes the spec marks `non` (5, 10, 18, 19, 27, 28, 35, 36) are simply absent; a UI
/// can fall back to "Service type N". `208`–`217` are the user's Custom slots.
pub const SERVICE_TYPES: &[(u16, &str)] = &[
    (1, "Multi-Dispatch"),
    (2, "Law Dispatch"),
    (3, "Fire Dispatch"),
    (4, "EMS Dispatch"),
    (6, "Multi-Tac"),
    (7, "Law Tac"),
    (8, "Fire-Tac"),
    (9, "EMS-Tac"),
    (11, "Interop"),
    (12, "Hospital"),
    (13, "Ham"),
    (14, "Public Works"),
    (15, "Aircraft"),
    (16, "Federal"),
    (17, "Business"),
    (20, "Railroad"),
    (21, "Other"),
    (22, "Multi-Talk"),
    (23, "Law Talk"),
    (24, "Fire-Talk"),
    (25, "EMS-Talk"),
    (26, "Transportation"),
    (29, "Emergency Ops"),
    (30, "Military"),
    (31, "Media"),
    (32, "Schools"),
    (33, "Security"),
    (34, "Utilities"),
    (37, "Corrections"),
    (208, "Custom 1"),
    (209, "Custom 2"),
    (210, "Custom 3"),
    (211, "Custom 4"),
    (212, "Custom 5"),
    (213, "Custom 6"),
    (214, "Custom 7"),
    (215, "Custom 8"),
    (216, "Racing Officials"),
    (217, "Racing Teams"),
];

/// Human name for a service-type code, from [`SERVICE_TYPES`]. `None` for the unused
/// (`non`) codes — a UI can fall back to "Service type N".
pub fn service_type_name(code: u16) -> Option<&'static str> {
    SERVICE_TYPES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| *name)
}

/// Iterate the typed records of a document under a given profile. Lines whose
/// command the profile doesn't recognize are skipped (they carry no semantics
/// we can interpret, but they remain in the `Document` for byte-exact output).
pub fn records<'a>(
    doc: &'a Document,
    profile: &'a dyn SdCardProfile,
) -> impl Iterator<Item = Record<'a>> {
    doc.lines.iter().filter_map(move |line| {
        let schema = profile.record_schema(line.command())?;
        Some(Record { line, schema })
    })
}

/// All geo-bearing records whose center lies within `radius_mi` of the point.
/// This is the "within X miles of here" primitive of the location-first design.
pub fn within_radius<'a>(
    doc: &'a Document,
    profile: &'a dyn SdCardProfile,
    lat: f64,
    lon: f64,
    radius_mi: f64,
) -> Vec<Record<'a>> {
    records(doc, profile)
        .filter(|r| {
            r.geo()
                .is_some_and(|g| haversine_miles(lat, lon, g.lat, g.lon) <= radius_mi)
        })
        .collect()
}

/// One county from the HPDB geo master (`CountyInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct County {
    pub id: u64,
    pub state_id: Option<u64>,
    pub name: String,
}

/// Lookup over the county master in `hpdb.cfg`. Maps `CountyId` ↔ name so the
/// UI can turn a county name into the ids the filter layer works with.
///
/// County names are **not** unique across states (many "Washington" counties),
/// so name lookups can return several entries — scope by state id when needed.
#[derive(Debug, Default)]
pub struct CountyIndex {
    counties: Vec<County>,
    by_id: HashMap<u64, usize>,
    by_name: HashMap<String, Vec<usize>>,
}

impl CountyIndex {
    /// Build from a parsed `hpdb.cfg` document.
    pub fn from_hpdb(doc: &Document, profile: &dyn SdCardProfile) -> Self {
        let mut idx = CountyIndex::default();
        for r in records(doc, profile).filter(|r| r.kind() == RecordKind::CountyInfo) {
            let Some(id) = r.id() else { continue };
            let county = County {
                id,
                state_id: r.parent_id(),
                name: r.name().unwrap_or("").to_string(),
            };
            let pos = idx.counties.len();
            idx.by_id.insert(id, pos);
            idx.by_name
                .entry(county.name.to_lowercase())
                .or_default()
                .push(pos);
            idx.counties.push(county);
        }
        idx
    }

    pub fn len(&self) -> usize {
        self.counties.len()
    }

    /// All counties, in file order. For listing in a UI picker.
    pub fn counties(&self) -> &[County] {
        &self.counties
    }

    pub fn is_empty(&self) -> bool {
        self.counties.is_empty()
    }

    pub fn by_id(&self, id: u64) -> Option<&County> {
        self.by_id.get(&id).map(|&i| &self.counties[i])
    }

    /// Display name for a county id.
    pub fn name(&self, id: u64) -> Option<&str> {
        self.by_id(id).map(|c| c.name.as_str())
    }

    /// Every county matching `name` (case-insensitive). May span states.
    pub fn counties_named(&self, name: &str) -> Vec<&County> {
        self.by_name
            .get(&name.to_lowercase())
            .map(|v| v.iter().map(|&i| &self.counties[i]).collect())
            .unwrap_or_default()
    }

    /// First county id matching `name` (case-insensitive), if any.
    pub fn id_by_name(&self, name: &str) -> Option<u64> {
        self.counties_named(name).first().map(|c| c.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_id_parses_and_rejects_blanks() {
        assert_eq!(keyed_id("SiteId=22044"), Some(22044));
        assert_eq!(keyed_id("TrunkId=7678"), Some(7678));
        assert_eq!(keyed_id(""), None);
        assert_eq!(keyed_id("NotKeyed"), None);
        // A keyed field with an empty value (the `Key=` shape) has no number.
        assert_eq!(keyed_id("SiteId="), None);
    }

    #[test]
    fn geo_with_malformed_coordinates_is_none() {
        use crate::device::Sds150;
        use crate::format::{Line, LineEnding};

        let profile = Sds150::new();
        let mk = |lat: &str, lon: &str| Line {
            // Site: id, parent, name, avoid, lat(5), lon(6), range(7).
            fields: ["Site", "SiteId=2", "TrunkId=1", "S1", "Off", lat, lon, "10"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ending: LineEnding::Crlf,
        };

        // Non-numeric latitude → None (and no panic).
        let bad_lat = mk("notanumber", "-100.0");
        assert_eq!(Record::new(&bad_lat, &profile).unwrap().geo(), None);
        // Empty longitude → None.
        let empty_lon = mk("45.0", "");
        assert_eq!(Record::new(&empty_lon, &profile).unwrap().geo(), None);
        // Sanity: well-formed coordinates DO parse (so None above is the malformed
        // path, not a missing schema).
        let good = mk("45.0", "-100.0");
        assert!(Record::new(&good, &profile).unwrap().geo().is_some());
    }

    #[test]
    fn shape_parse() {
        assert_eq!(Shape::parse("Circle"), Shape::Circle);
        assert_eq!(Shape::parse("Rectangle"), Shape::Rectangle);
        assert_eq!(Shape::parse("AUTO"), Shape::Unknown);
    }

    #[test]
    fn service_types_source_backs_the_name_lookup() {
        // Every entry in the source list resolves through the public accessor…
        for (code, name) in SERVICE_TYPES {
            assert_eq!(service_type_name(*code), Some(*name));
        }
        // …and a `non` (unused) code has no name.
        assert_eq!(service_type_name(5), None);
        assert_eq!(service_type_name(13), Some("Ham"));
    }

    #[test]
    fn haversine_known_distances() {
        // Same point is zero.
        assert!(haversine_miles(38.9, -77.0, 38.9, -77.0) < 1e-6);
        // DC -> NYC is ~204 miles.
        let d = haversine_miles(38.9072, -77.0369, 40.7128, -74.0060);
        assert!((200.0..210.0).contains(&d), "DC->NYC was {d}");
    }
}
