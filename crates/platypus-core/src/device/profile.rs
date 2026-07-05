// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! The [`SdCardProfile`] trait + registry. Model-agnostic definitions; concrete
//! profiles (e.g. [`super::sds150::Sds150`]) live in their own modules.

use super::ft60::CloneSpec;
use crate::format::FileHeader;

/// Identifies a file-format dialect by the two header sentences every card file
/// carries. A profile claims one (or more) of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelKey {
    /// `TargetModel`, e.g. `"BCDx36HP"`. Note this is a *family* tag — several
    /// physical models can share it (the SDS150 reports `BCDx36HP`).
    pub target_model: String,
    /// `FormatVersion`, e.g. `"1.00"` — the *file format* version, distinct from
    /// the spec-document version (V1.03).
    pub format_version: String,
}

impl ModelKey {
    pub fn new(target_model: impl Into<String>, format_version: impl Into<String>) -> Self {
        Self {
            target_model: target_model.into(),
            format_version: format_version.into(),
        }
    }
}

/// Names of the folders and files a model lays down on the SD card. Encoded as
/// data (not hard-coded paths) precisely so other models can differ — and so the
/// real-world `discvery.cfg` misspelling and the must-delete `app_data.cfg` are
/// captured in exactly one auditable place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdLayout {
    /// Top-level model folder, e.g. `"BCDx36HP"`.
    pub model_folder: &'static str,
    pub favorites_dir: &'static str,
    pub favorites_list_cfg: &'static str,
    pub hpdb_dir: &'static str,
    pub hpdb_cfg: &'static str,
    pub profile_cfg: &'static str,
    /// Misspelled on real cards (`discvery.cfg`, no second "o"). Stored verbatim.
    pub discovery_cfg: &'static str,
    /// Resume-state file. Per the spec's CRITICAL RULE, any writer that changes
    /// program data on the card **must delete this file**.
    pub app_data_cfg: &'static str,
}

/// How the leaner *favorites* dialect differs from the full HPDB dialect for a
/// given record. On real cards the favorites file keeps the same column count but
/// **blanks** the identity columns rather than dropping them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FavoritesDialect {
    /// Column indices (into a line's `fields`) that hold MyId/ParentId and are
    /// left empty in a favorites file. Empty slice = record unchanged / absent.
    pub blanked_id_columns: &'static [usize],
}

impl FavoritesDialect {
    /// Record absent from / unchanged in favorites files.
    pub const NONE: Self = Self {
        blanked_id_columns: &[],
    };
    /// Hierarchical record: favorites blank MyId@1 and ParentId@2.
    pub const HIERARCHICAL: Self = Self {
        blanked_id_columns: &[1, 2],
    };
}

/// Column indices for a record's geo fields. Encoded per-model here because the
/// layout differs even within one model (e.g. `Site` shape@11 vs `T-Group`
/// shape@8). All indices verified against a real card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeoColumns {
    pub lat: usize,
    pub lon: usize,
    pub range: usize,
    pub shape: usize,
}

/// Column indices for a channel record's filterable attributes (a frequency or a
/// talkgroup). All optional and per-model; verified against a real card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelColumns {
    /// Frequency in Hz (conventional / control-channel records).
    pub freq: Option<usize>,
    /// Talkgroup id (trunked talkgroups).
    pub tgid: Option<usize>,
    /// Modulation / audio type (`NFM`/`AM`/`FM`, or `ALL`/`ANALOG`/`DIGITAL`).
    pub mode: Option<usize>,
    /// Squelch tone (`TONE=C156.7` etc.).
    pub tone: Option<usize>,
    /// RadioReference service-type code (numeric).
    pub service_type: Option<usize>,
}

/// Schema for one record/command type: observed field count, the semantic column
/// map (id/parent/name/geo/channel/tech), and the favorites dialect rule. Field
/// counts and column indices come from real-card recon. The byte-exact round trip
/// never depends on any of this — it's the bridge from raw lines to a typed model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSchema {
    pub command: &'static str,
    /// Number of tab fields observed for this record on a real card.
    pub field_count: u16,
    /// Column holding this record's own id (e.g. `SiteId=…`), if any.
    pub id_col: Option<usize>,
    /// Column holding the parent id (e.g. `TrunkId=…`), if any.
    pub parent_col: Option<usize>,
    /// Column holding the human-readable name, if any.
    pub name_col: Option<usize>,
    /// Geo column map, for records that carry a location.
    pub geo: Option<GeoColumns>,
    /// Channel attribute columns (freq/tgid/mode/tone/service-type), for
    /// frequency/talkgroup records.
    pub channel: Option<ChannelColumns>,
    /// Column holding the system technology (`P25Standard`, `MotoTrbo`, …) on a
    /// `Conventional`/`Trunk` header line.
    pub tech_col: Option<usize>,
    pub favorites: FavoritesDialect,
}

impl RecordSchema {
    /// Header / global record: no identity, name, or geo columns.
    pub const fn header(command: &'static str, field_count: u16) -> Self {
        Self {
            command,
            field_count,
            id_col: None,
            parent_col: None,
            name_col: None,
            geo: None,
            channel: None,
            tech_col: None,
            favorites: FavoritesDialect::NONE,
        }
    }

    /// Hierarchical record present in favorites (MyId@1 / ParentId@2 blanked there).
    pub const fn hier(
        command: &'static str,
        field_count: u16,
        name_col: Option<usize>,
        geo: Option<GeoColumns>,
    ) -> Self {
        Self {
            command,
            field_count,
            id_col: Some(1),
            parent_col: Some(2),
            name_col,
            geo,
            channel: None,
            tech_col: None,
            favorites: FavoritesDialect::HIERARCHICAL,
        }
    }

    /// HPDB index record (id@1 / parent@2) that never appears in favorites.
    pub const fn index(
        command: &'static str,
        field_count: u16,
        name_col: Option<usize>,
        geo: Option<GeoColumns>,
    ) -> Self {
        Self {
            command,
            field_count,
            id_col: Some(1),
            parent_col: Some(2),
            name_col,
            geo,
            channel: None,
            tech_col: None,
            favorites: FavoritesDialect::NONE,
        }
    }

    /// Builder: attach channel attribute columns.
    pub const fn with_channel(mut self, channel: ChannelColumns) -> Self {
        self.channel = Some(channel);
        self
    }

    /// Builder: mark the system-technology column on a header line.
    pub const fn with_tech(mut self, col: usize) -> Self {
        self.tech_col = Some(col);
        self
    }
}

/// Known capacity limits for a scanner model, surfaced in the UI so the user knows
/// what their device can hold. `0` means "unknown / not characterized".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScannerLimits {
    /// Maximum number of favorites lists the scanner can store.
    pub max_favorites_lists: u32,
    /// Maximum bytes per favorites list (the cart size gauge's ceiling).
    pub max_favorite_list_bytes: u64,
    /// Quick keys available (favorites/department/system/site).
    pub quick_keys: u32,
}

/// Which value a tone-mode option needs from the user: a CTCSS frequency, a DCS code,
/// or nothing (plain off / no squelch value). Lets a UI show the right value field
/// without re-encoding "tone-mode 1–3 = CTCSS, 4–7 = DCS".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneValueKind {
    /// No value needed (e.g. "Off").
    None,
    /// A CTCSS tone frequency.
    Ctcss,
    /// A DCS/DTCS code.
    Dcs,
}

/// One selectable value for an editable radio field: the human `label` a UI shows and the
/// on-radio `code` the writer stores. `value_kind` is meaningful only for tone modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldOption {
    pub label: String,
    pub code: u8,
    pub value_kind: ToneValueKind,
}

impl FieldOption {
    /// A plain option (label + code); not a tone mode.
    pub fn new(label: impl Into<String>, code: u8) -> Self {
        Self {
            label: label.into(),
            code,
            value_kind: ToneValueKind::None,
        }
    }
}

/// The editable-field option sets a clone-image radio offers a UI, each as an ordered list
/// of `(label, code)`. The **single source of truth** for the channel-form pickers, so no
/// front-end re-declares them (the clone-image analog of a scanner's `channel_value_options`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CloneFieldOptions {
    pub modes: Vec<FieldOption>,
    pub tone_modes: Vec<FieldOption>,
    pub steps: Vec<FieldOption>,
    pub powers: Vec<FieldOption>,
    pub duplexes: Vec<FieldOption>,
}

/// The class of radio a profile describes — the axis along which support is split
/// (SD-card database scanners vs. clone-image handhelds). Lets shared code branch on
/// the kind. See `docs/architecture.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioClass {
    /// A database scanner programmed via an SD-card file format (e.g. the SDS150).
    SdCardScanner,
    /// A radio programmed by cloning a fixed EEPROM image over serial (e.g. the FT-60R).
    CloneImage,
}

/// The small cross-cutting base every radio shares, regardless of how it's
/// programmed: its identity and class. Format-specific contracts (SD-card layout,
/// serial protocol, …) live in sub-traits like [`SdCardProfile`]. This is the seam
/// the multi-radio-class design (`docs/architecture.md`) is built on.
pub trait RadioProfile: Send + Sync {
    /// Stable identifier for persistence + the UI radio list, e.g. `"sds150"`.
    /// Lowercase, no spaces; never change it once shipped (it's a persisted key).
    fn id(&self) -> &'static str;

    /// Marketing/product name, e.g. `"SDS150"`.
    fn product_name(&self) -> &'static str;

    /// Manufacturer, e.g. `"Uniden"`.
    fn maker(&self) -> &'static str;

    /// Short transport / programming descriptor for the UI, e.g. `"SD card"` or
    /// `"serial clone"`.
    fn transport(&self) -> &'static str;

    /// Which class of radio this is (drives which sub-trait/backend applies).
    fn class(&self) -> RadioClass;

    /// The SD-card contract, if this is an SD-card scanner (else `None`). Lets the
    /// registry hold every radio as `dyn RadioProfile` and recover the class trait.
    fn as_sd_card(&self) -> Option<&dyn SdCardProfile> {
        None
    }

    /// The clone-image contract, if this is a clone-image radio (else `None`).
    fn as_clone_image(&self) -> Option<&dyn CloneImageProfile> {
        None
    }
}

/// One SD-card scanner family's complete format description. Implement + register to
/// add support for a new model. Extends [`RadioProfile`] with the SD-card contract
/// (layout, record schemas, favorites dialect, write rules).
pub trait SdCardProfile: RadioProfile {
    /// Capacity limits for this model (for the UI). Default: all unknown (`0`).
    fn limits(&self) -> ScannerLimits {
        ScannerLimits::default()
    }

    /// The `(TargetModel, FormatVersion)` this profile handles.
    fn model_key(&self) -> ModelKey;

    /// The serial-protocol model id (phase-2 `MDL` handshake), e.g.
    /// `"SDS150GBT"`. `None` if serial isn't characterized yet.
    fn serial_model_id(&self) -> Option<&'static str>;

    /// SD-card folder/file names for this model.
    fn sd_layout(&self) -> &SdLayout;

    /// Schema for a command, if known.
    fn record_schema(&self, command: &str) -> Option<&RecordSchema>;

    /// Record types a favorites file carries that the HPDB source files do **not**,
    /// and which a writer must therefore synthesize (on the SDS150: `DQKs_Status`
    /// and `BandPlan_P25`). A per-model format fact — kept here, not in the generic
    /// favorites code. Empty until characterized.
    ///
    /// Note: `Rectangle` is *not* here — it exists in HPDB source and is preserved
    /// by extraction, so it never needs synthesizing.
    fn favorites_synthesized_records(&self) -> &'static [&'static str] {
        &[]
    }

    /// The per-system `DQKs_Status` line a synthesized favorites list needs, as
    /// full fields (command first). `on` selects the blanket quick-key value.
    /// `None` if this model doesn't use the record. (Value-only model fact; the
    /// generic favorites code decides placement.)
    fn dqks_status_line(&self, _on: bool) -> Option<Vec<String>> {
        None
    }

    /// The standard P25 band plan for this model, as fields after the command
    /// (i.e. excluding the leading `BandPlan_P25` token). `None` if unknown.
    fn standard_p25_bandplan(&self) -> Option<&'static [&'static str]> {
        None
    }

    /// An `f_list.cfg` `F-List` line registering a favorites slot: full fields
    /// (command first), with `label` shown in the scanner UI and `filename` the
    /// slot file (e.g. `f_000005.hpd`). `None` if this model isn't characterized.
    /// Used **only for a brand-new list** — an existing list's fields are preserved
    /// verbatim on rewrite (we never regenerate a block we don't fully understand).
    fn f_list_entry(&self, _label: &str, _filename: &str) -> Option<Vec<String>> {
        None
    }

    /// Zero-based index of the **Monitor** flag (`On`=monitored / `Off`) within the
    /// `F-List` record, if known for this model. The one F-List field beyond
    /// name/slot that Platypus writes; everything else is carried through verbatim.
    /// `None` if this model's F-List layout isn't characterized.
    fn f_list_monitor_field(&self) -> Option<usize> {
        None
    }

    /// Zero-based index of the **Quick key** field (`"Off"` / `"0".."99"`) within the
    /// `F-List` record, if known. `None` if not characterized.
    fn f_list_quick_key_field(&self) -> Option<usize> {
        None
    }

    /// Zero-based index of the **NumberTag** field (`"Off"` / `"0".."99"`) within the
    /// `F-List` record, if known. `None` if not characterized.
    fn f_list_number_tag_field(&self) -> Option<usize> {
        None
    }

    /// Per-record `(column, value)` defaults the favorites dialect must fill —
    /// scan-setting columns the HPDB source leaves blank but a working favorites
    /// file populates (e.g. on the SDS150, `T-Freq` col 4 = `Off`, without which a
    /// trunk won't lock its control channel). Derived from a known-good favorites.
    /// Empty for records/models with no such defaults.
    fn favorites_field_defaults(&self, _command: &str) -> &'static [(usize, &'static str)] {
        &[]
    }

    /// The field index holding a record's **Avoid** flag (`"On"` = skip, `"Off"` =
    /// scan), for the record types that carry one (systems, departments/groups, and
    /// channels). `None` for records without an avoid flag (or models that don't
    /// characterize it). Lets the UI surface and toggle scan/skip per record.
    fn avoid_column(&self, _command: &str) -> Option<usize> {
        None
    }

    /// The field index holding a **voice channel** record's **Priority Channel** flag
    /// (`"On"`/`"Off"`), for the channel record types that carry one. `None` otherwise.
    /// Lets the UI surface and toggle per-channel priority.
    fn priority_column(&self, _command: &str) -> Option<usize> {
        None
    }

    /// The editable per-channel **value** setting names this model exposes (e.g.
    /// `"delay"`, `"attenuator"`). The UI enumerates these; `channel_value_column`
    /// resolves each to a column per record type. Empty until characterized.
    fn channel_value_fields(&self) -> &'static [&'static str] {
        &[]
    }

    /// The field index of an editable per-channel **value** setting named `field`
    /// (e.g. `"delay"`) on a voice-channel record. Column indices differ by record
    /// type (`C-Freq` vs `TGID`), and a field may exist on only one. `None` for
    /// unknown fields / uncharacterized models. The generic hook behind the editors.
    fn channel_value_column(&self, _command: &str, _field: &str) -> Option<usize> {
        None
    }

    /// The selectable values for an editable per-channel value `field` (e.g.
    /// `"alertColor"`), in on-radio order — the enumeration a UI picker offers. Stored
    /// verbatim in the record, so the value **is** the label. The **single source of
    /// truth** for these menus, so no front-end re-hardcodes them. Empty for free-form /
    /// numeric fields, unknown names, or uncharacterized models.
    fn channel_value_options(&self, _field: &str) -> Vec<String> {
        Vec::new()
    }

    /// Whether this profile handles a file with the given header. Default: match
    /// on `TargetModel`, and on `FormatVersion` when the file carries one.
    fn matches(&self, header: &FileHeader) -> bool {
        let key = self.model_key();
        let model_ok = header.target_model.as_deref() == Some(key.target_model.as_str());
        let version_ok = match &header.format_version {
            Some(v) => v == &key.format_version,
            None => true, // some files omit it; don't reject on that alone
        };
        model_ok && version_ok
    }
}

/// One clone-image radio's contract: the serial-transport spec + image detection.
/// Extends [`RadioProfile`] for radios programmed by cloning a fixed EEPROM image over
/// serial (e.g. the FT-60R). The binary image codec is model-specific (see
/// `super::ft60`), the clone-image analog of "the SD-card format is generic, the schema
/// is per-model".
pub trait CloneImageProfile: RadioProfile {
    /// The serial clone-transport spec (baud, image size, block framing, ACK, magic).
    fn clone_spec(&self) -> CloneSpec;

    /// The radio's fixed memory capacity (channel slots, banks, name length) — the single
    /// source of truth the UI surfaces instead of re-declaring the numbers.
    fn capacity(&self) -> CloneCapacity;

    /// The editable-field option sets (modes, tone modes, steps, powers, duplexes) this
    /// radio offers the channel form, each as ordered `(label, code)` — the single source
    /// of truth for the pickers, so no front-end re-declares them.
    fn field_options(&self) -> CloneFieldOptions;

    /// Whether a clone image/stream belongs to this radio. Default: the model magic.
    fn matches_image(&self, bytes: &[u8]) -> bool {
        self.clone_spec().header_matches(bytes)
    }
}

/// A clone-image radio's fixed memory capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneCapacity {
    /// Standard memory channel slots.
    pub channels: usize,
    /// Memory banks.
    pub banks: usize,
    /// Channel-name length (characters).
    pub name_len: usize,
}

/// Ordered set of known radio profiles across every class. Class-specific lookups
/// (`detect`, `detect_clone_image`) recover the sub-trait via [`RadioProfile::as_sd_card`]
/// / [`RadioProfile::as_clone_image`].
pub struct ProfileRegistry {
    profiles: Vec<Box<dyn RadioProfile>>,
}

impl ProfileRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self {
            profiles: Vec::new(),
        }
    }

    /// Registry preloaded with every built-in profile.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register(Box::new(super::sds150::Sds150::new()));
        reg.register(Box::new(super::ft60::Ft60::new()));
        reg
    }

    /// Add a profile of any class. Later registrations are lower priority in `detect*`.
    pub fn register(&mut self, profile: Box<dyn RadioProfile>) {
        self.profiles.push(profile);
    }

    /// Every registered profile (identity/class only).
    pub fn profiles(&self) -> impl Iterator<Item = &dyn RadioProfile> + '_ {
        self.profiles.iter().map(Box::as_ref)
    }

    /// The SD-card scanner profiles.
    pub fn sd_card_profiles(&self) -> impl Iterator<Item = &dyn SdCardProfile> + '_ {
        self.profiles.iter().filter_map(|p| p.as_sd_card())
    }

    /// The clone-image radio profiles.
    pub fn clone_image_profiles(&self) -> impl Iterator<Item = &dyn CloneImageProfile> + '_ {
        self.profiles.iter().filter_map(|p| p.as_clone_image())
    }

    /// First SD-card profile whose `matches` accepts this file header.
    pub fn detect(&self, header: &FileHeader) -> Option<&dyn SdCardProfile> {
        self.sd_card_profiles().find(|p| p.matches(header))
    }

    /// First clone-image profile whose magic matches these image bytes.
    pub fn detect_clone_image(&self, bytes: &[u8]) -> Option<&dyn CloneImageProfile> {
        self.clone_image_profiles().find(|p| p.matches_image(bytes))
    }

    /// The (first) clone-image profile — convenience while there's a single clone radio.
    pub fn clone_image(&self) -> Option<&dyn CloneImageProfile> {
        self.clone_image_profiles().next()
    }
}

impl Default for ProfileRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}
