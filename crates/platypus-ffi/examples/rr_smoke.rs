// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Headless smoke for the RR FFI surface — exercises the same entry points the app calls.
//! Gated on env credentials; every unique query is cached, so re-runs replay offline.
//!
//! Usage: `RR_APP_KEY=… RR_USERNAME=… RR_PASSWORD=… cargo run -p platypus-ffi --example rr_smoke`

use std::ffi::{c_char, CStr, CString};
use std::ptr;

use platypus_ffi::{
    platypus_rr_source_categories_json, platypus_rr_source_category_channels_json,
    platypus_rr_source_channels_json, platypus_rr_source_free, platypus_rr_source_geo_json,
    platypus_rr_source_open, platypus_rr_source_refresh, platypus_rr_source_systems_json,
    platypus_rr_source_warm_batch, platypus_rr_validate, platypus_string_free,
};

fn s(v: &str) -> CString {
    CString::new(v).unwrap()
}

unsafe fn take(ptr: *mut c_char) -> String {
    if ptr.is_null() {
        return String::from("<null>");
    }
    let out = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    platypus_string_free(ptr);
    out
}

fn main() {
    let key = std::env::var("RR_APP_KEY").expect("RR_APP_KEY");
    let user = std::env::var("RR_USERNAME").expect("RR_USERNAME");
    let pass = std::env::var("RR_PASSWORD").expect("RR_PASSWORD");
    let (key, user, pass) = (s(&key), s(&user), s(&pass));
    let zip = std::env::args().nth(1).unwrap_or_else(|| "97201".into());

    unsafe {
        // 1) Validate credentials.
        let mut err: *mut c_char = ptr::null_mut();
        let account = platypus_rr_validate(key.as_ptr(), user.as_ptr(), pass.as_ptr(), &mut err);
        if account.is_null() {
            eprintln!("validate failed: {}", take(err));
            std::process::exit(1);
        }
        println!("account: {}", take(account));

        // 2) Open the location.
        let selector = s(&format!("zip:{zip}"));
        let mut err: *mut c_char = ptr::null_mut();
        let src = platypus_rr_source_open(
            key.as_ptr(),
            user.as_ptr(),
            pass.as_ptr(),
            selector.as_ptr(),
            ptr::null_mut(),
            None,
            None,
            &mut err,
        );
        if src.is_null() {
            eprintln!("open failed: {}", take(err));
            std::process::exit(1);
        }

        // 3) List systems.
        let systems = take(platypus_rr_source_systems_json(
            src,
            ptr::null(),
            ptr::null(),
            ptr::null(),
        ));
        let count = systems.matches("\"id\":").count();
        println!("systems: {count}");
        println!(
            "first row: {}",
            systems.split("},{").next().unwrap_or(&systems)
        );

        // 4) Drill a system (arg2 ref, else the first). // arg2 drill
        let want = std::env::args().nth(2);
        let pick = want.as_deref().or_else(|| {
            systems
                .split("\"id\":\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
        });
        if let Some(id) = pick {
            let sref = s(id);
            if id.starts_with('t') {
                // Trunked → the location-first category drill.
                let cats = take(platypus_rr_source_categories_json(src, sref.as_ptr(), 0));
                let nc = cats.matches("\"id\":").count();
                println!("in-range categories of {id}: {nc}");
                for row in cats.split("},{").take(4) {
                    println!(
                        "  cat: {}",
                        row.trim_start_matches('[').trim_start_matches('{')
                    );
                }
                if let Some(cid) = cats.split("\"id\":").nth(1).and_then(|s| {
                    s.split(|c: char| !c.is_ascii_digit())
                        .find(|x| !x.is_empty())
                }) {
                    let cc = take(platypus_rr_source_category_channels_json(
                        src,
                        sref.as_ptr(),
                        cid.parse().unwrap_or(0),
                        ptr::null(),
                        ptr::null(),
                    ));
                    println!(
                        "channels in first category ({cid}): {}",
                        cc.matches("\"id\":").count()
                    );
                }
            } else {
                let chans = take(platypus_rr_source_channels_json(
                    src,
                    sref.as_ptr(),
                    ptr::null(),
                    ptr::null(),
                ));
                println!("channels of {id}: {}", chans.matches("\"id\":").count());
            }
        }

        // 5) Map geo — real per-system pins (should be distinct coords, not the centroid).
        let geo = take(platypus_rr_source_geo_json(
            src,
            0.0,
            0.0,
            0.0,
            ptr::null(),
            ptr::null(),
            ptr::null(),
        ));
        println!("map pins: {}", geo.matches("\"lat\":").count());
        for row in geo.split("},{").take(4) {
            let name = row
                .split("\"name\":\"")
                .nth(1)
                .and_then(|s| s.split('"').next());
            let lat = row
                .split("\"lat\":")
                .nth(1)
                .and_then(|s| s.split(',').next());
            let lon = row
                .split("\"lon\":")
                .nth(1)
                .and_then(|s| s.split(',').next());
            let rng = row
                .split("\"rangeMi\":")
                .nth(1)
                .and_then(|s| s.split([',', '}']).next());
            println!("  pin: {:?} @ {:?},{:?}  range={:?}", name, lat, lon, rng);
        }

        // 6) Progressive pin warming — each pass fetches a bounded batch of real site positions
        //    (cached ones instant, live ones paced); the count falls to 0 once everything is placed.
        for pass in 1..=5 {
            let warmed = platypus_rr_source_warm_batch(src, 4);
            println!("warm pass {pass}: {warmed} sites");
            if warmed == 0 {
                break;
            }
        }

        // 7) Refresh — bypass the cache, re-fetch the county + site geo, and return the location JSON.
        println!("refresh: {}", take(platypus_rr_source_refresh(src)));

        platypus_rr_source_free(src);
    }
    println!("rr_smoke OK");
}
