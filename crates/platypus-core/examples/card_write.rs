// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! **DEV TOOL — writes to a real SD card.** Bring-up harness: build a favorites list from a
//! county filter and optionally commit it to a card. **Dry-run by default** — it only touches
//! the card when given `--write`, and it refuses to overwrite an existing slot.
//!
//! usage:
//!   cargo run --example card_write -- \
//!     <card_mount> <source_state.hpd> <selector> <slot> <label> [options]
//!
//!   selector : county:712 | tech:P25 | name:SAFE-T
//!   options  : --near=LAT,LON,MILES  (site+talkgroup location filter)
//!              --bandplan            (add band plan to kept P25 sites)
//!              --dqks-on             (DQKs On instead of the default Off — preference)
//!              --write               (actually commit; dry-run otherwise)

use std::path::Path;
use std::{env, fs, process};

use platypus_core::device::Sds150;
use platypus_core::extract::{self, Extraction};
use platypus_core::format::Document;
use platypus_core::{card, favorites};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 6 {
        eprintln!(
            "usage: phase_b_write <card_mount> <source_state.hpd> <selector> <slot> <label> [--write]\n  selector: county:N | tech:STR | name:STR"
        );
        process::exit(2);
    }
    let card_mount = Path::new(&args[1]);
    let source = &args[2];
    let selector = &args[3];
    let slot: u32 = args[4].parse().expect("slot must be a number");
    let label = &args[5];
    let do_write = args.iter().any(|a| a == "--write");

    let profile = Sds150::new();
    let doc = Document::parse(&fs::read(source).expect("read source")).expect("parse source");

    let ext = Extraction::segment(&doc, &profile);
    let (kind, value) = selector
        .split_once(':')
        .expect("selector must be county:N | tech:STR | name:STR");
    let selection = match kind {
        "county" => {
            let id: u64 = value.parse().expect("county id must be a number");
            ext.select(|s| s.is_in_county(id))
        }
        "tech" => ext.select(|s| s.tech().is_some_and(|t| t.contains(value))),
        "name" => ext.select(|s| s.name().is_some_and(|n| n.contains(value))),
        other => {
            eprintln!("unknown selector kind: {other}");
            process::exit(2);
        }
    };
    // Optional site-level location filter: --near=LAT,LON,MILES keeps only sites
    // near you (the location-first answer to deselecting 100+ sites by hand).
    let selection = match args.iter().find_map(|a| a.strip_prefix("--near=")) {
        Some(spec) => {
            let p: Vec<f64> = spec
                .split(',')
                .map(|s| s.parse().expect("--near=lat,lon,miles"))
                .collect();
            extract::filter_within_radius(&selection, &profile, p[0], p[1], p[2])
        }
        None => selection,
    };

    let n_systems = Extraction::segment(&selection, &profile).system_count();
    let n_sites = selection
        .lines
        .iter()
        .filter(|l| l.command() == "Site")
        .count();
    // DQKs is a user preference, not a scan requirement — both values are in working
    // files (Hawaii's P25 trunks use On, another list's Off) and both work. Default
    // Off; --dqks-on flips it. (In the app this is a checkbox.)
    let departments_on = args.iter().any(|a| a == "--dqks-on");
    let mut favs = favorites::build_favorites(&selection, &profile, departments_on);
    // Optional band plan on the kept P25 sites (--bandplan). Rare: only some
    // simulcast sites use one (e.g. SAFE-T's county simulcasts).
    if args.iter().any(|a| a == "--bandplan") {
        favs = favorites::with_synthesized_bandplan(&favs, &profile);
    }

    let count = |cmd: &str| favs.lines.iter().filter(|l| l.command() == cmd).count();
    let bytes = favs.to_bytes();
    let round_trips = Document::parse(&bytes).map(|d| d.to_bytes()) == Ok(bytes.clone());
    let target = card::favorites_path(card_mount, &profile, slot);

    println!("source         : {source}");
    println!("selector       : {selector}");
    println!("systems        : {n_systems}");
    println!("sites kept     : {n_sites}");
    println!("DQKs_Status    : {}", count("DQKs_Status"));
    println!("BandPlan_P25   : {}", count("BandPlan_P25"));
    println!("favorites bytes: {}", bytes.len());
    println!("round-trips    : {round_trips}");
    println!("target slot    : {}  (label: {label})", target.display());

    if !do_write {
        println!("\nDRY RUN — nothing written. Re-run with --write to commit.");
        return;
    }
    if !round_trips {
        eprintln!("REFUSING: favorites did not round-trip.");
        process::exit(1);
    }
    if target.exists() {
        eprintln!(
            "REFUSING: slot already exists, won't overwrite: {}",
            target.display()
        );
        process::exit(1);
    }

    card::commit_favorites(card_mount, &profile, slot, label, &favs).expect("commit failed");
    println!("\nWROTE slot {slot}, registered it in f_list.cfg, deleted app_data.cfg.");
    println!("NOW EJECT the card before reconnecting it to the scanner.");
}
