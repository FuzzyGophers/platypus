// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Dev tool — fetch one RR method's raw response (cached) for inspecting the wire format while
//! building the parser. Credentials come from the environment; each unique call hits RR once.
//!
//! Usage: `cargo run -p platypus-rr --example dump -- <method> [args…]`
//! methods:
//!   zip·county·trs-details·trs-sites·talkgroups·subcat-freqs·type·flavor·voice·state·mode·tag·
//!   tg-cats·agency-info   take `<id>`
//!   county-freqs-by-tag·agency-freqs-by-tag   take `<id> <tag>`
//!   country-list·user-data·feeds   take no args
//!   country-info·metro·metro-info   take `<id>`
//!   states-by-list·counties-by-list   take `<id> [<id>…]`
//!   by-sysid·fcc-callsign   take `<string>`
//!   fcc-service-code   takes `<code>`
//!   search-county·search-state·search-metro   take `<id> <freq> [<tone>]`
//!   fcc-prox   takes `<lat> <lon> <range> [<unit=m>]`

use platypus_core::rr::Credentials;
use platypus_rr::RrClient;

fn main() {
    let creds = Credentials {
        app_key: env("RR_APP_KEY"),
        username: env("RR_USERNAME"),
        password: env("RR_PASSWORD"),
    };
    let a: Vec<String> = std::env::args().skip(1).collect();
    let method = a.first().cloned().unwrap_or_default();
    let arg = |i: usize| a.get(i).cloned().unwrap_or_default();
    let u = |i: usize| arg(i).parse::<u32>().unwrap_or(0);
    let f = |i: usize| arg(i).parse::<f64>().unwrap_or(0.0);
    let ids: Vec<u32> = a.iter().skip(1).filter_map(|s| s.parse().ok()).collect();
    let prox_unit = if arg(4).is_empty() {
        "m".into()
    } else {
        arg(4)
    };

    let client = RrClient::new(creds);
    let result = match method.as_str() {
        "zip" => client.get_zipcode_info(u(1)),
        "county" => client.get_county_info(u(1)),
        "trs-details" => client.get_trs_details(u(1)),
        "trs-sites" => client.get_trs_sites(u(1)),
        "talkgroups" => client.get_trs_talkgroups(u(1)),
        "subcat-freqs" => client.get_subcat_freqs(u(1)),
        "type" => client.get_trs_type(u(1)),
        "flavor" => client.get_trs_flavor(u(1)),
        "voice" => client.get_trs_voice(u(1)),
        "state" => client.get_state_info(u(1)),
        "mode" => client.get_mode(u(1)),
        "tag" => client.get_tag(u(1)),
        "tg-cats" => client.get_trs_talkgroup_cats(u(1)),
        "tg-in-cat" => client.get_trs_talkgroups_in_cat(u(1), u(2)),
        "county-freqs-by-tag" => client.get_county_freqs_by_tag(u(1), u(2)),
        "agency-freqs-by-tag" => client.get_agency_freqs_by_tag(u(1), u(2)),
        "agency-info" => client.get_agency_info(u(1)),
        "country-list" => client.get_country_list(),
        "country-info" => client.get_country_info(u(1)),
        "states-by-list" => client.get_states_by_list(&ids),
        "counties-by-list" => client.get_counties_by_list(&ids),
        "metro" => client.get_metro_area(u(1)),
        "metro-info" => client.get_metro_area_info(u(1)),
        "by-sysid" => client.get_trs_by_sysid(&arg(1)),
        "search-county" => client.search_county_freq(u(1), f(2), &arg(3)),
        "search-state" => client.search_state_freq(u(1), f(2), &arg(3)),
        "search-metro" => client.search_metro_freq(u(1), f(2), &arg(3)),
        "fcc-callsign" => client.fcc_get_callsign(&arg(1)),
        "fcc-service-code" => client.fcc_get_radio_service_code(&arg(1)),
        "fcc-prox" => client.fcc_get_prox_callsigns(f(1), f(2), f(3), &prox_unit),
        "user-data" => client.get_user_data(),
        "feeds" => client.get_user_feed_broadcasts(),
        other => {
            eprintln!("unknown method: {other}");
            std::process::exit(2);
        }
    };
    match result {
        Ok(body) => println!("{body}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| {
        eprintln!("set {key} (RR_APP_KEY / RR_USERNAME / RR_PASSWORD)");
        std::process::exit(2);
    })
}
