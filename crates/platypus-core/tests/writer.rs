// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Phase-A writer pipeline, end to end against a temp "card": filter a real-shaped
//! selection → build a complete favorites doc → commit (slot file + f_list.cfg +
//! delete app_data). Uses the synthetic fixture; no hardware, no real location.

use std::fs;
use std::path::{Path, PathBuf};

use platypus_core::device::Sds150;
use platypus_core::format::Document;
use platypus_core::{card, extract, favorites};

const ALPHA: u64 = 9001; // synthetic county tagged by a conventional + the P25 trunk

fn synthetic_state() -> Document {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/synthetic/s_000090.hpd");
    Document::parse(&fs::read(path).unwrap()).unwrap()
}

fn temp_card() -> PathBuf {
    let base = std::env::temp_dir().join(format!("platypus-writer-{}", std::process::id()));
    fs::create_dir_all(base.join("BCDx36HP")).unwrap();
    base
}

#[test]
fn filter_build_and_commit_round_trips_onto_a_card() {
    let profile = Sds150::new();
    let state = synthetic_state();

    // Filter: just county 9001 (a conventional system + the P25 trunk).
    let selection = extract::by_county(&state, &profile, ALPHA);
    assert_eq!(
        extract::Extraction::segment(&selection, &profile).system_count(),
        2
    );

    // Build a complete favorites document.
    let favs = favorites::build_favorites(&selection, &profile, true);
    // One DQKs per selected system; a band plan for the P25 trunk's sites.
    assert_eq!(
        favs.lines
            .iter()
            .filter(|l| l.command() == "DQKs_Status")
            .count(),
        2
    );
    // No band plan is synthesized (adding one breaks trunk lock — device-confirmed).
    assert!(!favs.lines.iter().any(|l| l.command() == "BandPlan_P25"));
    assert!(!favs
        .lines
        .iter()
        .any(|l| matches!(l.command(), "AreaState" | "AreaCounty")));
    assert!(favorites::has_synthesized_records(&favs, &profile));
    // The thing we commit must itself round-trip byte-for-byte.
    assert_eq!(
        favs.to_bytes(),
        Document::parse(&favs.to_bytes()).unwrap().to_bytes()
    );

    // Commit to a throwaway card and verify the three effects.
    let base = temp_card();
    fs::write(base.join("BCDx36HP/app_data.cfg"), b"resume").unwrap();
    card::commit_favorites(&base, &profile, 7, "Alpha County", &favs).unwrap();

    let slot = card::favorites_path(&base, &profile, 7);
    assert_eq!(fs::read(&slot).unwrap(), favs.to_bytes());

    let written = Document::parse(&fs::read(&slot).unwrap()).unwrap();
    assert_eq!(written, favs); // parsed-back equals what we built

    let flist = String::from_utf8(fs::read(card::f_list_path(&base, &profile)).unwrap()).unwrap();
    assert!(flist.contains("f_000007.hpd") && flist.contains("Alpha County"));
    assert!(!card::app_data_path(&base, &profile).exists());

    fs::remove_dir_all(&base).ok();
}
