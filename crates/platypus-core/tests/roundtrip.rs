// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! The core safety net, in Rust: every fixture file under `samples/` (the committed
//! synthetic set under `samples/synthetic/`, plus any real card dumps a developer keeps
//! locally) must parse and serialize **byte-for-byte**. This is the gate CLAUDE.md requires
//! before any writer is trusted.
//!
//! Mirrors `roundtrip_hpd.py`; if either drifts, this test fails.

use std::fs;
use std::path::{Path, PathBuf};

use platypus_core::device::ProfileRegistry;
use platypus_core::format::Document;

/// Repo-root `samples/` (two levels up from this crate) — the committed synthetic
/// fixtures, plus any local real card dumps.
fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../samples")
        .canonicalize()
        .expect("samples/ must exist")
}

fn card_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            card_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("hpd") | Some("cfg") | Some("avd")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn every_sample_round_trips_byte_for_byte() {
    let mut files = Vec::new();
    card_files(&samples_dir(), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no fixture files found");

    for path in &files {
        let raw = fs::read(path).unwrap();
        let doc = Document::parse(&raw)
            .unwrap_or_else(|e| panic!("parse failed for {}: {e}", path.display()));
        assert_eq!(
            doc.to_bytes(),
            raw,
            "round-trip mismatch for {}",
            path.display()
        );
    }

    eprintln!("round-trip: {}/{} clean", files.len(), files.len());
}

#[test]
fn profile_detects_sds150_from_samples() {
    let reg = ProfileRegistry::with_builtins();
    let mut files = Vec::new();
    card_files(&samples_dir(), &mut files);

    for path in &files {
        let raw = fs::read(path).unwrap();
        let header = Document::parse(&raw).unwrap().header();
        let profile = reg
            .detect(&header)
            .unwrap_or_else(|| panic!("no profile matched {}", path.display()));
        assert_eq!(profile.product_name(), "SDS150");
    }
}

#[test]
fn favorites_blank_ids_but_state_files_fill_them() {
    // The dialect rule, on synthetic fixtures: in a state HPDB file the first Site
    // carries SiteId=/TrunkId=; in a favorites file those columns are blank — same
    // record, same column count.
    let base = samples_dir().join("synthetic");

    let state = Document::parse(&fs::read(base.join("s_000090.hpd")).unwrap()).unwrap();
    let state_site = state
        .lines
        .iter()
        .find(|l| l.command() == "Site")
        .expect("state file has a Site");
    assert!(
        state_site.field(1).unwrap().starts_with("SiteId="),
        "state Site col 1 should be SiteId=, got {:?}",
        state_site.field(1)
    );
    assert!(state_site.field(2).unwrap().starts_with("TrunkId="));

    let fav = Document::parse(&fs::read(base.join("f_example.hpd")).unwrap()).unwrap();
    let fav_site = fav
        .lines
        .iter()
        .find(|l| l.command() == "Site")
        .expect("favorites file has a Site");
    assert_eq!(fav_site.field(1), Some(""), "favorites Site col 1 blank");
    assert_eq!(fav_site.field(2), Some(""), "favorites Site col 2 blank");

    // Same column count in both dialects (the blank-not-drop rule).
    assert_eq!(state_site.fields.len(), fav_site.fields.len());
}
