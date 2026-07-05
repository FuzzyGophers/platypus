// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Favorites-dialect conversion.
//!
//! The favorites `.hpd` is a *different* dialect from the HPDB files, not just a
//! subset. Two things are well understood and validated against real cards, and
//! implemented here:
//!
//! - **Identity columns are blanked.** Hierarchical records keep their column
//!   count but MyId@1 / ParentId@2 are emptied ([`crate::device::FavoritesDialect`]).
//! - **Area tags are dropped.** `AreaState` / `AreaCounty` do not appear in
//!   favorites.
//!
//! ## Synthesis status (characterized 2026-06-29 via recon across real favorites)
//!
//! A real favorites file contains two record types absent from the HPDB source:
//!
//! - **`DQKs_Status`** — exactly one per system, right after the header line.
//!   A constant: `[blank, 100 × "On"|"Off"]` (every real card is uniformly one or
//!   the other). [`with_synthesized_dqks`] synthesizes it.
//! - **`BandPlan_P25`** — the standard P25 700/800/900 plan
//!   ([`SdCardProfile::standard_p25_bandplan`]). Per the spec it's **omissible unless a
//!   site designates a non-standard band plan**, so it's **not synthesized by default**
//!   (`--bandplan` opt-in). See the Phase B validation notes below — favorites (incl. a
//!   trunked P25 system) were written to and decoded on a real SDS150.
//!
//! `Rectangle` is **not** synthesized: it occurs in HPDB source too (for
//! `Rectangles`-shaped groups) and is preserved verbatim by extraction +
//! [`to_favorites_dialect`].
//!
//! ## Device validation (Phase B) — 2026-06-29
//! **Directly validated on a real SDS150** (decode observed):
//! - A **conventional** favorites we generated scanned with live activity.
//! - A **trunked P25** favorites (SAFE-T, location-filtered to one county) **decoded
//!   live traffic** (audio). It locked the control channel once
//!   `T-Freq col6 = 0` was applied (the NAC appeared then; `col21` changed in the
//!   same write, so not fully isolated).
//!
//! **Dialect facts matched to working files (necessity NOT individually isolated):**
//! - **Scan-setting defaults** (`favorites_field_defaults`) — copied from working
//!   files; we didn't prove each is required.
//! - **`T-Freq col6`** is the **LCN** (confirmed: Uniden File Spec V2.00). Forcing it
//!   to `0` is what SAFE-T working files use, and let SAFE-T lock its control channel.
//!   But it is **not universal** — across the owner's working files col6 is 0 on most
//!   T-Freqs yet a non-zero LCN on many (e.g. Honolulu). So forcing 0 is a heuristic
//!   valid for a control channel, not for systems that use explicit voice LCNs.
//! - **`BandPlan_P25`** is rare and, per the spec, **omissible unless the site
//!   designates a non-standard band plan**: only SAFE-T's 2 county-simulcast sites
//!   carry one; Honolulu P25 (10 sites) and a single-site test system (1) carry none.
//!   Not synthesized by default; `--bandplan` opt-in.
//!
//! **Corrected (was a false inference): `DQKs_Status` does NOT gate scanning.** Both
//! `On` and `Off` appear in working files — Hawaii's P25 trunks use `On`, another
//! working list's use `Off`. It's a quick-key preference. The earlier "Off fixed
//! talkgroup scanning" came from a single-site test, which had **no traffic** to
//! observe. We default to
//! `Off` (matches the owner's Home list); `On` is equally valid.
//!
//! `f_list.cfg` registration works; `app_data.cfg` deletion fine; no SD errors.

use std::collections::{HashMap, HashSet};

use crate::device::SdCardProfile;
use crate::extract::{is_voice_channel, Extraction, System};
use crate::format::{Document, Line};
use crate::model::{Record, RecordKind};

/// Convert full-HPDB lines to the favorites dialect: drop `AreaState`/`AreaCounty`
/// and blank each record's identity columns. Pure and lossless-preserving for the
/// records it keeps — the result round-trips. See the module note on the synthesis
/// gap before treating the output as a complete favorites file.
pub fn to_favorites_dialect(doc: &Document, profile: &dyn SdCardProfile) -> Document {
    let mut lines = Vec::with_capacity(doc.lines.len());

    for line in &doc.lines {
        match RecordKind::from_command(line.command()) {
            // Area tags are not carried in favorites.
            RecordKind::AreaState | RecordKind::AreaCounty => continue,
            _ => {
                let mut out = line.clone();
                if let Some(schema) = profile.record_schema(line.command()) {
                    for &col in schema.favorites.blanked_id_columns {
                        if let Some(field) = out.fields.get_mut(col) {
                            field.clear();
                        }
                    }
                }
                // Fill the favorites scan-setting defaults the source leaves blank
                // (e.g. T-Freq col 4 = "Off" so a trunk can lock its control channel).
                for &(col, value) in profile.favorites_field_defaults(line.command()) {
                    if out.fields.len() <= col {
                        out.fields.resize(col + 1, String::new());
                    }
                    out.fields[col] = value.to_string();
                }
                lines.push(out);
            }
        }
    }

    Document { lines }
}

/// Insert a synthesized `DQKs_Status` line after each system header (`Conventional`
/// / `Trunk`) that doesn't already have one. Idempotent. `departments_on` picks the
/// blanket quick-key value — see the module note on the unconfirmed polarity. A
/// no-op for models whose profile returns no template.
pub fn with_synthesized_dqks(
    doc: &Document,
    profile: &dyn SdCardProfile,
    departments_on: bool,
) -> Document {
    let Some(template) = profile.dqks_status_line(departments_on) else {
        return doc.clone();
    };

    let mut lines = Vec::with_capacity(doc.lines.len() + 8);
    for (i, line) in doc.lines.iter().enumerate() {
        let is_system = matches!(
            RecordKind::from_command(line.command()),
            RecordKind::Conventional | RecordKind::Trunk
        );
        let next_is_dqks = doc
            .lines
            .get(i + 1)
            .is_some_and(|n| n.command() == "DQKs_Status");
        lines.push(line.clone());
        if is_system && !next_is_dqks {
            lines.push(Line {
                fields: template.clone(),
                ending: line.ending,
            });
        }
    }
    Document { lines }
}

/// Insert a synthesized `BandPlan_P25` after each `Site` of a **P25** trunk that
/// doesn't already have one. The band-plan value is the model's standard P25 plan.
/// Idempotent; a no-op for models/data without P25 trunks or a known plan.
///
/// ⚠️ Placement hypothesis: real cards carry a band plan on only some sites (the
/// simulcast ones), but the rule isn't confirmed — this inserts on every P25-trunk
/// site, which is the safe over-inclusive default until the device test (Phase B)
/// settles which sites actually need one.
pub fn with_synthesized_bandplan(doc: &Document, profile: &dyn SdCardProfile) -> Document {
    let Some(plan) = profile.standard_p25_bandplan() else {
        return doc.clone();
    };

    let mut lines = Vec::with_capacity(doc.lines.len() + 8);
    let mut in_p25_trunk = false;
    for (i, line) in doc.lines.iter().enumerate() {
        match RecordKind::from_command(line.command()) {
            RecordKind::Conventional => in_p25_trunk = false,
            RecordKind::Trunk => {
                in_p25_trunk = Record::new(line, profile)
                    .and_then(|r| r.tech())
                    .is_some_and(|t| t.contains("P25"));
            }
            _ => {}
        }

        let next_is_bandplan = doc
            .lines
            .get(i + 1)
            .is_some_and(|n| n.command() == "BandPlan_P25");
        lines.push(line.clone());

        if in_p25_trunk && line.command() == "Site" && !next_is_bandplan {
            let mut fields = Vec::with_capacity(1 + plan.len());
            fields.push("BandPlan_P25".to_string());
            fields.extend(plan.iter().map(|s| s.to_string()));
            lines.push(Line {
                fields,
                ending: line.ending,
            });
        }
    }
    Document { lines }
}

/// Build a complete favorites document from a filtered HPDB selection: apply the
/// favorites dialect (drop area tags, blank ids, fill scan-setting defaults), then
/// synthesize `DQKs_Status` (one per system).
///
/// `departments_on` is a **user preference**, surfaced as a UI toggle — it sets the
/// department quick-key state (`DQKs_Status` all `On` vs all `Off`). It is *not* a
/// scan requirement: both values appear in working files (Hawaii's P25 trunks use
/// `On`, another list's use `Off`) and both work. Callers typically default to `false`.
///
/// Device-validated 2026-06-29 for conventional and P25 trunked systems.
///
/// **`BandPlan_P25` is deliberately NOT synthesized.** A working single-site P25 test
/// has none, and a 156-site system carried one on only 2 sites — the scanner uses its
/// built-in standard plan for almost all sites. Adding a band plan where the system
/// doesn't expect one *prevents* the trunk from locking. Per the Uniden spec a band
/// plan is omissible unless the site designates a non-standard plan; which rare sites
/// need one can't be known from the card alone (it's not in the HPDB), so resolve that
/// later from RR data. [`with_synthesized_bandplan`] stays available for when we do.
pub fn build_favorites(
    doc: &Document,
    profile: &dyn SdCardProfile,
    departments_on: bool,
) -> Document {
    let dialect = to_favorites_dialect(doc, profile);
    with_synthesized_dqks(&dialect, profile, departments_on)
}

/// Merge `addition`'s systems into `base`, **deduping by system and channel** so
/// adding to a list doesn't create duplicates. A system in `addition` that matches a
/// `base` system (same kind + name) has its *new* channels (by talkgroup id /
/// frequency, grouped) appended to that base system; an unmatched system is appended
/// whole. Used by the "add selection to an existing list" edit. Stays favorites
/// dialect (selection of existing lines).
pub fn merge_favorites(
    base: &Document,
    addition: &Document,
    profile: &dyn SdCardProfile,
) -> Document {
    let base_ext = Extraction::segment(base, profile);
    let add_ext = Extraction::segment(addition, profile);
    let add_systems: Vec<System> = add_ext.systems().collect();

    let key = |s: &System| format!("{}|{}", s.header().command(), s.name().unwrap_or(""));
    let mut add_by_key: HashMap<String, usize> = HashMap::new();
    for (i, s) in add_systems.iter().enumerate() {
        add_by_key.entry(key(s)).or_insert(i);
    }

    let mut out: Vec<Line> = base_ext.preamble_lines().to_vec();
    let mut used = HashSet::new();
    for bsys in base_ext.systems() {
        out.extend_from_slice(bsys.lines());
        if let Some(&ai) = add_by_key.get(&key(&bsys)) {
            used.insert(ai);
            let existing = channel_keys(&bsys, profile);
            append_new_groups(&add_systems[ai], &existing, profile, &mut out);
        }
    }
    for (i, asys) in add_systems.iter().enumerate() {
        if !used.contains(&i) {
            out.extend_from_slice(asys.lines());
        }
    }
    Document { lines: out }
}

/// Stable identity for a voice channel: its talkgroup id, else its frequency.
fn channel_key(rec: &Record) -> Option<String> {
    if let Some(t) = rec.talkgroup() {
        return Some(format!("t{t}"));
    }
    rec.frequency_hz().map(|f| format!("f{f}"))
}

/// The channel keys already present in a system.
fn channel_keys(sys: &System, profile: &dyn SdCardProfile) -> HashSet<String> {
    sys.lines()
        .iter()
        .filter(|l| is_voice_channel(l.command()))
        .filter_map(|l| Record::new(l, profile).as_ref().and_then(channel_key))
        .collect()
}

/// Append the addition system's groups whose channels aren't already in `existing`,
/// keeping only the new channels (and the group's `Rectangle` bounds). Emits nothing
/// for groups with no new channels.
fn append_new_groups(
    asys: &System,
    existing: &HashSet<String>,
    profile: &dyn SdCardProfile,
    out: &mut Vec<Line>,
) {
    let is_group = |c: &str| matches!(c, "T-Group" | "C-Group");
    let mut group: Option<Line> = None;
    let mut extra: Vec<Line> = Vec::new(); // Rectangle bounds under the group
    let mut kept: Vec<Line> = Vec::new(); // new channels under the group
    let flush = |group: &mut Option<Line>,
                 extra: &mut Vec<Line>,
                 kept: &mut Vec<Line>,
                 out: &mut Vec<Line>| {
        if let Some(g) = group.take() {
            if !kept.is_empty() {
                out.push(g);
                out.append(extra);
                out.append(kept);
            }
        }
        extra.clear();
        kept.clear();
    };

    for line in asys.lines() {
        let cmd = line.command();
        if is_group(cmd) {
            flush(&mut group, &mut extra, &mut kept, out);
            group = Some(line.clone());
        } else if is_voice_channel(cmd) {
            let new = Record::new(line, profile)
                .as_ref()
                .and_then(channel_key)
                .is_none_or(|k| !existing.contains(&k));
            if new {
                kept.push(line.clone());
            }
        } else if cmd == "Rectangle" && group.is_some() {
            extra.push(line.clone());
        }
        // header / DQKs / sites / t-freqs belong to the base system already — skip.
    }
    flush(&mut group, &mut extra, &mut kept, out);
}

/// Reorder a document's **systems alphabetically by name**, preserving the preamble
/// and each system's internal structure (sites, groups, channels, DQKs). For tidying
/// a favorites list — the kind of sort Sentinel never offered.
pub fn sort_systems(doc: &Document, profile: &dyn SdCardProfile) -> Document {
    let ext = crate::extract::Extraction::segment(doc, profile);
    let mut systems: Vec<(String, &[crate::format::Line])> = ext
        .systems()
        .map(|s| (s.name().unwrap_or("").to_lowercase(), s.lines()))
        .collect();
    systems.sort_by(|a, b| a.0.cmp(&b.0));

    let mut lines = ext.preamble_lines().to_vec();
    for (_, sys_lines) in systems {
        lines.extend_from_slice(sys_lines);
    }
    Document { lines }
}

/// Which record an avoid toggle targets within one system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvoidLevel {
    /// The system header (`Trunk`/`Conventional`) — skip the whole system.
    System,
    /// The `idx`-th department (`T-Group`/`C-Group`) — skip all its channels.
    Group,
    /// The `idx`-th voice channel (`TGID`/`C-Freq`) in the system.
    Channel,
}

/// Set the **Avoid** flag (`"On"` = skip / `"Off"` = scan) on one record in system
/// `si`, returning a new document. `idx` selects the department or channel within the
/// system (ignored for `System`). Mirrors the radio's hierarchy: avoiding a system or
/// department skips its channels without touching their own flags. A no-op (clone)
/// if the target or its avoid column can't be resolved.
pub fn set_avoid(
    doc: &Document,
    profile: &dyn SdCardProfile,
    si: usize,
    level: AvoidLevel,
    idx: usize,
    avoid: bool,
) -> Document {
    set_record_flag(doc, si, level, idx, avoid, |cmd| profile.avoid_column(cmd))
}

/// Set the **Priority Channel** flag (`"On"` = priority / `"Off"`) on the `idx`-th
/// voice channel of system `si`, returning a new document. A no-op (clone) if the
/// target or its priority column can't be resolved.
pub fn set_priority(
    doc: &Document,
    profile: &dyn SdCardProfile,
    si: usize,
    idx: usize,
    on: bool,
) -> Document {
    set_record_flag(doc, si, AvoidLevel::Channel, idx, on, |cmd| {
        profile.priority_column(cmd)
    })
}

/// Set an arbitrary per-channel value field (`field`, e.g. `"delay"`) on the `idx`-th
/// voice channel of system `si`, returning a new document. The value is written
/// verbatim. No-op (clone) if the field/column can't be resolved for the model.
pub fn set_channel_value(
    doc: &Document,
    profile: &dyn SdCardProfile,
    si: usize,
    idx: usize,
    field: &str,
    value: &str,
) -> Document {
    set_record_value(doc, si, AvoidLevel::Channel, idx, value, |cmd| {
        profile.channel_value_column(cmd, field)
    })
}

/// Shared machinery for the per-record `On`/`Off` flag toggles (avoid, priority, …):
/// locate the (system `si`, `level`, `idx`) record, then set the column the `column`
/// resolver returns for that record type. No-op clone if either can't be resolved.
fn set_record_flag(
    doc: &Document,
    si: usize,
    level: AvoidLevel,
    idx: usize,
    on: bool,
    column: impl Fn(&str) -> Option<usize>,
) -> Document {
    set_record_value(doc, si, level, idx, if on { "On" } else { "Off" }, column)
}

/// Set the located (system `si`, `level`, `idx`) record's resolved column to `value`.
fn set_record_value(
    doc: &Document,
    si: usize,
    level: AvoidLevel,
    idx: usize,
    value: &str,
    column: impl Fn(&str) -> Option<usize>,
) -> Document {
    let mut out = doc.clone();
    if let Some(li) = locate_record(&out, si, level, idx) {
        if let Some(col) = column(out.lines[li].command()) {
            if let Some(field) = out.lines[li].fields.get_mut(col) {
                *field = value.to_string();
            }
        }
    }
    out
}

/// Line index of the (system `si`, `level`, `idx`) target record within `doc`.
fn locate_record(doc: &Document, si: usize, level: AvoidLevel, idx: usize) -> Option<usize> {
    // System start lines (same rule as Extraction::segment).
    let starts: Vec<usize> = doc
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| matches!(l.command(), "Conventional" | "Trunk"))
        .map(|(i, _)| i)
        .collect();
    let start = *starts.get(si)?;
    let end = starts.get(si + 1).copied().unwrap_or(doc.lines.len());

    match level {
        AvoidLevel::System => Some(start),
        AvoidLevel::Group => {
            let mut g = 0;
            (start..end).find(|&i| {
                if matches!(doc.lines[i].command(), "C-Group" | "T-Group") {
                    let hit = g == idx;
                    g += 1;
                    hit
                } else {
                    false
                }
            })
        }
        AvoidLevel::Channel => {
            let mut c = 0;
            (start..end).find(|&i| {
                if is_voice_channel(doc.lines[i].command()) {
                    let hit = c == idx;
                    c += 1;
                    hit
                } else {
                    false
                }
            })
        }
    }
}

/// Whether `doc` contains all the favorites-only record types the profile says a
/// complete favorites file needs (see
/// [`SdCardProfile::favorites_synthesized_records`]). A conversion produced by
/// [`to_favorites_dialect`] reports `false` until the synthesis gap is closed — a
/// guard against shipping an incomplete list. Models that declare no such records
/// trivially return `true`.
pub fn has_synthesized_records(doc: &Document, profile: &dyn SdCardProfile) -> bool {
    profile
        .favorites_synthesized_records()
        .iter()
        .all(|rec| doc.lines.iter().any(|l| l.command() == *rec))
}

/// True if any line is an `AreaState`/`AreaCounty` (i.e. *not* favorites dialect).
fn has_area_tags(doc: &Document) -> bool {
    doc.lines.iter().any(|l| {
        matches!(
            RecordKind::from_command(l.command()),
            RecordKind::AreaState | RecordKind::AreaCounty
        )
    })
}

/// Necessary (not sufficient) check that a document is in the favorites dialect:
/// it carries no `AreaState`/`AreaCounty` tags. Does not check the synthesis gap
/// (see [`has_synthesized_records`]).
pub fn is_favorites_dialect(doc: &Document) -> bool {
    !has_area_tags(doc)
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

    #[test]
    fn blanks_ids_and_drops_area_tags() {
        let doc = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&["Trunk", "TrunkId=7678", "StateId=11", "American University"]),
                line(&["AreaState", "CountyId=315", "StateId=11"]),
                line(&["AreaCounty", "CountyId=315", "CountyId=315"]),
                line(&["Site", "SiteId=22044", "TrunkId=7678", "Anderson Hall"]),
            ],
        };
        let fav = to_favorites_dialect(&doc, &Sds150::new());

        // Area tags gone.
        assert!(!has_area_tags(&fav));
        // Trunk + Site ids blanked, names kept, column count preserved.
        let trunk = &fav.lines[1];
        assert_eq!(trunk.command(), "Trunk");
        assert_eq!(trunk.field(1), Some(""));
        assert_eq!(trunk.field(2), Some(""));
        assert_eq!(trunk.field(3), Some("American University"));
        let site = &fav.lines[2];
        assert_eq!(site.field(1), Some(""));
        assert_eq!(site.field(2), Some(""));
        assert_eq!(site.field(3), Some("Anderson Hall"));
    }

    #[test]
    fn set_avoid_targets_group_channel_system() {
        let profile = Sds150::new();
        // One trunk: 2 departments, each with a channel. Field 4 (index 4) = avoid.
        let doc = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&["Trunk", "", "", "SAFE-T", "Off", "", "P25Standard"]),
                line(&["T-Group", "", "", "Alpha County", "Off", "0", "0", "0"]),
                line(&["TGID", "", "", "Alpha Disp", "Off", "1001", "FM", "2"]),
                line(&["T-Group", "", "", "Bravo County", "Off", "0", "0", "0"]),
                line(&["TGID", "", "", "Bravo Police", "Off", "1002", "FM", "2"]),
            ],
        };
        // Avoid department 1 (Bravo County).
        let d = set_avoid(&doc, &profile, 0, AvoidLevel::Group, 1, true);
        assert_eq!(d.lines[4].command(), "T-Group");
        assert_eq!(d.lines[4].field(4), Some("On"), "group avoided");
        assert_eq!(d.lines[2].field(4), Some("Off"), "other group untouched");
        assert_eq!(
            d.lines[5].field(4),
            Some("Off"),
            "child channel flag untouched"
        );

        // Avoid channel 0 (EC Disp), then re-enable.
        let d2 = set_avoid(&doc, &profile, 0, AvoidLevel::Channel, 0, true);
        assert_eq!(d2.lines[3].field(4), Some("On"));
        let d3 = set_avoid(&d2, &profile, 0, AvoidLevel::Channel, 0, false);
        assert_eq!(d3.lines[3].field(4), Some("Off"));

        // Avoid the whole system.
        let d4 = set_avoid(&doc, &profile, 0, AvoidLevel::System, 0, true);
        assert_eq!(d4.lines[1].field(4), Some("On"));
    }

    #[test]
    fn set_priority_targets_channel() {
        let profile = Sds150::new();
        // A full-width TGID (17 fields) so the Priority Channel column (15) exists.
        let doc = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&["Trunk", "", "", "SAFE-T", "Off", "", "P25Standard"]),
                line(&["T-Group", "", "", "Alpha County", "Off", "0", "0", "0"]),
                line(&[
                    "TGID",
                    "",
                    "",
                    "Alpha Disp",
                    "Off",
                    "1001",
                    "ALL",
                    "2",
                    "0",
                    "0",
                    "Off",
                    "Auto",
                    "Off",
                    "Off",
                    "Off",
                    "Off",
                    "Off",
                ]),
            ],
        };
        // Channel 0 → priority on; avoid (field 4) untouched.
        let d = set_priority(&doc, &profile, 0, 0, true);
        assert_eq!(d.lines[3].command(), "TGID");
        assert_eq!(d.lines[3].field(15), Some("On"), "priority set");
        assert_eq!(d.lines[3].field(4), Some("Off"), "avoid untouched");
        // Toggle back off.
        let d2 = set_priority(&d, &profile, 0, 0, false);
        assert_eq!(d2.lines[3].field(15), Some("Off"));
    }

    #[test]
    fn set_channel_value_sets_delay() {
        let profile = Sds150::new();
        // Full-width TGID so the Delay column (8) exists.
        let doc = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&["Trunk", "", "", "SAFE-T", "Off", "", "P25Standard"]),
                line(&["T-Group", "", "", "Alpha County", "Off", "0", "0", "0"]),
                line(&[
                    "TGID",
                    "",
                    "",
                    "Alpha Disp",
                    "Off",
                    "1001",
                    "ALL",
                    "2",
                    "2",
                    "0",
                    "Off",
                    "Auto",
                    "Off",
                    "Off",
                    "Off",
                    "Off",
                    "Off",
                ]),
            ],
        };
        // Channel 0 delay 2 → 5; avoid (4) and TGID (5) untouched.
        let d = set_channel_value(&doc, &profile, 0, 0, "delay", "5");
        assert_eq!(d.lines[3].field(8), Some("5"), "delay set");
        assert_eq!(d.lines[3].field(4), Some("Off"), "avoid untouched");
        assert_eq!(d.lines[3].field(5), Some("1001"), "tgid untouched");
        // Unknown field is a no-op.
        let d2 = set_channel_value(&doc, &profile, 0, 0, "nonesuch", "9");
        assert_eq!(d2.lines[3].field(8), Some("2"), "unknown field no-op");
    }

    #[test]
    fn conversion_is_flagged_incomplete() {
        let doc = Document {
            lines: vec![line(&["Trunk", "TrunkId=1", "StateId=1", "X"])],
        };
        let profile = Sds150::new();
        let fav = to_favorites_dialect(&doc, &profile);
        // to_favorites_dialect alone doesn't add DQKs_Status / BandPlan_P25.
        assert!(!has_synthesized_records(&fav, &profile));
    }

    #[test]
    fn build_favorites_is_complete_for_a_p25_system() {
        let profile = Sds150::new();
        // A P25 trunk (tech at col 6) with a site + an area tag to drop.
        let doc = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&[
                    "Trunk",
                    "TrunkId=1",
                    "StateId=1",
                    "Sys",
                    "Off",
                    "",
                    "P25Standard",
                ]),
                line(&["AreaCounty", "TrunkId=1", "CountyId=5"]),
                line(&[
                    "Site",
                    "SiteId=2",
                    "TrunkId=1",
                    "S1",
                    "Off",
                    "45.0",
                    "-100.0",
                    "10",
                ]),
                line(&["T-Freq", "TFreqId=0", "SiteId=2", "", "", "851000000"]),
            ],
        };
        let fav = build_favorites(&doc, &profile, true);

        assert!(is_favorites_dialect(&fav)); // area tag dropped
        assert_eq!(
            fav.lines
                .iter()
                .filter(|l| l.command() == "DQKs_Status")
                .count(),
            1
        );
        // No band plan synthesized — adding one stops a real trunk from locking.
        assert_eq!(
            fav.lines
                .iter()
                .filter(|l| l.command() == "BandPlan_P25")
                .count(),
            0
        );
        // ids blanked on the kept records
        let site = fav.lines.iter().find(|l| l.command() == "Site").unwrap();
        assert_eq!(site.field(1), Some(""));
        // now structurally complete, and still byte round-trips
        assert!(has_synthesized_records(&fav, &profile));
        assert_eq!(
            fav.to_bytes(),
            Document::parse(&fav.to_bytes()).unwrap().to_bytes()
        );
    }

    #[test]
    fn bandplan_only_on_p25_not_other_tech() {
        let profile = Sds150::new();
        let doc = Document {
            lines: vec![
                line(&[
                    "Trunk",
                    "TrunkId=1",
                    "StateId=1",
                    "DMR",
                    "Off",
                    "",
                    "MotoTrbo",
                ]),
                line(&[
                    "Site",
                    "SiteId=2",
                    "TrunkId=1",
                    "S1",
                    "Off",
                    "45.0",
                    "-100.0",
                    "10",
                ]),
            ],
        };
        let fav = with_synthesized_bandplan(&doc, &profile);
        assert!(!fav.lines.iter().any(|l| l.command() == "BandPlan_P25"));
    }

    #[test]
    fn bandplan_synthesized_on_p25_site_idempotent_and_roundtrips() {
        let profile = Sds150::new();
        // A P25 trunk (tech at col 6) with one site.
        let doc = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&[
                    "Trunk",
                    "TrunkId=1",
                    "StateId=1",
                    "Sys",
                    "Off",
                    "",
                    "P25Standard",
                ]),
                line(&[
                    "Site",
                    "SiteId=2",
                    "TrunkId=1",
                    "S1",
                    "Off",
                    "45.0",
                    "-100.0",
                    "10",
                ]),
            ],
        };
        let out = with_synthesized_bandplan(&doc, &profile);

        // Exactly one band plan, inserted immediately after the Site.
        assert_eq!(
            out.lines
                .iter()
                .filter(|l| l.command() == "BandPlan_P25")
                .count(),
            1
        );
        let site_idx = out
            .lines
            .iter()
            .position(|l| l.command() == "Site")
            .unwrap();
        assert_eq!(out.lines[site_idx + 1].command(), "BandPlan_P25");
        // The synthesized record carries the model's standard P25 plan.
        let plan = profile.standard_p25_bandplan().unwrap();
        let synthesized: Vec<&str> = out.lines[site_idx + 1].fields[1..]
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(synthesized, plan);

        // Idempotent: an already-present band plan is not duplicated.
        let again = with_synthesized_bandplan(&out, &profile);
        assert_eq!(again.lines.len(), out.lines.len());
        assert_eq!(
            again
                .lines
                .iter()
                .filter(|l| l.command() == "BandPlan_P25")
                .count(),
            1
        );

        // Still byte round-trips.
        assert_eq!(
            out.to_bytes(),
            Document::parse(&out.to_bytes()).unwrap().to_bytes()
        );
    }

    #[test]
    fn sort_systems_reorders_by_name_keeping_children_and_roundtrips() {
        let profile = Sds150::new();
        // Preamble + two systems out of order, each with a distinct child channel.
        let doc = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&["Conventional", "", "", "Bravo System"]),
                line(&["C-Group", "", "", "Bravo Group"]),
                line(&["C-Freq", "", "", "Bravo Ch", "Off", "154000000"]),
                line(&["Conventional", "", "", "Alpha System"]),
                line(&["C-Group", "", "", "Alpha Group"]),
                line(&["C-Freq", "", "", "Alpha Ch", "Off", "155000000"]),
            ],
        };
        let sorted = sort_systems(&doc, &profile);

        // Preamble stays first.
        assert_eq!(sorted.lines[0].command(), "TargetModel");
        // System headers now alphabetical: Alpha before Bravo.
        let headers: Vec<&str> = sorted
            .lines
            .iter()
            .filter(|l| l.command() == "Conventional")
            .map(|l| l.field(3).unwrap())
            .collect();
        assert_eq!(headers, ["Alpha System", "Bravo System"]);
        // Each system's children stay attached to it: the Alpha header is
        // immediately followed by its group and channel, then Bravo's.
        let alpha = sorted
            .lines
            .iter()
            .position(|l| l.field(3) == Some("Alpha System"))
            .unwrap();
        assert_eq!(sorted.lines[alpha + 1].field(3), Some("Alpha Group"));
        assert_eq!(sorted.lines[alpha + 2].field(3), Some("Alpha Ch"));

        // No lines lost, and it still byte round-trips.
        assert_eq!(sorted.lines.len(), doc.lines.len());
        assert_eq!(
            sorted.to_bytes(),
            Document::parse(&sorted.to_bytes()).unwrap().to_bytes()
        );
    }

    #[test]
    fn fills_favorites_field_defaults() {
        let profile = Sds150::new();
        let doc = Document {
            lines: vec![
                line(&[
                    "Trunk",
                    "TrunkId=1",
                    "StateId=1",
                    "Sys",
                    "Off",
                    "",
                    "P25Standard",
                ]),
                line(&[
                    "Site",
                    "SiteId=2",
                    "TrunkId=1",
                    "S1",
                    "Off",
                    "45.0",
                    "-100.0",
                    "10",
                ]),
                line(&[
                    "T-Freq",
                    "TFreqId=0",
                    "SiteId=2",
                    "",
                    "",
                    "851000000",
                    "1",
                    "Srch",
                    "Any",
                ]),
            ],
        };
        let fav = to_favorites_dialect(&doc, &profile);

        // The control-channel default that makes a trunk lock (was blank).
        let tfreq = fav.lines.iter().find(|l| l.command() == "T-Freq").unwrap();
        assert_eq!(tfreq.field(4), Some("Off"));
        // Trunk scan-setting defaults.
        let trunk = fav.lines.iter().find(|l| l.command() == "Trunk").unwrap();
        assert_eq!(trunk.field(11), Some("Srch"));
        assert_eq!(trunk.field(20), Some("On"));
        // Site defaults.
        let site = fav.lines.iter().find(|l| l.command() == "Site").unwrap();
        assert_eq!(site.field(13), Some("400"));
    }

    #[test]
    fn merge_dedupes_channels_and_appends_new_systems() {
        let profile = Sds150::new();
        let base = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&["Trunk", "", "", "SAFE-T", "Off", "", "P25Standard"]),
                line(&["T-Group", "", "", "Alpha County"]),
                line(&["TGID", "", "", "PD Disp", "Off", "1001"]),
            ],
        };
        // Addition: same SAFE-T (the existing TG 1001 + a NEW TG 1002), plus a
        // brand-new system.
        let addition = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&["Trunk", "", "", "SAFE-T", "Off", "", "P25Standard"]),
                line(&["T-Group", "", "", "Alpha County"]),
                line(&["TGID", "", "", "PD Disp", "Off", "1001"]),
                line(&["TGID", "", "", "Fire Disp", "Off", "1002"]),
                line(&["Conventional", "", "", "Local FD"]),
                line(&["C-Group", "", "", "Fire"]),
                line(&["C-Freq", "", "", "Dispatch", "Off", "154000000"]),
            ],
        };
        let merged = merge_favorites(&base, &addition, &profile);
        let s = String::from_utf8(merged.to_bytes()).unwrap();

        // SAFE-T appears once (no duplicate system).
        assert_eq!(s.matches("\tSAFE-T\t").count(), 1);
        // The pre-existing talkgroup isn't duplicated; the new one was added.
        assert_eq!(s.matches("1001").count(), 1);
        assert!(s.contains("1002"));
        // The genuinely new system was appended whole.
        assert!(s.contains("Local FD"));
        assert!(s.contains("154000000"));
        // Still round-trips.
        assert_eq!(
            merged.to_bytes(),
            Document::parse(&merged.to_bytes()).unwrap().to_bytes()
        );
    }

    #[test]
    fn synthesizes_one_dqks_per_system() {
        let profile = Sds150::new();
        let doc = Document {
            lines: vec![
                line(&["TargetModel", "BCDx36HP"]),
                line(&["Conventional", "", "", "Alpha County"]),
                line(&["C-Group", "", "", "Schools"]),
                line(&["Trunk", "", "", "SAFE-T"]),
                line(&["Site", "", "", "Primary"]),
            ],
        };
        let out = with_synthesized_dqks(&doc, &profile, true);

        // Exactly one DQKs_Status per system, each immediately after its header.
        assert_eq!(out.lines[1].command(), "Conventional");
        assert_eq!(out.lines[2].command(), "DQKs_Status");
        assert_eq!(
            out.lines
                .iter()
                .filter(|l| l.command() == "DQKs_Status")
                .count(),
            2
        );
        // Structure: 102 fields, blank id, 100 uniform slots.
        let d = &out.lines[2];
        assert_eq!(d.fields.len(), 102);
        assert_eq!(d.field(1), Some(""));
        assert!(d.fields[2..].iter().all(|v| v == "On"));

        // Idempotent: re-running adds nothing.
        let again = with_synthesized_dqks(&out, &profile, true);
        assert_eq!(again.lines.len(), out.lines.len());
    }

    #[test]
    fn standard_bandplan_is_available_and_well_formed() {
        let bp = Sds150::new().standard_p25_bandplan().unwrap();
        assert_eq!(bp.len(), 49); // fields after the BandPlan_P25 command
        assert_eq!(bp[0], ""); // blank MyId column
        assert_eq!(bp[1], "851006250"); // P25 800 MHz base
    }
}
