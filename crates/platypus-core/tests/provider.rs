// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! The provider seam + canonical model: an HPDB card loads into a source-agnostic
//! `Dataset` that filters operate on, regardless of where the data came from.
//! Asserted against the synthetic fixture (no real location in test code).

use std::fs;
use std::path::Path;

use platypus_core::format::Document;
use platypus_core::model::service_type_name;
use platypus_core::provider::{ChannelKind, Dataset, HpdbProvider, Provider, SystemKind};
use platypus_core::Error;

fn load(rel: &str) -> Dataset {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../samples")
        .join(rel);
    let doc = Document::parse(&fs::read(path).unwrap()).unwrap();
    HpdbProvider::from_document(doc, "card").load().unwrap()
}

fn load_synthetic() -> Dataset {
    load("synthetic/s_000090.hpd")
}

#[test]
fn unknown_model_header_is_rejected() {
    // A header whose TargetModel no registered profile claims must fail loudly
    // (Err(UnknownModel)) rather than panic or return an empty Ok dataset.
    let doc = Document::parse(b"TargetModel\tNOSUCHMODEL\r\nFormatVersion\t9.99\r\n").unwrap();
    let err = HpdbProvider::from_document(doc, "unknown card")
        .load()
        .unwrap_err();
    assert_eq!(err, Error::UnknownModel);
}

#[test]
fn hpdb_provider_yields_canonical_systems() {
    let data = load_synthetic();
    assert_eq!(data.len(), 4);

    // The P25 trunk is a multi-county system with several geo locations.
    let trunk = data
        .systems
        .iter()
        .find(|s| s.kind == SystemKind::Trunk && s.name.contains("P25"))
        .expect("a P25 trunk");
    assert!(trunk.county_ids.contains(&9001) && trunk.county_ids.contains(&9003));
    assert!(trunk.state_ids.contains(&90));
    assert!(trunk.locations.iter().any(|l| l.name == "Central Site"));
}

#[test]
fn dataset_select_is_generic() {
    let data = load_synthetic();
    // The same generic predicate engine as the HPDB layer, now on canonical data.
    assert_eq!(data.select(|s| s.is_in_county(9001)).len(), 2);
    assert_eq!(data.select(|s| s.is_in_county(9003)).len(), 1);
    assert_eq!(data.select(|s| s.is_in_state(90)).len(), 4);
    // An agency id is not a county.
    assert_eq!(data.select(|s| s.is_in_county(9101)).len(), 0);
    // Compound predicate, no special-casing.
    assert_eq!(
        data.select(|s| s.kind == SystemKind::Trunk && !s.locations.is_empty())
            .len(),
        2
    );
}

#[test]
fn rich_attributes_decode() {
    let data = load_synthetic();

    // Tech is decoded from the system header.
    let p25 = data
        .systems
        .iter()
        .find(|s| s.tech_is("P25Standard"))
        .unwrap();
    assert_eq!(p25.kind, SystemKind::Trunk);
    assert!(data.systems.iter().any(|s| s.tech_is("MotoTrbo")));
    assert!(data.systems.iter().any(|s| s.tech_is("Conventional")));

    // A conventional channel: frequency, mode, tone, service type.
    let conv = data
        .systems
        .iter()
        .find(|s| s.tech_is("Conventional"))
        .unwrap();
    let ch = conv
        .channels
        .iter()
        .find(|c| c.name == "Alpha Dispatch")
        .unwrap();
    assert_eq!(ch.kind, ChannelKind::Frequency);
    assert_eq!(ch.freq_hz, Some(154_100_000));
    assert_eq!(ch.mode.as_deref(), Some("NFM"));
    assert_eq!(ch.tone.as_deref(), Some("C156.7")); // TONE= stripped
    assert_eq!(ch.service_type, Some(3));

    // A trunked talkgroup: tgid, audio mode, service type.
    let tg = p25.channels.iter().find(|c| c.name == "Police").unwrap();
    assert_eq!(tg.kind, ChannelKind::Talkgroup);
    assert_eq!(tg.tgid.as_deref(), Some("101"));
    assert_eq!(tg.mode.as_deref(), Some("DIGITAL"));
    assert_eq!(tg.service_type, Some(2));
}

#[test]
fn service_type_filtering_and_names() {
    let data = load_synthetic();

    // Aggregated per system, so "give me systems with any fire-dispatch channel".
    assert_eq!(data.select(|s| s.has_service_type(3)).len(), 1); // Fire Dispatch
    assert_eq!(data.select(|s| s.has_service_type(2)).len(), 1); // Law Dispatch
                                                                 // A system with a law OR fire-tac channel (the P25 trunk has both 2 and 8).
    assert_eq!(
        data.select(|s| s.has_service_type(2) || s.has_service_type(8))
            .len(),
        1
    );

    assert_eq!(service_type_name(2), Some("Law Dispatch"));
    assert_eq!(service_type_name(3), Some("Fire Dispatch"));
    assert_eq!(service_type_name(8), Some("Fire-Tac"));
    // Authoritative Uniden-spec codes that earlier guesses had wrong / missing.
    assert_eq!(service_type_name(22), Some("Multi-Talk")); // was mis-labeled "Transportation"
    assert_eq!(service_type_name(23), Some("Law Talk")); // was mis-labeled "City Services"
    assert_eq!(service_type_name(26), Some("Transportation"));
    assert_eq!(service_type_name(32), Some("Schools"));
    assert_eq!(service_type_name(215), Some("Custom 8"));
    assert_eq!(service_type_name(217), Some("Racing Teams")); // spec names 216/217 specially
    assert_eq!(service_type_name(5), None); // spec "non" (unused)
    assert_eq!(service_type_name(999), None);
}
