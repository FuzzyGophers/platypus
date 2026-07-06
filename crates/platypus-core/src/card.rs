// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! SD-card filesystem helpers — path resolution, the favorites write path, and
//! the spec's safety rules.
//!
//! These operate on a mounted card root (e.g. `/Volumes/NO NAME`). Path layout
//! comes from the profile, so model differences stay in one place.
//!
//! [`commit_favorites`] is the full write path: write the slot file, register it
//! in `f_list.cfg`, and delete `app_data.cfg` (the non-negotiable safety rule).
//! The favorites *content* is assembled by [`crate::favorites::build_favorites`];
//! its band-plan/DQKs specifics still await the Phase-B device validation, so
//! callers should write to a spare/copied card until then.

use std::path::{Path, PathBuf};
use std::{fs, io};

use crate::device::SdCardProfile;
use crate::extract::{self, Extraction};
use crate::format::{Document, Line, LineEnding};

/// Metadata for one favorites list registered on the card (from `f_list.cfg` plus
/// the slot file). For showing and managing the user's current configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavoriteListInfo {
    /// Slot number (from the `f_<NNNNNN>.hpd` filename).
    pub slot: u32,
    /// Display name shown on the scanner (`F-List` field 1).
    pub name: String,
    /// Slot filename, e.g. `f_000005.hpd`.
    pub filename: String,
    /// Systems in the list (0 if the slot file is missing/unreadable).
    pub systems: usize,
    /// Voice channels (talkgroups + conventional freqs) in the list.
    pub channels: usize,
    /// Slot file size in bytes.
    pub bytes: u64,
    /// The list's **Monitor** flag (`F-List` Monitor field = `On`): scanned
    /// whenever the favorites list is active, regardless of quick keys. `false`
    /// for models whose F-List Monitor field isn't characterized.
    pub monitor: bool,
    /// The list's **quick key** (`F-List` Quick key field): `Some(0..=99)`, or
    /// `None` for "Off"/unassigned (then the list is always scanned).
    pub quick_key: Option<u32>,
    /// The list's **number tag** (`F-List` NumberTag field): `Some(0..=99)`, or
    /// `None` for "Off"/unassigned.
    pub number_tag: Option<u32>,
}

/// One desired `F-List` entry for [`apply_favorites_layout`]. `monitor`/`quick_key`/
/// `number_tag` are per-field overrides: `None` leaves the existing field as-is
/// (preserved), `Some(..)` sets it. `quick_key`/`number_tag` values are the raw
/// field string (`"Off"` or `"0".."99"`).
#[derive(Debug, Clone)]
pub struct ListLayout {
    pub slot: u32,
    pub name: String,
    pub monitor: Option<bool>,
    pub quick_key: Option<String>,
    pub number_tag: Option<String>,
}

impl ListLayout {
    /// A rename/reorder-only entry (leaves Monitor/quick-key/number-tag untouched).
    pub fn new(slot: u32, name: impl Into<String>) -> Self {
        Self {
            slot,
            name: name.into(),
            monitor: None,
            quick_key: None,
            number_tag: None,
        }
    }
}

/// Parse an `F-List` quick-key / number-tag field: `"Off"`/empty → `None`, else the
/// number.
fn parse_qk(field: Option<&str>) -> Option<u32> {
    match field {
        Some(s) if !s.is_empty() && !s.eq_ignore_ascii_case("Off") => s.parse().ok(),
        _ => None,
    }
}

/// Read every favorites list registered in `f_list.cfg`, with each list's system /
/// channel counts (parsed from its slot file). Lists appear in `f_list.cfg` order
/// (the order shown on the scanner). Missing/unreadable slot files yield zero counts
/// rather than failing the whole read.
pub fn read_favorites_lists(
    card_mount: &Path,
    profile: &dyn SdCardProfile,
) -> crate::Result<Vec<FavoriteListInfo>> {
    let cfg = f_list_path(card_mount, profile);
    let bytes = match fs::read(&cfg) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let doc = Document::parse(&bytes)?;

    let mut out = Vec::new();
    for line in &doc.lines {
        if line.command() != "F-List" {
            continue;
        }
        let name = line.field(1).unwrap_or("").to_string();
        let filename = line.field(2).unwrap_or("").to_string();
        let Some(slot) = slot_from_filename(&filename) else {
            continue;
        };
        let monitor = profile.f_list_monitor_field().and_then(|i| line.field(i)) == Some("On");
        let quick_key = parse_qk(profile.f_list_quick_key_field().and_then(|i| line.field(i)));
        let number_tag = parse_qk(
            profile
                .f_list_number_tag_field()
                .and_then(|i| line.field(i)),
        );
        let path = favorites_path(card_mount, profile, slot);
        let (systems, channels, fbytes) = match fs::read(&path) {
            Ok(fb) => {
                let len = fb.len() as u64;
                let (s, c) = Document::parse(&fb)
                    .map(|d| count_systems_channels(&d, profile))
                    .unwrap_or((0, 0));
                (s, c, len)
            }
            Err(_) => (0, 0, 0),
        };
        out.push(FavoriteListInfo {
            slot,
            name,
            filename,
            systems,
            channels,
            bytes: fbytes,
            monitor,
            quick_key,
            number_tag,
        });
    }
    Ok(out)
}

/// Remove a favorites list: delete its slot file, drop its `F-List` entry from
/// `f_list.cfg`, and delete `app_data.cfg` (the spec CRITICAL RULE). Idempotent —
/// a missing slot/entry is success. The caller must still **eject** the card.
pub fn delete_favorites_slot(
    card_mount: &Path,
    profile: &dyn SdCardProfile,
    slot: u32,
) -> crate::Result<()> {
    let fav = favorites_path(card_mount, profile, slot);
    match fs::remove_file(&fav) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    let path = f_list_path(card_mount, profile);
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(mut doc) = Document::parse(&bytes) {
            let filename = format!("f_{slot:06}.hpd");
            doc.lines
                .retain(|l| !(l.command() == "F-List" && l.field(2) == Some(filename.as_str())));
            write_synced(&path, &doc.to_bytes())?;
        }
    }
    delete_app_data(card_mount, profile)
}

/// Reorder the favorites lists **alphabetically by name** in `f_list.cfg` (the order
/// the scanner shows them) — the sort Sentinel never let you do. Header and any
/// non-`F-List` lines are preserved; deletes `app_data.cfg` (program data changed).
/// The caller must still **eject**.
pub fn sort_favorites_lists(card_mount: &Path, profile: &dyn SdCardProfile) -> crate::Result<()> {
    let path = f_list_path(card_mount, profile);
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let doc = Document::parse(&bytes)?;

    let (mut entries, others): (Vec<Line>, Vec<Line>) =
        doc.lines.into_iter().partition(|l| l.command() == "F-List");
    entries.sort_by_key(|l| l.field(1).unwrap_or("").to_lowercase());

    let mut lines = others; // header etc. stay at the top
    lines.extend(entries);
    write_synced(&path, &Document { lines }.to_bytes())?;
    delete_app_data(card_mount, profile)
}

/// Reorder the favorites lists into the given slot order (the scanner display order).
/// Slots not listed keep their relative order at the end; header preserved; deletes
/// `app_data.cfg`. The caller must still **eject**.
pub fn reorder_favorites_lists(
    card_mount: &Path,
    profile: &dyn SdCardProfile,
    slots: &[u32],
) -> crate::Result<()> {
    let path = f_list_path(card_mount, profile);
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let doc = Document::parse(&bytes)?;

    let (mut entries, others): (Vec<Line>, Vec<Line>) =
        doc.lines.into_iter().partition(|l| l.command() == "F-List");
    let mut ordered = Vec::with_capacity(entries.len());
    for slot in slots {
        let filename = format!("f_{slot:06}.hpd");
        if let Some(pos) = entries
            .iter()
            .position(|l| l.field(2) == Some(filename.as_str()))
        {
            ordered.push(entries.remove(pos));
        }
    }
    ordered.extend(entries); // any not named keep their original relative order

    let mut lines = others;
    lines.extend(ordered);
    write_synced(&path, &Document { lines }.to_bytes())?;
    delete_app_data(card_mount, profile)
}

/// `f_000005.hpd` → `5`.
fn slot_from_filename(filename: &str) -> Option<u32> {
    filename
        .strip_prefix("f_")
        .and_then(|s| s.strip_suffix(".hpd"))
        .and_then(|s| s.parse().ok())
}

/// (systems, voice channels) in a favorites/HPDB document.
fn count_systems_channels(doc: &Document, profile: &dyn SdCardProfile) -> (usize, usize) {
    let systems = Extraction::segment(doc, profile).system_count();
    let channels = doc
        .lines
        .iter()
        .filter(|l| extract::is_voice_channel(l.command()))
        .count();
    (systems, channels)
}

/// `<card>/<model_folder>` — e.g. `/Volumes/NO NAME/BCDx36HP`.
pub fn model_root(card_mount: &Path, profile: &dyn SdCardProfile) -> PathBuf {
    card_mount.join(profile.sd_layout().model_folder)
}

/// Path to a favorites slot file, e.g. `…/favorites_lists/f_000001.hpd`.
pub fn favorites_path(card_mount: &Path, profile: &dyn SdCardProfile, slot: u32) -> PathBuf {
    let layout = profile.sd_layout();
    model_root(card_mount, profile)
        .join(layout.favorites_dir)
        .join(format!("f_{slot:06}.hpd"))
}

/// Path to the resume-state file `app_data.cfg`.
pub fn app_data_path(card_mount: &Path, profile: &dyn SdCardProfile) -> PathBuf {
    model_root(card_mount, profile).join(profile.sd_layout().app_data_cfg)
}

/// Path to the favorites index `f_list.cfg`.
pub fn f_list_path(card_mount: &Path, profile: &dyn SdCardProfile) -> PathBuf {
    let layout = profile.sd_layout();
    model_root(card_mount, profile)
        .join(layout.favorites_dir)
        .join(layout.favorites_list_cfg)
}

/// Commit a favorites list to the card, the full safe write path:
/// 1. write `favorites_lists/f_<slot>.hpd`,
/// 2. register/replace its entry in `f_list.cfg` with `label`,
/// 3. delete `app_data.cfg` (the spec CRITICAL RULE).
///
/// `favorites` should come from [`crate::favorites::build_favorites`]. Writing a
/// **new** slot (not overwriting an existing one) is non-destructive to the user's
/// other lists — prefer that, and back up the card, until Phase B is done.
pub fn commit_favorites(
    card_mount: &Path,
    profile: &dyn SdCardProfile,
    slot: u32,
    label: &str,
    favorites: &Document,
) -> crate::Result<()> {
    write_favorites_slot(card_mount, profile, slot, favorites)?;
    update_f_list(card_mount, profile, slot, label)?;
    delete_app_data(card_mount, profile)?;
    Ok(())
}

/// Write **only** a favorites slot file `f_<slot>.hpd` (fsync'd) — no `f_list.cfg`
/// or `app_data.cfg` touch. The building block of the batched save: write each
/// changed list's content, then do one [`apply_favorites_layout`] pass for the
/// index + the `app_data` delete. The caller must still **eject**.
pub fn write_favorites_slot(
    card_mount: &Path,
    profile: &dyn SdCardProfile,
    slot: u32,
    favorites: &Document,
) -> crate::Result<()> {
    let fav_path = favorites_path(card_mount, profile, slot);
    if let Some(dir) = fav_path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_synced(&fav_path, &favorites.to_bytes())
}

/// Apply the full desired favorites layout in **one** structural pass: `lists`
/// becomes the complete, ordered set of `F-List` entries in `f_list.cfg`. Each
/// [`ListLayout`] carries the slot + display name and optional per-field overrides
/// (Monitor, quick key, number tag) — `None` leaves that field as-is. Any slot
/// registered in the *current* `f_list.cfg` but absent from `lists` has its slot
/// file removed (a delete). Ends by deleting `app_data.cfg` (the spec CRITICAL RULE).
///
/// **Preserves the existing F-List record verbatim** for every surviving list — only
/// the display name and the requested override fields change. The hardcoded
/// `f_list_entry` defaults are used **only for a brand-new slot**, so a rewrite never
/// disturbs fields we don't own (startup keys, S-Qkeys, …). Content slot files are
/// written separately first via [`write_favorites_slot`]; this does a single
/// `f_list.cfg` rewrite. Idempotent; the caller must **eject**.
pub fn apply_favorites_layout(
    card_mount: &Path,
    profile: &dyn SdCardProfile,
    lists: &[ListLayout],
) -> crate::Result<()> {
    let keep: std::collections::BTreeSet<u32> = lists.iter().map(|l| l.slot).collect();

    // Parse the existing index once: used both to delete dropped slot files and to
    // preserve each surviving list's full F-List field block.
    let path = f_list_path(card_mount, profile);
    let existing = fs::read(&path).ok().and_then(|b| Document::parse(&b).ok());

    if let Some(doc) = &existing {
        for line in &doc.lines {
            if line.command() != "F-List" {
                continue;
            }
            if let Some(slot) = line.field(2).and_then(slot_from_filename) {
                if !keep.contains(&slot) {
                    match fs::remove_file(favorites_path(card_mount, profile, slot)) {
                        Ok(()) => {}
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        }
    }

    // Rebuild f_list.cfg: model header, then one F-List per kept list, in order.
    let mut doc = new_index_doc(profile);
    for list in lists {
        let filename = format!("f_{:06}.hpd", list.slot);
        // Preserve the existing record (change only the label); synthesize the
        // default block only for a genuinely new list.
        let mut fields = existing
            .as_ref()
            .and_then(|d| f_list_fields_for(d, &filename))
            .map(|mut f| {
                if f.len() > 1 {
                    f[1] = list.name.clone();
                }
                f
            })
            .or_else(|| profile.f_list_entry(&list.name, &filename))
            .ok_or(crate::Error::Unsupported("model has no F-List format"))?;
        // Apply per-field overrides where requested and the model exposes the field.
        let set = |fields: &mut Vec<String>, col: Option<usize>, val: String| {
            if let Some(i) = col {
                if i < fields.len() {
                    fields[i] = val;
                }
            }
        };
        if let Some(b) = list.monitor {
            set(
                &mut fields,
                profile.f_list_monitor_field(),
                if b { "On" } else { "Off" }.to_string(),
            );
        }
        if let Some(v) = &list.quick_key {
            set(&mut fields, profile.f_list_quick_key_field(), v.clone());
        }
        if let Some(v) = &list.number_tag {
            set(&mut fields, profile.f_list_number_tag_field(), v.clone());
        }
        doc.lines.push(Line {
            fields,
            ending: LineEnding::Crlf,
        });
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_synced(&path, &doc.to_bytes())?;
    delete_app_data(card_mount, profile)
}

/// The existing `F-List` field vector for `filename` in a parsed index, if present —
/// so a rewrite can carry a list's record through verbatim.
fn f_list_fields_for(doc: &Document, filename: &str) -> Option<Vec<String>> {
    doc.lines
        .iter()
        .find(|l| l.command() == "F-List" && l.field(2) == Some(filename))
        .map(|l| l.fields.clone())
}

/// Write a file and `fsync` it. macOS buffers writes to FAT removable media, so a
/// plain `fs::write` returning `Ok` does NOT mean the bytes reached the card —
/// they can be lost (or left half-written, corrupting the FAT) if the card is
/// pulled / mass-storage mode is exited without ejecting. `fsync` flushes this
/// file; the caller must still **eject** the volume before disconnecting.
fn write_synced(path: &Path, bytes: &[u8]) -> crate::Result<()> {
    use std::io::Write;
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// Add or replace the `F-List` entry for `slot` in `f_list.cfg`, preserving the
/// header and any other lists. Creates the file (with header) if absent.
pub fn update_f_list(
    card_mount: &Path,
    profile: &dyn SdCardProfile,
    slot: u32,
    label: &str,
) -> crate::Result<()> {
    let path = f_list_path(card_mount, profile);
    let filename = format!("f_{slot:06}.hpd");

    let mut doc = match fs::read(&path) {
        Ok(bytes) => Document::parse(&bytes)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => new_index_doc(profile),
        Err(e) => return Err(e.into()),
    };

    match doc
        .lines
        .iter_mut()
        .find(|l| l.command() == "F-List" && l.field(2) == Some(filename.as_str()))
    {
        // Existing list: change only the display name; preserve every other field
        // (Monitor, quick key, number tag, startup keys…) verbatim.
        Some(existing) => {
            if existing.fields.len() > 1 {
                existing.fields[1] = label.to_string();
            }
        }
        // New list: synthesize the default block.
        None => {
            let fields = profile
                .f_list_entry(label, &filename)
                .ok_or(crate::Error::Unsupported("model has no F-List format"))?;
            doc.lines.push(Line {
                fields,
                ending: LineEnding::Crlf,
            });
        }
    }

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_synced(&path, &doc.to_bytes())
}

/// A fresh config document with just the model header sentences.
fn new_index_doc(profile: &dyn SdCardProfile) -> Document {
    let key = profile.model_key();
    let line = |fields: Vec<String>| Line {
        fields,
        ending: LineEnding::Crlf,
    };
    Document {
        lines: vec![
            line(vec!["TargetModel".to_string(), key.target_model]),
            line(vec!["FormatVersion".to_string(), key.format_version]),
        ],
    }
}

/// **Spec CRITICAL RULE.** Any tool that writes program data to the card must
/// delete `app_data.cfg`, or the scanner misbehaves on resume. Idempotent: a
/// missing file is success, so this is safe to call as the final step of every
/// write path.
pub fn delete_app_data(card_mount: &Path, profile: &dyn SdCardProfile) -> crate::Result<()> {
    let path = app_data_path(card_mount, profile);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Sds150;
    use std::path::Path;

    #[test]
    fn paths_use_profile_layout() {
        let p = Sds150::new();
        let root = Path::new("/Volumes/NO NAME");
        assert_eq!(
            favorites_path(root, &p, 1),
            Path::new("/Volumes/NO NAME/BCDx36HP/favorites_lists/f_000001.hpd")
        );
        assert_eq!(
            app_data_path(root, &p),
            Path::new("/Volumes/NO NAME/BCDx36HP/app_data.cfg")
        );
    }

    #[test]
    fn delete_app_data_is_idempotent() {
        let p = Sds150::new();
        // Build a throwaway fake card: <tmp>/BCDx36HP/app_data.cfg
        let base = std::env::temp_dir().join(format!("platypus-card-{}", std::process::id()));
        let model = base.join("BCDx36HP");
        fs::create_dir_all(&model).unwrap();
        let app_data = model.join("app_data.cfg");
        fs::write(&app_data, b"ModeInfo\tIDscan\r\n").unwrap();
        assert!(app_data.exists());

        // First delete removes it; second is still Ok (idempotent).
        delete_app_data(&base, &p).unwrap();
        assert!(!app_data.exists());
        delete_app_data(&base, &p).unwrap();

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn sort_and_reorder_favorites_lists() {
        let p = Sds150::new();
        let base = std::env::temp_dir().join(format!("platypus-order-{}", std::process::id()));
        fs::create_dir_all(base.join("BCDx36HP")).unwrap();
        let doc = Document {
            lines: vec![Line {
                fields: vec!["TargetModel".into(), "BCDx36HP".into()],
                ending: LineEnding::Crlf,
            }],
        };
        commit_favorites(&base, &p, 1, "Charlie", &doc).unwrap();
        commit_favorites(&base, &p, 2, "Alpha", &doc).unwrap();
        commit_favorites(&base, &p, 3, "Bravo", &doc).unwrap();

        // Alphabetize → Alpha, Bravo, Charlie.
        sort_favorites_lists(&base, &p).unwrap();
        let names: Vec<String> = read_favorites_lists(&base, &p)
            .unwrap()
            .into_iter()
            .map(|l| l.name)
            .collect();
        assert_eq!(names, ["Alpha", "Bravo", "Charlie"]);

        // Explicit reorder by slot → Charlie(1), Bravo(3), Alpha(2).
        reorder_favorites_lists(&base, &p, &[1, 3, 2]).unwrap();
        let order: Vec<u32> = read_favorites_lists(&base, &p)
            .unwrap()
            .into_iter()
            .map(|l| l.slot)
            .collect();
        assert_eq!(order, [1, 3, 2]);

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn apply_layout_batches_delete_rename_reorder() {
        let p = Sds150::new();
        let base = std::env::temp_dir().join(format!("platypus-layout-{}", std::process::id()));
        fs::create_dir_all(base.join("BCDx36HP")).unwrap();
        let doc = Document {
            lines: vec![Line {
                fields: vec!["TargetModel".into(), "BCDx36HP".into()],
                ending: LineEnding::Crlf,
            }],
        };
        commit_favorites(&base, &p, 1, "Alpha", &doc).unwrap();
        commit_favorites(&base, &p, 2, "Bravo", &doc).unwrap();
        commit_favorites(&base, &p, 3, "Charlie", &doc).unwrap();
        // app_data must be re-deleted by the layout pass.
        fs::write(app_data_path(&base, &p), b"x").unwrap();

        // One pass: drop slot 2, rename slot 1, order = [3, 1].
        apply_favorites_layout(
            &base,
            &p,
            &[
                ListLayout::new(3, "Charlie"),
                ListLayout::new(1, "Alpha Renamed"),
            ],
        )
        .unwrap();

        let lists = read_favorites_lists(&base, &p).unwrap();
        assert_eq!(
            lists.iter().map(|l| l.slot).collect::<Vec<_>>(),
            [3, 1],
            "order follows the layout"
        );
        assert_eq!(lists[1].name, "Alpha Renamed", "rename applied");
        assert!(
            !favorites_path(&base, &p, 2).exists(),
            "dropped slot file removed"
        );
        assert!(
            favorites_path(&base, &p, 1).exists(),
            "kept slot file intact"
        );
        assert!(!app_data_path(&base, &p).exists(), "app_data deleted");

        fs::remove_dir_all(&base).ok();
    }

    /// Build an `F-List` line with a NON-default flag block: Monitor (field 4) and
    /// Quick key (field 5) set to distinctive values, the rest `Off`. Mirrors a
    /// user-configured list whose settings must survive a rewrite.
    fn f_list_line(name: &str, filename: &str, monitor: &str, quick_key: &str) -> Line {
        let mut fields = vec![
            "F-List".to_string(),
            name.to_string(),
            filename.to_string(),
            "Off".to_string(),     // 3 LocationControl
            monitor.to_string(),   // 4 Monitor
            quick_key.to_string(), // 5 Quick key
        ];
        fields.resize(118, "Off".to_string());
        Line {
            fields,
            ending: LineEnding::Crlf,
        }
    }

    /// The `F-List` line for `filename` in a written `f_list.cfg`, parsed back.
    fn read_f_list_line(base: &Path, p: &Sds150, filename: &str) -> Line {
        let bytes = fs::read(f_list_path(base, p)).unwrap();
        let doc = Document::parse(&bytes).unwrap();
        doc.lines
            .into_iter()
            .find(|l| l.command() == "F-List" && l.field(2) == Some(filename))
            .expect("F-List entry present")
    }

    #[test]
    fn apply_layout_preserves_unknown_fields_and_sets_monitor() {
        let p = Sds150::new();
        let base = std::env::temp_dir().join(format!("platypus-preserve-{}", std::process::id()));
        fs::create_dir_all(base.join("BCDx36HP").join("favorites_lists")).unwrap();

        // Seed f_list.cfg: one list (slot 5) with Monitor=Off and Quick key=7.
        let seed = Document {
            lines: vec![
                Line {
                    fields: vec!["TargetModel".into(), "BCDx36HP".into()],
                    ending: LineEnding::Crlf,
                },
                f_list_line("Original", "f_000005.hpd", "Off", "7"),
            ],
        };
        fs::write(f_list_path(&base, &p), seed.to_bytes()).unwrap();

        // Rewrite with a rename and NO overrides → everything but the name is
        // carried through verbatim.
        apply_favorites_layout(&base, &p, &[ListLayout::new(5, "Renamed")]).unwrap();
        let line = read_f_list_line(&base, &p, "f_000005.hpd");
        assert_eq!(line.field(1), Some("Renamed"), "label updated");
        assert_eq!(line.field(4), Some("Off"), "Monitor preserved");
        assert_eq!(line.field(5), Some("7"), "Quick key preserved");
        assert_eq!(line.fields.len(), 118, "field count preserved");
        let info = &read_favorites_lists(&base, &p).unwrap()[0];
        assert!(!info.monitor);
        assert_eq!(info.quick_key, Some(7), "quick key read");
        assert_eq!(info.number_tag, None, "number tag Off → None");

        // Turn Monitor on → field 4 flips, Quick key still preserved.
        apply_favorites_layout(
            &base,
            &p,
            &[ListLayout {
                monitor: Some(true),
                ..ListLayout::new(5, "Renamed")
            }],
        )
        .unwrap();
        let line = read_f_list_line(&base, &p, "f_000005.hpd");
        assert_eq!(line.field(4), Some("On"), "Monitor set on");
        assert_eq!(line.field(5), Some("7"), "Quick key still preserved");
        assert!(read_favorites_lists(&base, &p).unwrap()[0].monitor);

        // Set quick key + number tag; Monitor left as-is (still On).
        apply_favorites_layout(
            &base,
            &p,
            &[ListLayout {
                quick_key: Some("3".to_string()),
                number_tag: Some("12".to_string()),
                ..ListLayout::new(5, "Renamed")
            }],
        )
        .unwrap();
        let line = read_f_list_line(&base, &p, "f_000005.hpd");
        assert_eq!(
            line.field(4),
            Some("On"),
            "Monitor preserved through qk/nt set"
        );
        assert_eq!(line.field(5), Some("3"), "quick key set");
        assert_eq!(line.field(6), Some("12"), "number tag set");
        let info = &read_favorites_lists(&base, &p).unwrap()[0];
        assert_eq!(info.quick_key, Some(3));
        assert_eq!(info.number_tag, Some(12));

        // Clear quick key back to Off.
        apply_favorites_layout(
            &base,
            &p,
            &[ListLayout {
                quick_key: Some("Off".to_string()),
                ..ListLayout::new(5, "Renamed")
            }],
        )
        .unwrap();
        assert_eq!(
            read_favorites_lists(&base, &p).unwrap()[0].quick_key,
            None,
            "quick key cleared to Off"
        );

        // Turn Monitor off again.
        apply_favorites_layout(
            &base,
            &p,
            &[ListLayout {
                monitor: Some(false),
                ..ListLayout::new(5, "Renamed")
            }],
        )
        .unwrap();
        assert_eq!(
            read_f_list_line(&base, &p, "f_000005.hpd").field(4),
            Some("Off")
        );

        // A brand-new slot (8) with no prior entry gets the default block.
        apply_favorites_layout(
            &base,
            &p,
            &[ListLayout::new(5, "Renamed"), ListLayout::new(8, "New")],
        )
        .unwrap();
        let new = read_f_list_line(&base, &p, "f_000008.hpd");
        assert_eq!(new.field(4), Some("On"), "new list default Monitor=On");
        assert_eq!(new.field(5), Some("0"), "new list default Quick key=0");
        // The existing list (5) is preserved through the same pass (its quick key was
        // last cleared to Off; number tag stays 12 — a no-override entry touches neither).
        let l5 = read_f_list_line(&base, &p, "f_000005.hpd");
        assert_eq!(l5.field(5), Some("Off"), "list 5 quick key preserved");
        assert_eq!(l5.field(6), Some("12"), "list 5 number tag preserved");

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn update_f_list_preserves_existing_fields() {
        let p = Sds150::new();
        let base =
            std::env::temp_dir().join(format!("platypus-updateflist-{}", std::process::id()));
        fs::create_dir_all(base.join("BCDx36HP").join("favorites_lists")).unwrap();
        let seed = Document {
            lines: vec![
                Line {
                    fields: vec!["TargetModel".into(), "BCDx36HP".into()],
                    ending: LineEnding::Crlf,
                },
                f_list_line("Original", "f_000005.hpd", "Off", "7"),
            ],
        };
        fs::write(f_list_path(&base, &p), seed.to_bytes()).unwrap();

        update_f_list(&base, &p, 5, "Renamed").unwrap();
        let line = read_f_list_line(&base, &p, "f_000005.hpd");
        assert_eq!(line.field(1), Some("Renamed"), "label updated");
        assert_eq!(line.field(4), Some("Off"), "Monitor preserved");
        assert_eq!(line.field(5), Some("7"), "Quick key preserved");

        // A new slot still gets the default block.
        update_f_list(&base, &p, 9, "Fresh").unwrap();
        let fresh = read_f_list_line(&base, &p, "f_000009.hpd");
        assert_eq!(fresh.field(4), Some("On"), "new list default Monitor=On");

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn read_and_delete_favorites_lists() {
        let p = Sds150::new();
        let base = std::env::temp_dir().join(format!("platypus-flist-{}", std::process::id()));
        let model = base.join("BCDx36HP");
        fs::create_dir_all(&model).unwrap();

        // Two favorites lists: one Conventional system w/ a C-Freq, one empty-ish.
        let one_system = Document {
            lines: vec![
                Line {
                    fields: vec!["TargetModel".into(), "BCDx36HP".into()],
                    ending: LineEnding::Crlf,
                },
                Line {
                    fields: vec!["Conventional".into(), "".into(), "".into(), "Sys".into()],
                    ending: LineEnding::Crlf,
                },
                Line {
                    fields: vec![
                        "C-Freq".into(),
                        "".into(),
                        "".into(),
                        "Ch".into(),
                        "Off".into(),
                        "154000000".into(),
                    ],
                    ending: LineEnding::Crlf,
                },
            ],
        };
        commit_favorites(&base, &p, 1, "Alpha", &one_system).unwrap();
        commit_favorites(&base, &p, 2, "Bravo", &one_system).unwrap();

        let lists = read_favorites_lists(&base, &p).unwrap();
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0].name, "Alpha");
        assert_eq!(lists[0].slot, 1);
        assert_eq!(lists[0].systems, 1);
        assert_eq!(lists[0].channels, 1);
        assert!(lists[0].bytes > 0);

        // Delete slot 1 → file gone, entry gone, only Bravo remains.
        delete_favorites_slot(&base, &p, 1).unwrap();
        assert!(!favorites_path(&base, &p, 1).exists());
        let after = read_favorites_lists(&base, &p).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "Bravo");

        // Idempotent.
        delete_favorites_slot(&base, &p, 1).unwrap();

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn corrupt_f_list_surfaces_parse_error() {
        // A non-round-tripping f_list.cfg (a byte the ASCII contract forbids) must
        // surface the parse error, not be silently accepted as an empty list. With
        // the unified error type the real `NonAscii` now propagates (previously it
        // was flattened into an io::ErrorKind::InvalidData).
        let p = Sds150::new();
        let base = std::env::temp_dir().join(format!("platypus-corrupt-{}", std::process::id()));
        let dir = f_list_path(&base, &p).parent().unwrap().to_path_buf();
        fs::create_dir_all(&dir).unwrap();
        // 0xFF is outside printable ASCII, so Document::parse rejects it.
        fs::write(f_list_path(&base, &p), b"F-List\t\xFF\r\n").unwrap();

        let err = read_favorites_lists(&base, &p).unwrap_err();
        assert!(
            matches!(err, crate::Error::NonAscii { byte: 0xFF, .. }),
            "expected NonAscii, got {err:?}"
        );

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn commit_writes_slot_registers_it_and_deletes_app_data() {
        let p = Sds150::new();
        let base = std::env::temp_dir().join(format!("platypus-commit-{}", std::process::id()));
        let model = base.join("BCDx36HP");
        fs::create_dir_all(&model).unwrap();
        // a pre-existing resume file that must be deleted
        fs::write(model.join("app_data.cfg"), b"x").unwrap();

        let favs = Document {
            lines: vec![Line {
                fields: vec!["TargetModel".to_string(), "BCDx36HP".to_string()],
                ending: LineEnding::Crlf,
            }],
        };

        commit_favorites(&base, &p, 5, "Test List", &favs).unwrap();

        // 1. slot file written, byte-exact
        let fp = favorites_path(&base, &p, 5);
        assert!(fp.exists());
        assert_eq!(fs::read(&fp).unwrap(), favs.to_bytes());
        // 2. registered in f_list.cfg
        let flist = String::from_utf8(fs::read(f_list_path(&base, &p)).unwrap()).unwrap();
        assert!(flist.contains("F-List\tTest List\tf_000005.hpd"));
        // 3. app_data deleted
        assert!(!app_data_path(&base, &p).exists());

        // Re-commit replaces (no duplicate entry).
        commit_favorites(&base, &p, 5, "Renamed", &favs).unwrap();
        let flist2 = String::from_utf8(fs::read(f_list_path(&base, &p)).unwrap()).unwrap();
        assert_eq!(flist2.matches("f_000005.hpd").count(), 1);
        assert!(flist2.contains("Renamed"));

        fs::remove_dir_all(&base).ok();
    }
}
