// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Benchmark: how fast is **in-memory** filtering over a full HPDB at USA scale?
//! Answers the "do we need a database for fast filtering?" question with numbers.
//!
//! Loads every `s_*.hpd` in a directory into the canonical `Dataset`, then times
//! a few representative filters across ALL systems at once. Generic over the input
//! path (no location hard-coded).
//!
//!   cargo run --release --example bench_fullusa -- <dir-of-state-hpd-files>

use std::time::Instant;
use std::{env, fs, process};

use platypus_core::provider::{Dataset, HpdbProvider, Provider, SystemKind};

fn main() {
    let Some(dir) = env::args().nth(1) else {
        eprintln!("usage: bench_fullusa <dir-of-state-hpd-files>");
        process::exit(2);
    };

    // ---- load: parse every state file into one combined canonical Dataset ----
    let t0 = Instant::now();
    let mut all = Dataset::default();
    let mut files = 0usize;
    for entry in fs::read_dir(&dir).expect("read dir") {
        let path = entry.unwrap().path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !(name.starts_with("s_") && name.ends_with(".hpd")) {
            continue;
        }
        let bytes = fs::read(&path).expect("read file");
        let Ok(doc) = platypus_core::format::Document::parse(&bytes) else {
            continue;
        };
        if let Ok(ds) = HpdbProvider::from_document(doc, name).load() {
            all.systems.extend(ds.systems);
            files += 1;
        }
    }
    let load = t0.elapsed();

    let channels: usize = all.systems.iter().map(|s| s.channels.len()).sum();
    let sites: usize = all.systems.iter().map(|s| s.locations.len()).sum();
    println!("loaded {files} files in {load:.2?}");
    println!(
        "  systems={}  sites/groups={}  channels={}",
        all.systems.len(),
        sites,
        channels
    );

    // ---- filter: time representative predicates across the whole dataset ----
    let bench = |label: &str, f: &dyn Fn() -> usize| {
        // a few iterations to get past noise; report best.
        let mut best = std::time::Duration::MAX;
        let mut n = 0;
        for _ in 0..5 {
            let t = Instant::now();
            n = f();
            best = best.min(t.elapsed());
        }
        println!("  {label:<34} -> {n:>6} systems in {best:.3?}");
    };

    println!("filters (best of 5, whole-USA in memory):");
    bench("service type = Fire Dispatch (3)", &|| {
        all.select(|s| s.has_service_type(3)).len()
    });
    bench("tech = P25Standard", &|| {
        all.select(|s| s.tech_is("P25Standard")).len()
    });
    bench("kind = Trunk", &|| {
        all.select(|s| s.kind == SystemKind::Trunk).len()
    });
    // compound: fire OR ems, trunked, any one county tag present
    bench("fire|ems AND trunked", &|| {
        all.select(|s| {
            s.kind == SystemKind::Trunk && (s.has_service_type(3) || s.has_service_type(4))
        })
        .len()
    });
    // channel-level scan (the worst case: touch every channel)
    bench("channel scan: name contains 'Police'", &|| {
        all.systems
            .iter()
            .filter(|s| s.channels.iter().any(|c| c.name.contains("Police")))
            .count()
    });
}
