// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Uniden **SDS150** profile (file family `BCDx36HP`, FormatVersion `1.00`).
//!
//! Every constant here was validated against a real SDS150 SD card; the committed
//! fixtures are the synthetic set in `samples/synthetic/`. This is the template to
//! copy when adding another model — implement [`SdCardProfile`], fill in the layout
//! and schema from that model's own card, and register it.

use super::profile::{
    ChannelColumns, GeoColumns, ModelKey, RadioClass, RadioProfile, RecordSchema, ScannerLimits,
    SdCardProfile, SdLayout,
};

/// SD-card layout for the SDS150. `discovery_cfg` intentionally holds the
/// misspelled real filename.
const LAYOUT: SdLayout = SdLayout {
    model_folder: "BCDx36HP",
    favorites_dir: "favorites_lists",
    favorites_list_cfg: "f_list.cfg",
    hpdb_dir: "HPDB",
    hpdb_cfg: "hpdb.cfg",
    profile_cfg: "profile.cfg",
    discovery_cfg: "discvery.cfg", // sic — real cards misspell it
    app_data_cfg: "app_data.cfg",  // writer MUST delete this (spec CRITICAL RULE)
};

/// Geo column layouts (verified on a real card). `Site` puts its shape flag at
/// column 11; the group records put it at 8. lat/lon/range are consistent.
const GEO_SITE: GeoColumns = GeoColumns {
    lat: 5,
    lon: 6,
    range: 7,
    shape: 11,
};
const GEO_GROUP: GeoColumns = GeoColumns {
    lat: 5,
    lon: 6,
    range: 7,
    shape: 8,
};

/// Record schemas observed on a real SDS150 card. Field counts and column indices
/// are validated against `samples/`; the byte-exact round trip never relies on
/// them — they bridge raw lines to the typed `model` layer.
const SCHEMAS: &[RecordSchema] = &[
    // Header / global.
    RecordSchema::header("TargetModel", 2),
    RecordSchema::header("FormatVersion", 2),
    RecordSchema::header("DateModified", 2),
    // Conventional hierarchy (name@3; C-Group carries geo; C-Freq is a channel).
    RecordSchema::hier("Conventional", 15, Some(3), None).with_tech(6),
    RecordSchema::hier("C-Group", 11, Some(3), Some(GEO_GROUP)),
    RecordSchema::hier("C-Freq", 18, Some(3), None).with_channel(ChannelColumns {
        freq: Some(5),
        tgid: None,
        mode: Some(6),
        tone: Some(7),
        service_type: Some(8),
    }),
    // Trunked hierarchy (Site and T-Group carry geo; TGID/T-Freq are channels).
    RecordSchema::hier("Trunk", 22, Some(3), None).with_tech(6),
    RecordSchema::hier("Site", 20, Some(3), Some(GEO_SITE)),
    RecordSchema::hier("T-Group", 10, Some(3), Some(GEO_GROUP)),
    RecordSchema::hier("T-Freq", 9, None, None).with_channel(ChannelColumns {
        freq: Some(5),
        tgid: None,
        mode: None,
        tone: None,
        service_type: None,
    }),
    RecordSchema::hier("TGID", 17, Some(3), None).with_channel(ChannelColumns {
        freq: None,
        tgid: Some(5),
        mode: Some(6),
        tone: None,
        service_type: Some(7),
    }),
    // Area tagging (full HPDB dialect only). CountyId@1; see `model` for parsing.
    RecordSchema::header("AreaState", 3),
    RecordSchema::header("AreaCounty", 3),
    RecordSchema::header("Rectangle", 6),
    RecordSchema::header("BandPlan_P25", 50),
    RecordSchema::header("DQKs_Status", 102),
    // hpdb.cfg geo master / index (name@3).
    RecordSchema::index("StateInfo", 5, Some(3), None),
    RecordSchema::index("CountyInfo", 4, Some(3), None),
    RecordSchema::index("LM", 9, None, None), // special geo lat@7/lon@8; see `model`
    RecordSchema::header("LM_Frequency", 4),
];

/// The SDS150 scanner profile. Stateless; cheap to construct.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sds150;

impl Sds150 {
    pub fn new() -> Self {
        Sds150
    }
}

impl RadioProfile for Sds150 {
    fn id(&self) -> &'static str {
        "sds150"
    }

    fn product_name(&self) -> &'static str {
        "SDS150"
    }

    fn maker(&self) -> &'static str {
        "Uniden"
    }

    fn transport(&self) -> &'static str {
        "SD card"
    }

    fn class(&self) -> RadioClass {
        RadioClass::SdCardScanner
    }

    fn as_sd_card(&self) -> Option<&dyn SdCardProfile> {
        Some(self)
    }
}

impl SdCardProfile for Sds150 {
    fn limits(&self) -> ScannerLimits {
        // Per the SDS150 specs (Uniden DMA): 256 favorites lists, 1 MB each, and 100
        // quick keys — the 100 quick keys are exactly the 100 `DQKs_Status` slots in
        // the favorites format (our internal corroboration).
        ScannerLimits {
            max_favorites_lists: 256,
            max_favorite_list_bytes: 1_048_576,
            quick_keys: 100,
        }
    }

    fn model_key(&self) -> ModelKey {
        ModelKey::new("BCDx36HP", "1.00")
    }

    fn serial_model_id(&self) -> Option<&'static str> {
        // Reported over serial per the V2.00 remote-command spec.
        Some("SDS150GBT")
    }

    fn sd_layout(&self) -> &SdLayout {
        &LAYOUT
    }

    fn record_schema(&self, command: &str) -> Option<&RecordSchema> {
        SCHEMAS.iter().find(|s| s.command == command)
    }

    fn favorites_synthesized_records(&self) -> &'static [&'static str] {
        // What we synthesize for a complete favorites file. Only `DQKs_Status`:
        // device-validated. `BandPlan_P25` is favorites-only too but is NOT
        // synthesized — almost no P25 site uses one, and adding it wrongly stops a
        // trunk from locking (see `favorites::build_favorites`). `Rectangle` occurs
        // in HPDB source and is carried through by extraction, not synthesized.
        &["DQKs_Status"]
    }

    fn dqks_status_line(&self, on: bool) -> Option<Vec<String>> {
        // Observed structure: command, one blank field, then 100 uniform slots,
        // every real card showing either all "On" or all "Off".
        let value = if on { "On" } else { "Off" };
        let mut fields = Vec::with_capacity(2 + DQKS_SLOTS);
        fields.push("DQKs_Status".to_string());
        fields.push(String::new());
        fields.extend(std::iter::repeat_n(value.to_string(), DQKS_SLOTS));
        Some(fields)
    }

    fn standard_p25_bandplan(&self) -> Option<&'static [&'static str]> {
        Some(STANDARD_P25_BANDPLAN)
    }

    fn avoid_column(&self, command: &str) -> Option<usize> {
        // Field 4 is the Avoid flag on every system / department / channel record
        // (validated across Trunk/Conventional/T-Group/C-Group/TGID/C-Freq/T-Freq).
        match command {
            "Trunk" | "Conventional" | "T-Group" | "C-Group" | "TGID" | "C-Freq" | "T-Freq" => {
                Some(4)
            }
            _ => None,
        }
    }

    fn priority_column(&self, command: &str) -> Option<usize> {
        // Priority Channel flag — Uniden File Spec V2.00: C-Freq field 17, TGID
        // field 15.
        match command {
            "C-Freq" => Some(17),
            "TGID" => Some(15),
            _ => None,
        }
    }

    fn channel_value_fields(&self) -> &'static [&'static str] {
        &[
            "delay",
            "attenuator",
            "volumeOffset",
            "modulation",
            "audioType",
            "numberTag",
            "alertTone",
            "alertVolume",
            "alertColor",
            "alertPattern",
        ]
    }

    fn channel_value_column(&self, command: &str, field: &str) -> Option<usize> {
        // Editable per-channel value fields — Uniden File Spec V2.00 record layouts.
        // C-Freq (conventional freq) and TGID (talkgroup) differ; some fields exist on
        // only one (attenuator/modulation are C-Freq only; audioType is TGID only).
        match (command, field) {
            ("C-Freq", "delay") => Some(10),
            ("TGID", "delay") => Some(8),
            ("C-Freq", "attenuator") => Some(9),
            ("C-Freq", "volumeOffset") => Some(11),
            ("TGID", "volumeOffset") => Some(9),
            ("C-Freq", "modulation") => Some(6),
            ("TGID", "audioType") => Some(6),
            // Alerts + number tag: C-Freq (0-17) shifts +2 vs TGID (0-16) from field 12.
            ("C-Freq", "alertTone") => Some(12),
            ("TGID", "alertTone") => Some(10),
            ("C-Freq", "alertVolume") => Some(13),
            ("TGID", "alertVolume") => Some(11),
            ("C-Freq", "alertColor") => Some(14),
            ("TGID", "alertColor") => Some(12),
            ("C-Freq", "alertPattern") => Some(15),
            ("TGID", "alertPattern") => Some(13),
            ("C-Freq", "numberTag") => Some(16),
            ("TGID", "numberTag") => Some(14),
            _ => None,
        }
    }

    fn channel_value_options(&self, field: &str) -> Vec<String> {
        // The enumerated per-channel value options (Uniden File Spec V2.00). Stored verbatim
        // as these strings in the C-Freq/TGID record, so the value **is** the label; a UI
        // supplies only presentation (the color swatch, a "+"/"s" suffix). The single source
        // of truth for these menus. Free-form/numeric fields (numberTag) and the boolean
        // `attenuator` have no enumeration and return empty.
        let owned = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        match field {
            "modulation" => owned(&["AUTO", "AM", "NFM", "FM"]),
            "audioType" => owned(&["ALL", "ANALOG", "DIGITAL"]),
            "delay" => owned(&["30", "10", "5", "4", "3", "2", "1", "0", "-5", "-10"]),
            "volumeOffset" => owned(&["-3", "-2", "-1", "0", "1", "2", "3"]),
            "alertColor" => owned(&[
                "Off", "Blue", "Red", "Magenta", "Green", "Cyan", "Yellow", "White",
            ]),
            "alertPattern" => owned(&["On", "Slow Blink", "Fast Blink"]),
            "alertTone" => std::iter::once("Off".to_string())
                .chain((1..=9).map(|n| n.to_string()))
                .collect(),
            "alertVolume" => std::iter::once("Auto".to_string())
                .chain((1..=15).map(|n| n.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn favorites_field_defaults(&self, command: &str) -> &'static [(usize, &'static str)] {
        // Columns the favorites dialect fills that the HPDB source leaves blank.
        // Derived 2026-06-29 by diffing a known-good favorites (the owner's working
        // SAFE-T) against the HPDB source. `T-Freq` col 4 = "Off" is the one that
        // makes a P25 trunk lock its control channel.
        match command {
            "Conventional" => &[
                (7, "Off"),
                (8, "Off"),
                (9, "0"),
                (10, "Off"),
                (11, "Off"),
                (12, "400"),
                (13, "Auto"),
                (14, "8"),
            ],
            "C-Freq" => &[
                (9, "Off"),
                (10, "2"),
                (11, "0"),
                (12, "Off"),
                (13, "Auto"),
                (14, "Off"),
                (15, "On"),
                (16, "Off"),
                (17, "Off"),
            ],
            "Trunk" => &[
                (7, "Off"),
                (8, "Off"),
                (9, "Auto"),
                (10, "Ignore"),
                (11, "Srch"),
                (12, "Off"),
                (13, "Off"),
                (14, "0"),
                (15, "Off"),
                (16, "Off"),
                (17, "Ignore"),
                (18, "Off"),
                (19, "Off"),
                (20, "On"),
                (21, "IDAS"), // constant default, overrides the source trunk subtype
            ],
            "Site" => &[
                (12, "Off"),
                (13, "400"),
                (14, "Auto"),
                (15, "8"),
                (16, "Off"),
            ],
            // col 4 = Reserve(Avoid), always "Off" (Uniden File Spec V2.00 — universal).
            // col 6 = LCN (spec-confirmed): SAFE-T working files zero it (each freq
            // becomes a control-channel candidate), and forcing 0 let SAFE-T decode.
            // HEURISTIC, not universal — other working systems (e.g. Honolulu) keep
            // non-zero voice LCNs. Revisit if a system that uses explicit LCNs fails to
            // lock (ideally preserve the source LCN instead of forcing 0).
            "T-Freq" => &[(4, "Off"), (6, "0")],
            "T-Group" => &[(9, "Off")],
            "TGID" => &[
                (8, "2"),
                (9, "0"),
                (10, "Off"),
                (11, "Auto"),
                (12, "Off"),
                (13, "On"),
                (14, "Off"),
                (15, "Off"),
            ],
            _ => &[],
        }
    }

    fn f_list_entry(&self, label: &str, filename: &str) -> Option<Vec<String>> {
        // F-List record (Uniden File Spec V2.00; see docs/radios/sds150.md):
        //   0 F-List · 1 UserName · 2 Filename · 3 LocationControl · 4 Monitor ·
        //   5 Quick key · 6 NumberTag · 7-16 Startup key 0-9 · 17-116 S-Qkey_00-99.
        // Used only for a BRAND-NEW list — existing lists are preserved verbatim.
        // A fresh list: LocationControl Off, Monitor On, Quick key 0, rest Off
        // (matches the "Home" entry on a real card). Total 118 fields.
        let mut fields = vec![
            "F-List".to_string(),
            label.to_string(),
            filename.to_string(),
            "Off".to_string(), // LocationControl
            "On".to_string(),  // Monitor
            "0".to_string(),   // Quick key
        ];
        fields.resize(F_LIST_FIELDS, "Off".to_string());
        Some(fields)
    }

    fn f_list_monitor_field(&self) -> Option<usize> {
        Some(4) // Monitor (On/Off) — Uniden File Spec V2.00 F-List field 4.
    }

    fn f_list_quick_key_field(&self) -> Option<usize> {
        Some(5) // Quick key (Off/0-99) — Uniden File Spec V2.00 F-List field 5.
    }

    fn f_list_number_tag_field(&self) -> Option<usize> {
        Some(6) // NumberTag (Off/0-99) — Uniden File Spec V2.00 F-List field 6.
    }
}

/// Field count of an `F-List` record.
const F_LIST_FIELDS: usize = 118;

/// Number of department-quick-key slots in a `DQKs_Status` line.
const DQKS_SLOTS: usize = 100;

/// The standard P25 700/800/900 MHz band plan, exactly as a real SDS150 favorites
/// file emits it (fields after the `BandPlan_P25` command). Identical across every
/// P25 site observed, so it is treated as a constant. Captured verbatim from a real
/// favorites file. The leading field is the (blank) MyId column.
const STANDARD_P25_BANDPLAN: &[&str] = &[
    "",
    "851006250",
    "6250",
    "762006250",
    "6250",
    "851012500",
    "12500",
    "762006250",
    "12500",
    "935012500",
    "12500",
    "935012500",
    "12500",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "3",
    "3",
    "0",
    "3",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
    "0",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::FileHeader;

    #[test]
    fn detects_real_header() {
        let p = Sds150::new();
        let h = FileHeader {
            target_model: Some("BCDx36HP".into()),
            format_version: Some("1.00".into()),
        };
        assert!(p.matches(&h));
    }

    #[test]
    fn rejects_other_model() {
        let p = Sds150::new();
        let h = FileHeader {
            target_model: Some("BCDx00".into()),
            format_version: Some("1.00".into()),
        };
        assert!(!p.matches(&h));
    }

    #[test]
    fn site_blanks_id_columns_in_favorites() {
        let p = Sds150::new();
        let site = p.record_schema("Site").unwrap();
        assert_eq!(site.favorites.blanked_id_columns, &[1, 2]);
        assert_eq!(site.field_count, 20);
    }

    #[test]
    fn layout_uses_misspelled_discovery() {
        assert_eq!(Sds150::new().sd_layout().discovery_cfg, "discvery.cfg");
    }

    #[test]
    fn channel_value_options_enumerate_alerts() {
        let p = Sds150::new();
        // Alert color / pattern are fixed lists; the value is the stored string.
        assert_eq!(p.channel_value_options("alertColor")[0], "Off");
        assert!(p
            .channel_value_options("alertColor")
            .contains(&"Magenta".to_string()));
        assert_eq!(
            p.channel_value_options("alertPattern"),
            ["On", "Slow Blink", "Fast Blink"]
        );
        // Tone / volume are ranges with a leading sentinel.
        let tone = p.channel_value_options("alertTone");
        assert_eq!(tone.first().unwrap(), "Off");
        assert_eq!(tone.last().unwrap(), "9");
        assert_eq!(tone.len(), 10);
        let vol = p.channel_value_options("alertVolume");
        assert_eq!(vol.first().unwrap(), "Auto");
        assert_eq!(vol.last().unwrap(), "15");
        // Scan-setting menus are enumerated too.
        assert_eq!(
            p.channel_value_options("modulation"),
            ["AUTO", "AM", "NFM", "FM"]
        );
        assert_eq!(
            p.channel_value_options("audioType"),
            ["ALL", "ANALOG", "DIGITAL"]
        );
        // Every enumerated field is one this model exposes as an editable value.
        for field in [
            "modulation",
            "audioType",
            "delay",
            "volumeOffset",
            "alertColor",
            "alertPattern",
            "alertTone",
            "alertVolume",
        ] {
            assert!(p.channel_value_fields().contains(&field));
            assert!(!p.channel_value_options(field).is_empty());
        }
        // Free-form / numeric fields and the boolean toggle have no enumeration.
        assert!(p.channel_value_options("numberTag").is_empty());
        assert!(p.channel_value_options("attenuator").is_empty());
        assert!(p.channel_value_options("nonsense").is_empty());
    }
}
