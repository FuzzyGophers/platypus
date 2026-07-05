// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Location-first primitives — radius filtering, favorites-dialect geo, and the
//! county index — all exercised against the synthetic fictional fixtures, so the test
//! code carries no real location. All read-only.

use std::fs;
use std::path::{Path, PathBuf};

use platypus_core::device::Sds150;
use platypus_core::format::Document;
use platypus_core::model::{self, CountyIndex};

fn sample(group: &str, rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../samples")
        .join(group)
        .join(rel)
}

fn parse(group: &str, rel: &str) -> Document {
    Document::parse(&fs::read(sample(group, rel)).unwrap()).unwrap()
}

#[test]
fn radius_filter_finds_nearby_sites() {
    let doc = parse("synthetic", "s_000090.hpd");
    let profile = Sds150::new();

    // Center on the synthetic "Central Site" (45.5, -100.5).
    let (lat, lon) = (45.5, -100.5);
    let hits = model::within_radius(&doc, &profile, lat, lon, 25.0);

    assert!(!hits.is_empty(), "expected sites within 25 mi");
    for r in &hits {
        let g = r.geo().unwrap();
        assert!(model::haversine_miles(lat, lon, g.lat, g.lon) <= 25.0);
    }
}

#[test]
fn radius_filter_excludes_far_points() {
    let doc = parse("synthetic", "s_000090.hpd");
    let profile = Sds150::new();
    let hits = model::within_radius(&doc, &profile, 0.0, -160.0, 10.0); // mid-Pacific
    assert!(hits.is_empty());
}

#[test]
fn radius_filter_works_on_favorites_dialect() {
    // Favorites blank the id columns but keep geo — the filter still works.
    let doc = parse("synthetic", "f_example.hpd");
    let profile = Sds150::new();
    let (lat, lon) = (45.5, -100.5); // the synthetic hub site

    let hits = model::within_radius(&doc, &profile, lat, lon, 10.0);
    assert!(!hits.is_empty(), "expected a site within 10 mi of the hub");
    for r in &hits {
        let g = r.geo().unwrap();
        assert!(model::haversine_miles(lat, lon, g.lat, g.lon) <= 10.0);
        assert!(r.id().is_none(), "favorites records have blanked ids");
    }
}

#[test]
fn county_index_maps_names_and_ids() {
    let doc = parse("synthetic", "hpdb.cfg");
    let index = CountyIndex::from_hpdb(&doc, &Sds150::new());

    assert_eq!(index.len(), 3);
    assert_eq!(index.id_by_name("Alpha"), Some(9001));
    assert_eq!(index.name(9001), Some("Alpha"));
    // Case-insensitive lookup.
    assert!(index.counties_named("bravo").iter().any(|c| c.id == 9002));
}
