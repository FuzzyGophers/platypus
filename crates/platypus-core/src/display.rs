// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! SDS150 **display customization** — the third card file class (after the HPDB browse DB and the
//! favorites lists). The scanner's on-screen theme lives in `profile.cfg` (already named by
//! [`crate::device::profile::SdLayout::profile_cfg`]): which data items show in each screen area,
//! and the per-element text/background colors.
//!
//! `profile.cfg` is a rich, multi-record settings file (it also holds owner info, band defaults,
//! tone-out, GPS, …). We touch **only** the four display records — `DisplayOption`, `Backlight`,
//! `DispOptItems`, `DispColors` — and carry every other record, and every field we don't own,
//! **verbatim**. Combined with the byte-exact round-trip gate this is the "never overwrite what we
//! don't know" rule; it's also what keeps the owner's real data (never referenced here) untouched.
//!
//! Records (tab-separated, spec-derived — see `docs/radios/sds150-display.md`, confirmed against a
//! real SDS150 card):
//! - `DisplayOption` — positional globals; only a few columns carry values (rest Reserve).
//! - `Backlight` — positional globals (SDS150 populates brightness/dimmer fields too).
//! - `DispOptItems  DispOptId=N  DispLayoutId=M  tok1 … tokK` — the ordered item tokens per area.
//! - `DispColors  DispColorId=N  ColorLayoutId=M  text1 back1 …` — the ordered text/back color
//!   pairs per element group. Order is fixed by the two ids; counts vary by layout mode.

use crate::format::Document;
use crate::model::keyed_id;
use crate::Result;

/// A global display setting bound to a fixed column of a `DisplayOption`/`Backlight` record.
/// Only columns whose meaning is confirmed appear here; every other (Reserve/device-managed)
/// column is preserved verbatim.
pub struct GlobalSpec {
    pub command: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub col: usize,
    pub options: &'static [&'static str],
}

/// The editable global display settings. All but one column are pinned unambiguously by their
/// value domain on a real card (col 12=`AFS`→EDACS format, col 6=`DEC`→Motorola format,
/// col 13=`COLOR`→Color mode; Backlight col 10→Squelch light, col 11→Key light). The one
/// exception is DisplayOption col 11 (see its note) — an inference, not yet hardware-confirmed.
/// Every other (Reserve/device-managed) column is preserved verbatim.
pub const GLOBAL_SPECS: &[GlobalSpec] = &[
    GlobalSpec {
        command: "DisplayOption",
        key: "MotTgidFormat",
        label: "Motorola TGID format",
        col: 6,
        options: &["DEC", "HEX"],
    },
    // INFERRED (not hardware-confirmed): col 11 carries an Off/On value; "Simple mode" is
    // consistent with a real card (Simple Mode On ↔ col 11 `On`) but could be Upside_down or
    // another Off/On field. A write only ever changes this one column, so a mislabel is
    // non-destructive — confirm via a hardware round-trip before trusting the label.
    GlobalSpec {
        command: "DisplayOption",
        key: "SimpleMode",
        label: "Simple mode",
        col: 11,
        options: &["Off", "On"],
    },
    GlobalSpec {
        command: "DisplayOption",
        key: "EdacTgidFormat",
        label: "EDACS TGID format",
        col: 12,
        options: &["AFS", "DEC"],
    },
    GlobalSpec {
        command: "DisplayOption",
        key: "ColorMode",
        label: "Color mode",
        col: 13,
        options: &["COLOR", "BLACK", "WHITE"],
    },
    GlobalSpec {
        command: "Backlight",
        key: "SqLight",
        label: "Squelch light",
        col: 10,
        options: &["Off", "5", "10", "15", "OpenSquelch"],
    },
    GlobalSpec {
        command: "Backlight",
        key: "KeyLight",
        label: "Key light",
        col: 11,
        options: &["15", "30", "60", "120", "Infinite"],
    },
];

/// A display layout mode. The two ids differ between the item and color records (per the spec).
pub struct LayoutMode {
    pub name: &'static str,
    pub disp_layout_id: u64,
    pub color_layout_id: u64,
}

/// The seven display modes (`DispLayoutId` / `ColorLayoutId`).
pub const LAYOUT_MODES: &[LayoutMode] = &[
    LayoutMode {
        name: "Simple Conventional",
        disp_layout_id: 1,
        color_layout_id: 1,
    },
    LayoutMode {
        name: "Simple Trunk",
        disp_layout_id: 2,
        color_layout_id: 6,
    },
    LayoutMode {
        name: "Detail Conventional",
        disp_layout_id: 3,
        color_layout_id: 2,
    },
    LayoutMode {
        name: "Detail Trunk",
        disp_layout_id: 4,
        color_layout_id: 7,
    },
    LayoutMode {
        name: "Search / Close Call",
        disp_layout_id: 5,
        color_layout_id: 3,
    },
    LayoutMode {
        name: "Weather",
        disp_layout_id: 6,
        color_layout_id: 4,
    },
    LayoutMode {
        name: "Tone Out",
        disp_layout_id: 7,
        color_layout_id: 5,
    },
];

/// The allowed item (`DispOptItems`) File tokens for one screen area (`DispOptId`).
pub struct ItemArea {
    pub disp_opt_id: u64,
    pub label: &'static str,
    pub tokens: &'static [&'static str],
}

/// Item vocabularies by option area. Areas 1-3 from the spec; area 4 shares the small-area
/// vocabulary (confirmed: real cards carry data items there, not the icons the spec table implies).
pub const ITEM_AREAS: &[ItemArea] = &[
    ItemArea {
        disp_opt_id: 1,
        label: "Huge",
        tokens: &[
            "Empty",
            "CTCSS/DCS",
            "FL_Name",
            "Frequency",
            "NumberTag",
            "SysSubID",
            "ServiceType",
            "SiteId",
            "SiteName",
            "SystemType",
            "SystemId",
            "TGID",
            "UnitId",
            "UnitIdName",
            "Volume&Squelch",
            "WACN",
        ],
    },
    ItemArea {
        disp_opt_id: 2,
        label: "Large",
        tokens: &[
            "Empty",
            "BattVoltage",
            "CTCSS/DCS",
            "D_ErrorCount",
            "Filter",
            "FL_Name",
            "Frequency",
            "latitude",
            "Lcn",
            "longitude",
            "Noise",
            "NumberTag",
            "SysSubID",
            "Rssi",
            "Rssi Bar",
            "ServiceType",
            "SiteId",
            "SiteName",
            "SystemType",
            "SystemId",
            "TdmaSlot",
            "TGID",
            "UnitId",
            "UnitIdName",
            "USB1_vbus",
            "USB2_vbus",
            "Volume&Squelch",
            "WACN",
            "Bluetooth",
            "Battery Current",
            "Battery Temperature",
        ],
    },
    ItemArea {
        disp_opt_id: 3,
        label: "Small",
        tokens: SMALL_AREA_ITEMS,
    },
    ItemArea {
        disp_opt_id: 4,
        label: "Small (lower)",
        tokens: SMALL_AREA_ITEMS,
    },
];

const SMALL_AREA_ITEMS: &[&str] = &[
    "Empty",
    "ATT",
    "SCR",
    "CC",
    "Day",
    "P25Status",
    "GPS",
    "IFX",
    "Modulation",
    "P_Ch",
    "PRI",
    "REC",
    "REP",
    "Squelch",
    "TdmaSlot",
    "Time",
    "Volume",
    "LVL",
    "WxPRI",
    "Bluetooth",
];

/// The element groups a `DispColors` row paints (`DispColorId`). The element list is nominal;
/// the actual pair count per row varies by layout mode.
pub struct ColorGroupInfo {
    pub disp_color_id: u64,
    pub elements: &'static [&'static str],
}

/// Nominal element names per color group (spec `DispColors` table).
pub const COLOR_GROUPS: &[ColorGroupInfo] = &[
    ColorGroupInfo {
        disp_color_id: 1,
        elements: &[
            "System Name",
            "System Avoid",
            "Dept Name",
            "Dept Avoid",
            "Channel Name",
            "Channel Avoid",
        ],
    },
    ColorGroupInfo {
        disp_color_id: 2,
        elements: &["System Option", "Dept Option", "Channel Option"],
    },
    ColorGroupInfo {
        disp_color_id: 3,
        elements: &["Option_1", "Option_2", "Option_3", "Option_4", "Option_5"],
    },
    ColorGroupInfo {
        disp_color_id: 4,
        elements: &["Option A_1", "Option_B_1"],
    },
    ColorGroupInfo {
        disp_color_id: 5,
        elements: &["ICON1", "ICON2", "ICON3", "ICON4", "ICON5"],
    },
    ColorGroupInfo {
        disp_color_id: 6,
        elements: &["F", "SIG", "BAT", "SP0", "KEY"],
    },
    ColorGroupInfo {
        disp_color_id: 7,
        elements: &["Soft Key 1", "SP1", "Soft Key 2", "SP2", "Soft Key 3"],
    },
];

/// The allowed color palette: X11-style names with Uniden's own hex values (a `DispColors` field
/// is one 6-hex value). 147 colors, spec `Color, Item code` sheet.
pub const COLOR_PALETTE: &[(&str, &str)] = &[
    ("Aliceblue", "eff7ff"),
    ("Antiquewhite", "f7ebd6"),
    ("Aqua", "00fbf7"),
    ("Aquamarine", "7bffce"),
    ("Azure", "efffff"),
    ("Beige", "eff3d6"),
    ("Bisque", "ffe3bd"),
    ("Black", "000000"),
    ("Blanchedalmond", "ffebc6"),
    ("Blue", "0000ff"),
    ("Blueviolet", "8429de"),
    ("Brass", "b5a542"),
    ("Brown", "a52929"),
    ("Burlywood", "d6b584"),
    ("Cadetblue", "5a9c9c"),
    ("Chartreuse", "7bff00"),
    ("Chocolate", "ce6718"),
    ("Coolcopper", "d68418"),
    ("Copper", "bd00de"),
    ("Coral", "ff7f4a"),
    ("Cornflower", "bdefde"),
    ("Cornflowerblue", "6390e7"),
    ("Cornsilk", "fff7d6"),
    ("Crimson", "d61039"),
    ("Cyan", "00ffff"),
    ("Darkblue", "000084"),
    ("Darkbrown", "d60800"),
    ("Darkcyan", "008884"),
    ("Darkgoldenrod", "b58408"),
    ("Darkgray", "a5a5a5"),
    ("Darkgreen", "006300"),
    ("Darkkhaki", "b5b56b"),
    ("Darkmagenta", "840084"),
    ("Darkolivegreen", "526b29"),
    ("Darkorange", "ff8800"),
    ("Darkorchid", "9431c6"),
    ("Darkred", "840000"),
    ("Darksalmon", "e79473"),
    ("Darkseagreen", "8cb98c"),
    ("Darkslateblue", "423d84"),
    ("Darkslategray", "294e4a"),
    ("Darkturquoise", "00cace"),
    ("Darkviolet", "8c00ce"),
    ("Deeppink", "ff108c"),
    ("Deepskyblue", "00bdff"),
    ("Dimgray", "636763"),
    ("Dodgerblue", "188cff"),
    ("Feldspar", "f7cede"),
    ("Firebrick", "ad2121"),
    ("Floralwhite", "fff7ef"),
    ("Forestgreen", "218821"),
    ("Fuchsia", "f700f7"),
    ("Gainsboro", "d6dad6"),
    ("Ghostwhite", "f7f7ff"),
    ("Gold", "ffd600"),
    ("Goldenrod", "d6a118"),
    ("Gray", "7b7f7b"),
    ("Green", "007f00"),
    ("Greenyellow", "adff29"),
    ("Honeydew", "efffef"),
    ("Hotpink", "ff67ad"),
    ("Indianred", "c65a5a"),
    ("Indigo", "4a007b"),
    ("Ivory", "ffffef"),
    ("Khaki", "efe38c"),
    ("Lavender", "dee3f7"),
    ("Lavenderblush", "ffefef"),
    ("Lawngreen", "7bfb00"),
    ("Lemonchiffon", "fff7c6"),
    ("Lightblue", "add6de"),
    ("Lightcoral", "ef7f7b"),
    ("Lightcyan", "deffff"),
    ("Lightgoldenrodyellow", "f7f7ce"),
    ("Lightgreen", "8ceb8c"),
    ("Lightgray", "ced2ce"),
    ("Lightpink", "ffb1bd"),
    ("Lightsalmon", "ff9c73"),
    ("Lightseagreen", "18ada5"),
    ("Lightskyblue", "84caf7"),
    ("Lightslategray", "738494"),
    ("Lightsteelblue", "adc2d6"),
    ("Lightyellow", "ffffde"),
    ("Lime", "00ff00"),
    ("Limegreen", "31ca31"),
    ("Linen", "f7efde"),
    ("Magenta", "ff00ff"),
    ("Maroon", "7b0000"),
    ("Mediumaquamarine", "63caa5"),
    ("Mediumblue", "0000c6"),
    ("Mediumorchid", "b556ce"),
    ("Mediumpurple", "8c6fd6"),
    ("Mediumseagreen", "39b16b"),
    ("Mediumslateblue", "7367e7"),
    ("Mediumspringgreen", "00f794"),
    ("Mediumturquoise", "42cec6"),
    ("Mediumvioletred", "c61484"),
    ("Midnightblue", "18186b"),
    ("Mintcream", "effff7"),
    ("Mistyrose", "ffe3de"),
    ("Moccasin", "ffe3b5"),
    ("Navajowhite", "ffdaad"),
    ("Navy", "00007b"),
    ("Oldlace", "f7f3de"),
    ("Olive", "7b7f00"),
    ("Olivered", "6b8c21"),
    ("Orange", "ffa100"),
    ("Orangered", "ff4600"),
    ("Orchid", "d66fd6"),
    ("Palegoldenrod", "e7e7a5"),
    ("Palegreen", "94fb94"),
    ("Paleturquoise", "adebe7"),
    ("Palevioletred", "d66f8c"),
    ("Papayawhip", "ffefce"),
    ("Peachpuff", "ffd6b5"),
    ("Peru", "c68039"),
    ("Pink", "ffbdc6"),
    ("Plum", "d69cd6"),
    ("Powderblue", "addede"),
    ("Purple", "7b007b"),
    ("Red", "ff0000"),
    ("Richblue", "08adde"),
    ("Rosybrown", "b58c8c"),
    ("Royalblue", "3967de"),
    ("Saddlebrown", "844610"),
    ("Salmon", "f77f6b"),
    ("Sandybrown", "efa15a"),
    ("Seagreen", "298852"),
    ("Seashell", "fff3e7"),
    ("Sienna", "9c5229"),
    ("Silver", "bdbdbd"),
    ("Skyblue", "84cae7"),
    ("Slateblue", "635ac6"),
    ("Slategray", "6b7f8c"),
    ("Snow", "fff7f7"),
    ("Springgreen", "00ff7b"),
    ("Steelblue", "4280ad"),
    ("Tan", "ceb18c"),
    ("Teal", "007f7b"),
    ("Thistle", "d6bdd6"),
    ("Tomato", "ff6342"),
    ("Turquoise", "39dece"),
    ("Violet", "e780e7"),
    ("Wheat", "efdaad"),
    ("White", "ffffff"),
    ("Whitesmoke", "eff3ef"),
    ("Yellow", "ffff00"),
    ("Yellowgreen", "94ca31"),
];

/// A read view of one global display setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalSetting {
    pub key: String,
    pub label: String,
    pub value: String,
    pub options: Vec<String>,
}

/// The ordered item tokens for one `(DispOptId, DispLayoutId)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemGroup {
    pub disp_opt_id: u64,
    pub disp_layout_id: u64,
    pub tokens: Vec<String>,
}

/// One text/background color pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorPair {
    pub text: String,
    pub back: String,
}

/// The ordered color pairs for one `(DispColorId, ColorLayoutId)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorGroup {
    pub disp_color_id: u64,
    pub color_layout_id: u64,
    pub pairs: Vec<ColorPair>,
}

/// The parsed display customization of a `profile.cfg`. Wraps the whole document so every
/// non-display record round-trips; edits touch only the targeted display field.
#[derive(Debug, Clone)]
pub struct DisplayConfig {
    doc: Document,
}

impl DisplayConfig {
    /// Parse a whole `profile.cfg`.
    pub fn parse(raw: &[u8]) -> Result<Self> {
        Ok(Self {
            doc: Document::parse(raw)?,
        })
    }

    /// Re-encode. `parse(raw).to_bytes() == raw` for an unedited config.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.doc.to_bytes()
    }

    // --- Global settings (DisplayOption / Backlight) ---

    /// The identified global settings and their current values.
    pub fn globals(&self) -> Vec<GlobalSetting> {
        GLOBAL_SPECS
            .iter()
            .filter_map(|spec| {
                let line = self
                    .doc
                    .lines
                    .iter()
                    .find(|l| l.command() == spec.command)?;
                let value = line.field(spec.col).unwrap_or("").to_owned();
                Some(GlobalSetting {
                    key: spec.key.to_owned(),
                    label: spec.label.to_owned(),
                    value,
                    options: spec.options.iter().map(|s| s.to_string()).collect(),
                })
            })
            .collect()
    }

    /// Set a global setting by key. Returns `true` if a field actually changed (change-gated, so a
    /// no-op set re-encodes byte-for-byte identically).
    pub fn set_global(&mut self, key: &str, value: &str) -> bool {
        let Some(spec) = GLOBAL_SPECS.iter().find(|s| s.key == key) else {
            return false;
        };
        let Some(li) = self
            .doc
            .lines
            .iter()
            .position(|l| l.command() == spec.command)
        else {
            return false;
        };
        let fields = &mut self.doc.lines[li].fields;
        if spec.col >= fields.len() || fields[spec.col] == value {
            return false;
        }
        fields[spec.col] = value.to_owned();
        true
    }

    // --- Item assignments (DispOptItems) ---

    /// Every `DispOptItems` group, in file order.
    pub fn items(&self) -> Vec<ItemGroup> {
        self.doc
            .lines
            .iter()
            .filter(|l| l.command() == "DispOptItems")
            .filter_map(|l| {
                Some(ItemGroup {
                    disp_opt_id: keyed_id(l.field(1)?)?,
                    disp_layout_id: keyed_id(l.field(2)?)?,
                    tokens: l.fields[3.min(l.fields.len())..].to_vec(),
                })
            })
            .collect()
    }

    /// Set the item token at `index` within a `(DispOptId, DispLayoutId)` group. Item order/count
    /// is fixed by the ids, so this only assigns within the existing range. Returns `true` if it
    /// changed a field.
    pub fn set_item_token(
        &mut self,
        disp_opt_id: u64,
        disp_layout_id: u64,
        index: usize,
        token: &str,
    ) -> bool {
        let Some(li) = self.find_pair("DispOptItems", disp_opt_id, disp_layout_id) else {
            return false;
        };
        let col = 3 + index;
        let fields = &mut self.doc.lines[li].fields;
        if col >= fields.len() || fields[col] == token {
            return false;
        }
        fields[col] = token.to_owned();
        true
    }

    // --- Colors (DispColors) ---

    /// Every `DispColors` group, in file order.
    pub fn colors(&self) -> Vec<ColorGroup> {
        self.doc
            .lines
            .iter()
            .filter(|l| l.command() == "DispColors")
            .filter_map(|l| {
                let rest = &l.fields[3.min(l.fields.len())..];
                let pairs = rest
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| ColorPair {
                        text: c[0].clone(),
                        back: c[1].clone(),
                    })
                    .collect();
                Some(ColorGroup {
                    disp_color_id: keyed_id(l.field(1)?)?,
                    color_layout_id: keyed_id(l.field(2)?)?,
                    pairs,
                })
            })
            .collect()
    }

    /// Set the text+back colors of pair `index` within a `(DispColorId, ColorLayoutId)` group.
    /// Colors are 6-hex strings from [`COLOR_PALETTE`]. Returns `true` if a field changed.
    pub fn set_color(
        &mut self,
        disp_color_id: u64,
        color_layout_id: u64,
        index: usize,
        text: &str,
        back: &str,
    ) -> bool {
        let Some(li) = self.find_pair("DispColors", disp_color_id, color_layout_id) else {
            return false;
        };
        let (tc, bc) = (3 + index * 2, 3 + index * 2 + 1);
        let fields = &mut self.doc.lines[li].fields;
        if bc >= fields.len() {
            return false;
        }
        let mut changed = false;
        if fields[tc] != text {
            fields[tc] = text.to_owned();
            changed = true;
        }
        if fields[bc] != back {
            fields[bc] = back.to_owned();
            changed = true;
        }
        changed
    }

    /// Index of the line for a record with the two keyed ids in fields 1 and 2.
    fn find_pair(&self, command: &str, id1: u64, id2: u64) -> Option<usize> {
        self.doc.lines.iter().position(|l| {
            l.command() == command
                && l.field(1).and_then(keyed_id) == Some(id1)
                && l.field(2).and_then(keyed_id) == Some(id2)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A synthetic profile.cfg fragment: the four display records + a non-display record, to prove
    // preservation. No owner/location data (privacy). Tab-separated, CRLF, no final newline.
    const SAMPLE: &[u8] = b"TargetModel\tBCDx36HP\r\n\
FormatVersion\t2.00\r\n\
BandDefault\t1\t25000000\t54000000\tAM\r\n\
DisplayOption\t\t\t\t\t\tDEC\t\t\t\t\tOn\tAFS\tCOLOR\r\n\
Backlight\t\tHigh\t\t\t30\t40\tOff\tOff\tOn\t5\tInfinite\r\n\
DispOptItems\tDispOptId=1\tDispLayoutId=1\tFL_Name\tEmpty\tFrequency\r\n\
DispOptItems\tDispOptId=3\tDispLayoutId=1\tATT\tBluetooth\tDay\r\n\
DispColors\tDispColorId=1\tColorLayoutId=1\tff4600\t000000\tff8800\t000000\r\n\
DispColors\tDispColorId=4\tColorLayoutId=1\te79473\t000000";

    #[test]
    fn palette_has_147_colors() {
        assert_eq!(COLOR_PALETTE.len(), 147);
        assert_eq!(
            COLOR_PALETTE
                .iter()
                .find(|(n, _)| *n == "Aqua")
                .map(|(_, h)| *h),
            Some("00fbf7")
        );
        // every hex is 6 lowercase hex digits
        for (_, hex) in COLOR_PALETTE {
            assert_eq!(hex.len(), 6);
            assert!(hex
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        }
    }

    #[test]
    fn round_trips_byte_for_byte() {
        let cfg = DisplayConfig::parse(SAMPLE).unwrap();
        assert_eq!(
            cfg.to_bytes(),
            SAMPLE,
            "unedited profile.cfg must round-trip"
        );
    }

    #[test]
    fn reads_globals() {
        let cfg = DisplayConfig::parse(SAMPLE).unwrap();
        let g = cfg.globals();
        let get = |k: &str| g.iter().find(|s| s.key == k).map(|s| s.value.as_str());
        assert_eq!(get("MotTgidFormat"), Some("DEC"));
        assert_eq!(get("SimpleMode"), Some("On"));
        assert_eq!(get("EdacTgidFormat"), Some("AFS"));
        assert_eq!(get("ColorMode"), Some("COLOR"));
        assert_eq!(get("SqLight"), Some("5"));
        assert_eq!(get("KeyLight"), Some("Infinite"));
    }

    #[test]
    fn reads_items_and_colors() {
        let cfg = DisplayConfig::parse(SAMPLE).unwrap();
        let items = cfg.items();
        assert_eq!(items[0].disp_opt_id, 1);
        assert_eq!(items[0].tokens, vec!["FL_Name", "Empty", "Frequency"]);
        let colors = cfg.colors();
        assert_eq!(colors[0].disp_color_id, 1);
        assert_eq!(colors[0].pairs.len(), 2);
        assert_eq!(
            colors[0].pairs[0],
            ColorPair {
                text: "ff4600".into(),
                back: "000000".into()
            }
        );
    }

    #[test]
    fn edits_touch_only_the_target_field() {
        let mut cfg = DisplayConfig::parse(SAMPLE).unwrap();
        assert!(cfg.set_global("ColorMode", "BLACK"));
        assert!(cfg.set_item_token(1, 1, 1, "TGID")); // "Empty" -> "TGID"
        assert!(cfg.set_color(1, 1, 0, "ffffff", "000000")); // text ff4600 -> ffffff

        let out = String::from_utf8(cfg.to_bytes()).unwrap();
        assert!(out.contains("\tOn\tAFS\tBLACK\r\n"));
        assert!(out.contains("DispOptId=1\tDispLayoutId=1\tFL_Name\tTGID\tFrequency\r\n"));
        assert!(out.contains("DispColorId=1\tColorLayoutId=1\tffffff\t000000\tff8800\t000000\r\n"));
        // untouched records preserved verbatim
        assert!(out.contains("BandDefault\t1\t25000000\t54000000\tAM\r\n"));
        assert!(out.contains("DispOptItems\tDispOptId=3\tDispLayoutId=1\tATT\tBluetooth\tDay\r\n"));
    }

    #[test]
    fn no_op_edits_are_byte_identical() {
        let mut cfg = DisplayConfig::parse(SAMPLE).unwrap();
        assert!(!cfg.set_global("ColorMode", "COLOR")); // already COLOR
        assert!(!cfg.set_item_token(1, 1, 0, "FL_Name")); // already FL_Name
        assert!(!cfg.set_color(1, 1, 0, "ff4600", "000000")); // unchanged
        assert!(!cfg.set_global("Nonexistent", "x"));
        assert!(!cfg.set_item_token(9, 9, 0, "x")); // missing group
        assert_eq!(cfg.to_bytes(), SAMPLE);
    }
}
