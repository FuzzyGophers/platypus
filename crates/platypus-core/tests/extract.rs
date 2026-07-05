// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! County / radius extraction + favorites-dialect conversion, asserted against a
//! synthetic, fictional fixture (`samples/synthetic/s_000090.hpd`) so the test
//! code carries no real location. Real card files are exercised separately by the
//! byte-exact round-trip gate. Extracted documents must still round-trip.

use std::fs;
use std::path::{Path, PathBuf};

use platypus_core::device::Sds150;
use platypus_core::extract::{self, Extraction};
use platypus_core::favorites::{self, has_synthesized_records};
use platypus_core::format::Document;

// Fictional ids from samples/synthetic/generate.py.
const STATE: u64 = 90;
const ALPHA: u64 = 9001; // county; tagged by 2 systems
const BRAVO: u64 = 9002; // county; tagged by 2 systems
const CEDAR: u64 = 9003; // county; tagged by 1 system
const AGENCY: u64 = 9101; // an AgencyId — NOT a county
const TOTAL_SYSTEMS: usize = 4;

fn synthetic(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../samples/synthetic")
        .join(rel)
}

fn state_file() -> Document {
    Document::parse(&fs::read(synthetic("s_000090.hpd")).unwrap()).unwrap()
}

#[test]
fn segments_all_systems() {
    let doc = state_file();
    let profile = Sds150::new();
    let ext = Extraction::segment(&doc, &profile);

    assert_eq!(ext.system_count(), TOTAL_SYSTEMS);
    let pre = ext.preamble_lines();
    assert_eq!(pre[0].command(), "TargetModel");
    assert_eq!(pre[1].command(), "FormatVersion");
    for sys in ext.systems() {
        assert!(matches!(sys.header().command(), "Conventional" | "Trunk"));
    }
}

#[test]
fn extract_by_county() {
    let doc = state_file();
    let profile = Sds150::new();

    let picked = extract::by_county(&doc, &profile, ALPHA);
    let ext = Extraction::segment(&picked, &profile);
    assert_eq!(ext.system_count(), 2);
    for sys in ext.systems() {
        assert!(
            sys.is_in_county(ALPHA),
            "system {:?} not in county",
            sys.name()
        );
    }
    assert!(ext.system_count() < TOTAL_SYSTEMS);

    // Extraction is loss-free.
    assert_eq!(
        picked.to_bytes(),
        Document::parse(&picked.to_bytes()).unwrap().to_bytes()
    );

    // Other counties select their own systems.
    assert_eq!(
        Extraction::segment(&extract::by_county(&doc, &profile, BRAVO), &profile).system_count(),
        2
    );
    assert_eq!(
        Extraction::segment(&extract::by_county(&doc, &profile, CEDAR), &profile).system_count(),
        1
    );
}

#[test]
fn county_tag_is_field_two_not_owner_id() {
    // The key semantic: AreaCounty is [owner-id, county-tag]. An AgencyId in
    // field 1 must NOT be treated as a county. (One system is agency-organized.)
    let doc = state_file();
    let profile = Sds150::new();
    let by_agency_id = extract::by_county(&doc, &profile, AGENCY);
    assert_eq!(
        Extraction::segment(&by_agency_id, &profile).system_count(),
        0,
        "an AgencyId must not match as a county"
    );
}

#[test]
fn select_is_generic_over_predicate() {
    // The engine hard-codes no filter: any predicate over System metadata works.
    let doc = state_file();
    let profile = Sds150::new();
    let ext = Extraction::segment(&doc, &profile);

    let in_state = ext.select(|s| s.is_in_state(STATE));
    assert_eq!(
        Extraction::segment(&in_state, &profile).system_count(),
        TOTAL_SYSTEMS
    );

    // A compound predicate (state AND has a location) is just as easy.
    let with_geo = ext.select(|s| s.is_in_state(STATE) && s.geos().next().is_some());
    assert!(Extraction::segment(&with_geo, &profile).system_count() <= TOTAL_SYSTEMS);
}

#[test]
fn extract_within_radius() {
    let doc = state_file();
    let profile = Sds150::new();
    // The hub site sits at (45.5, -100.5); other systems are ~40+ mi away, the
    // "Far Site" hundreds.
    let near = extract::within_radius(&doc, &profile, 45.5, -100.5, 15.0);
    let wide = extract::within_radius(&doc, &profile, 45.5, -100.5, 60.0);

    let near_n = Extraction::segment(&near, &profile).system_count();
    let wide_n = Extraction::segment(&wide, &profile).system_count();
    assert_eq!(near_n, 1, "only the hub system is within 15 mi");
    assert_eq!(wide_n, TOTAL_SYSTEMS, "all systems within 60 mi");
    assert!(near_n < wide_n);
}

#[test]
fn site_radius_filter_keeps_only_nearby_sites() {
    // The P25 trunk has a hub site (45.5,-100.5) and a "Far Site" (48,-110).
    // Filtering sites within 15 mi of the hub keeps the hub, drops the far ones —
    // the site-level location-first selection (no hand-deselecting).
    let doc = state_file();
    let profile = Sds150::new();
    let filtered = extract::filter_within_radius(&doc, &profile, 45.5, -100.5, 15.0);

    let sites: Vec<_> = filtered
        .lines
        .iter()
        .filter(|l| l.command() == "Site")
        .filter_map(|l| l.field(3))
        .collect();
    assert!(sites.contains(&"Central Site"));
    assert!(!sites.contains(&"Far Site"));
    assert!(!sites.contains(&"Business Site")); // ~42 mi away
    assert_eq!(
        filtered.to_bytes(),
        Document::parse(&filtered.to_bytes()).unwrap().to_bytes()
    );
}

#[test]
fn favorites_conversion_of_extraction() {
    let doc = state_file();
    let profile = Sds150::new();

    let picked = extract::by_county(&doc, &profile, ALPHA);
    let fav = favorites::to_favorites_dialect(&picked, &profile);

    for line in &fav.lines {
        assert!(!matches!(line.command(), "AreaState" | "AreaCounty"));
    }
    let first_site = fav.lines.iter().find(|l| l.command() == "Site").unwrap();
    assert_eq!(first_site.field(1), Some(""));
    assert_eq!(first_site.field(2), Some(""));

    assert_eq!(
        fav.to_bytes(),
        Document::parse(&fav.to_bytes()).unwrap().to_bytes()
    );
    // to_favorites_dialect alone doesn't add the synthesized records.
    assert!(!has_synthesized_records(&fav, &profile));
}

#[test]
fn favorites_preserves_rectangle_records() {
    // Rectangle is a real HPDB record (for `Rectangles`-shaped groups), not a
    // synthesized favorites-only one — it must survive extraction + conversion.
    let doc = state_file();
    let profile = Sds150::new();
    let ext = Extraction::segment(&doc, &profile);

    let with_rect = ext.select(|s| s.lines().iter().any(|l| l.command() == "Rectangle"));
    let before = with_rect
        .lines
        .iter()
        .filter(|l| l.command() == "Rectangle")
        .count();
    assert!(before > 0, "fixture has a Rectangles-shaped system");

    let fav = favorites::to_favorites_dialect(&with_rect, &profile);
    let after = fav
        .lines
        .iter()
        .filter(|l| l.command() == "Rectangle")
        .count();
    assert_eq!(before, after, "Rectangle records must survive conversion");
    assert_eq!(
        fav.to_bytes(),
        Document::parse(&fav.to_bytes()).unwrap().to_bytes()
    );
}

#[test]
fn synthesizes_dqks_for_extracted_systems() {
    let doc = state_file();
    let profile = Sds150::new();
    let picked = extract::by_county(&doc, &profile, ALPHA); // 2 systems
    let fav = favorites::to_favorites_dialect(&picked, &profile);
    let built = favorites::with_synthesized_dqks(&fav, &profile, true);

    let dqks = built
        .lines
        .iter()
        .filter(|l| l.command() == "DQKs_Status")
        .count();
    assert_eq!(dqks, 2);
    assert_eq!(
        built.to_bytes(),
        Document::parse(&built.to_bytes()).unwrap().to_bytes()
    );
}
