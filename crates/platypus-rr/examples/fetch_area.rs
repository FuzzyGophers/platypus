// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Gated live spike — the ONLY code that talks to RadioReference. Runs the location-first chain for
//! one ZIP (zip → county → a few systems), caches every response on disk, maps them through
//! `platypus_core`, and prints the canonical `Dataset`. Re-runs replay from the cache (no RR calls).
//!
//! Usage:
//!   RR_APP_KEY=… RR_USERNAME=… RR_PASSWORD=… \
//!     cargo run -p platypus-rr --example fetch_area -- --zip 97201
//!
//! The default ZIP is a neutral public area — never the owner's real location (privacy rule). Only
//! the first few systems are fetched, to stay gentle on the API.

use platypus_core::provider::Provider;
use platypus_core::rr::{Credentials, RadioReferenceProvider};
use platypus_rr::{parse, RrClient};

const DEFAULT_ZIP: u32 = 97201;
const MAX_TRUNKED: usize = 2;
const MAX_CONVENTIONAL: usize = 3;

fn main() {
    let creds = match env_creds() {
        Ok(c) => c,
        Err(missing) => {
            eprintln!(
                "missing env var {missing}. Run with:\n  \
                 RR_APP_KEY=… RR_USERNAME=… RR_PASSWORD=… \\\n    \
                 cargo run -p platypus-rr --example fetch_area -- --zip {DEFAULT_ZIP}"
            );
            std::process::exit(2);
        }
    };
    let zip = zip_arg().unwrap_or(DEFAULT_ZIP);
    let client = RrClient::new(creds);

    println!("→ getZipcodeInfo({zip})");
    let Some(z) = client
        .get_zipcode_info(zip)
        .ok()
        .and_then(|xml| parse::parse_zipcode_info(&xml))
    else {
        eprintln!("could not resolve ZIP {zip} — check auth / Premium subscription");
        std::process::exit(1);
    };
    println!("  {} — county {} (state {})", z.city, z.ctid, z.stid);

    println!("→ fetch_county({}) [first {MAX_TRUNKED} trunked + {MAX_CONVENTIONAL} conventional, cached]", z.ctid);
    let systems = match client.fetch_county(z.ctid as u32, MAX_TRUNKED, MAX_CONVENTIONAL) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let ds = RadioReferenceProvider::from_systems(systems, "RadioReference")
        .load()
        .expect("map to dataset");
    println!("\n== canonical Dataset: {} system(s) ==", ds.systems.len());
    for s in &ds.systems {
        println!(
            "  [{:?}] {} — tech {:?}, {} channels, {} sites",
            s.kind,
            s.name,
            s.tech.as_deref().unwrap_or("—"),
            s.channels.len(),
            s.locations.len(),
        );
    }
}

fn env_creds() -> Result<Credentials, &'static str> {
    Ok(Credentials {
        app_key: std::env::var("RR_APP_KEY").map_err(|_| "RR_APP_KEY")?,
        username: std::env::var("RR_USERNAME").map_err(|_| "RR_USERNAME")?,
        password: std::env::var("RR_PASSWORD").map_err(|_| "RR_PASSWORD")?,
    })
}

fn zip_arg() -> Option<u32> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--zip" {
            return args.next().and_then(|v| v.parse().ok());
        }
        if let Some(v) = a.strip_prefix("--zip=") {
            return v.parse().ok();
        }
    }
    None
}
