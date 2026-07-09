// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Offline parser tests against **synthetic** fixtures (fictional data in the real RR rpc/encoded
//! SOAP shape — no RR data is shipped). Zero network.

use platypus_core::provider::{Provider, SystemKind};
use platypus_core::rr::{RadioReferenceProvider, RrSystem};
use platypus_rr::parse::*;

fn fx(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {path}"))
}

#[test]
fn zipcode_info() {
    let z = parse_zipcode_info(&fx("zipcode.xml")).unwrap();
    assert_eq!(z.ctid, 9001);
    assert_eq!(z.stid, 90);
    assert_eq!(z.city, "Example City");
    assert!((z.lat - 45.0).abs() < 1e-9);
}

#[test]
fn trs_details() {
    let t = parse_trs_details(&fx("trs_details.xml")).unwrap();
    assert_eq!(t.s_name, "Example Statewide P25");
    assert_eq!(t.s_type, 8);
    assert_eq!(t.s_flavor, 20);
    assert_eq!(t.county_ids, vec![9001, 9002]);
    assert_eq!(t.state_ids, vec![90]);
}

#[test]
fn trs_sites_with_freqs_and_nil_nac() {
    let sites = parse_trs_sites(&fx("trs_sites.xml"));
    assert_eq!(sites.len(), 2);
    assert_eq!(sites[0].site_descr, "North Hill");
    assert_eq!(sites[0].nac.as_deref(), Some("293"));
    assert_eq!(sites[0].frequencies.len(), 1);
    assert!((sites[0].frequencies[0].freq_mhz - 851.0125).abs() < 1e-9);
    assert_eq!(sites[0].frequencies[0].use_, "d");
    // xsi:nil nac parses to None.
    assert_eq!(sites[1].nac, None);
}

#[test]
fn talkgroups_with_entity_and_tags() {
    let tgs = parse_talkgroups(&fx("talkgroups.xml"));
    assert_eq!(tgs.len(), 2);
    assert_eq!(tgs[0].tg_dec, "1001");
    assert_eq!(tgs[0].tg_alpha, "PD DISP");
    assert_eq!(tgs[0].tags[0].tag_id, 2);
    // `&amp;` unescapes.
    assert_eq!(tgs[1].tg_descr, "Fire & EMS Dispatch");
}

#[test]
fn subcat_freqs() {
    let freqs = parse_subcat_freqs(&fx("subcat_freqs.xml"));
    assert_eq!(freqs.len(), 2);
    assert!((freqs[0].out_mhz - 154.265).abs() < 1e-9);
    assert_eq!(freqs[0].tone, "131.8 PL");
    assert_eq!(freqs[0].mode, "FM");
    assert_eq!(freqs[0].tags[0].tag_id, 8);
}

#[test]
fn state_info_lists_systems_counties_agencies() {
    let s = parse_state(&fx("state.xml"));
    assert_eq!(s.trs.len(), 2);
    assert_eq!(s.trs[0].sid, 7001);
    assert_eq!(s.trs[0].s_type, 8);
    assert_eq!(s.trs[0].s_flavor, 20);
    assert_eq!(s.county_ids, vec![9001, 9002]);
    assert_eq!(s.agency_ids, vec![3300]);
}

#[test]
fn mode_name_resolves() {
    assert_eq!(parse_mode_name(&fx("mode.xml")).as_deref(), Some("FM"));
}

#[test]
fn talkgroup_cats_with_entity() {
    let cats = parse_talkgroup_cats(&fx("tg_cats.xml"));
    assert_eq!(cats.len(), 2);
    assert_eq!(cats[0].tg_cid, 4101);
    assert_eq!(cats[0].name, "Law Enforcement");
    // Geo drives location-first ranking; a systemwide category has lat/lon 0.
    assert!((cats[0].lat - 45.0).abs() < 1e-9 && (cats[0].range - 20.0).abs() < 1e-9);
    assert_eq!(cats[1].lat, 0.0);
    // `&amp;` unescapes.
    assert_eq!(cats[1].name, "Fire & EMS");
}

#[test]
fn county_freqs_by_tag_parse_as_freqs() {
    // The tag-filtered conventional response is the same `freq` array as getSubcatFreqs.
    let freqs = parse_subcat_freqs(&fx("county_freqs_by_tag.xml"));
    assert_eq!(freqs.len(), 2);
    assert!((freqs[0].out_mhz - 154.28).abs() < 1e-9);
    assert_eq!(freqs[0].mode, "FM");
    assert_eq!(freqs[0].tags[0].tag_id, 8);
}

#[test]
fn country_list_and_info() {
    let cs = parse_country_list(&fx("country_list.xml"));
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0].coid, 1);
    assert_eq!(cs[0].name, "Exampleland");
    assert_eq!(cs[0].code, "EX");

    let ci = parse_country_info(&fx("country_info.xml")).unwrap();
    assert_eq!(ci.coid, 1);
    assert_eq!(ci.states.len(), 2);
    assert_eq!(ci.states[1].stid, 91);
    assert_eq!(ci.states[1].name, "Sample");
}

#[test]
fn states_and_counties_and_metros() {
    let states = parse_states(&fx("states.xml"));
    assert_eq!(states.len(), 2);
    assert_eq!(states[0].code, "EX");

    // getCountiesByList and getMetroAreaInfo share the Counties shape.
    let counties = parse_counties(&fx("counties.xml"));
    assert_eq!(counties.len(), 2);
    assert_eq!(counties[1].ctid, 9002);
    assert_eq!(counties[1].name, "Sample");

    let metros = parse_metros(&fx("metros.xml"));
    assert_eq!(metros.len(), 1);
    assert_eq!(metros[0].mid, 44);
    assert_eq!(metros[0].name, "Example Metro");
}

#[test]
fn trs_by_sysid_lists_systems() {
    let hits = parse_trs_list(&fx("trs_by_sysid.xml"));
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].sid, 7001);
    assert_eq!(hits[0].s_type, 8);
    assert_eq!(hits[1].sid, 7002);
}

#[test]
fn search_freqs_carry_identifying_refs() {
    let r = parse_search_freqs(&fx("search_freqs.xml"));
    assert_eq!(r.len(), 2);
    assert!((r[0].out_mhz - 453.525).abs() < 1e-9);
    assert_eq!(r[0].callsign, "WQAA123");
    assert_eq!(r[0].aid, 3300);
    assert_eq!(r[0].scid, 4242);
    assert_eq!(r[0].tags[0].tag_id, 11);
    // Refs absent on a match ⇒ 0 (graceful), fields still parse.
    assert_eq!(r[1].aid, 0);
    assert_eq!(r[1].descr, "Fire Dispatch");
}

#[test]
fn fcc_callsign_frequencies_and_codes() {
    let c = parse_fcc_callsign(&fx("fcc_callsign.xml")).unwrap();
    assert_eq!(c.callsign, "WQAA123");
    assert_eq!(c.licensee, "Example Utilities Inc");
    assert_eq!(c.radio_service, "PW");
    // The nested element is `frequency`, not `freq`.
    assert_eq!(c.frequencies.len(), 2);
    assert!((c.frequencies[0] - 453.525).abs() < 1e-9);

    let codes = parse_fcc_service_codes(&fx("fcc_service_code.xml"));
    assert_eq!(codes.len(), 1);
    assert_eq!(codes[0].code, "PW");
    assert_eq!(codes[0].description, "Public Safety Pool");
}

#[test]
fn prox_callsigns_with_distance() {
    let p = parse_prox_callsigns(&fx("prox_callsigns.xml"));
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].callsign, "WQAA123");
    assert!((p[0].distance - 0.8).abs() < 1e-9);
    assert_eq!(p[1].callsign, "KEX456");
}

#[test]
fn user_data_and_feeds_drop_password() {
    let u = parse_user_data(&fx("user_data.xml")).unwrap();
    assert_eq!(u.username, "example_user");
    assert!(u.sub_expire_date.starts_with("2027-12-31"));

    let feeds = parse_feed_broadcasts(&fx("feed_broadcasts.xml"));
    assert_eq!(feeds.len(), 1);
    assert_eq!(feeds[0].feed_id, 12345);
    assert_eq!(feeds[0].mount, "/EX1");
    // The feed password must never enter the parsed struct.
    assert!(!format!("{feeds:?}").contains("SHOULD-NOT-BE-PARSED"));
}

#[test]
fn trunked_system_maps_to_canonical_dataset() {
    let sys = platypus_rr::parse_trunked_system(
        &fx("trs_details.xml"),
        &fx("trs_sites.xml"),
        &fx("talkgroups.xml"),
    )
    .unwrap();
    let provider = RadioReferenceProvider::from_systems(vec![RrSystem::Trunked(sys)], "RR fixture");
    let ds = provider.load().unwrap();
    assert_eq!(ds.systems.len(), 1);
    let s = &ds.systems[0];
    assert_eq!(s.name, "Example Statewide P25");
    assert_eq!(s.kind, SystemKind::Trunk);
    assert_eq!(s.locations.len(), 2); // two sites
    assert_eq!(s.channels.len(), 2); // two talkgroups
    assert!(s.has_service_type(2)); // Law Dispatch tag from TG 1
}
