// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! FT-60 clone-image round-trip against **real captured `*.img` files** at the repo root
//! (opaque, gitignored, never committed, like the real SD-card fixtures) — each must
//! `decode → encode` byte-for-byte, and re-applying its channels must be the identity.
//!
//! `#[ignore]` by default (it needs a real capture, which isn't in the repo). Run it
//! explicitly against a capture with:
//!
//! ```sh
//! cargo test -p platypus-core --test ft60_roundtrip -- --ignored
//! ```
//!
//! The CI-run FT-60 round-trip + write-gate coverage lives in the `device::ft60` unit tests,
//! which exercise the same codec against an in-crate `synthetic()` image.

use std::fs;
use std::path::{Path, PathBuf};

use platypus_core::device::ft60::Ft60Image;

/// Captured `*.img` files at the repo root (two levels up from this crate).
fn img_fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("img") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
#[ignore = "requires a real *.img capture at the repo root; run with --ignored"]
fn ft60_images_round_trip_byte_for_byte() {
    let files = img_fixtures();
    if files.is_empty() {
        eprintln!(
            "no *.img fixtures present — skipping (capture one from a radio to exercise this)"
        );
        return;
    }
    for path in &files {
        let raw = fs::read(path).unwrap();
        let img = Ft60Image::decode(&raw)
            .unwrap_or_else(|e| panic!("decode failed for {}: {e}", path.display()));
        let enc = img.encode();
        assert_eq!(
            enc.as_slice(),
            &raw[..enc.len()],
            "round-trip mismatch for {}",
            path.display()
        );

        // Write-side gate: re-applying the decoded channels must not move a single byte
        // (proves every field setter is a perfect inverse of its decoder, on real data).
        let mut edited = img.clone();
        let chans = img.channels();
        edited.apply_channels(&chans);
        assert_eq!(
            edited.encode(),
            enc,
            "apply(channels()) must be identity for {}",
            path.display()
        );
        // Sanity: the interpretation runs without panicking and finds programmed channels.
        eprintln!(
            "{}: round-trip clean, {} channels",
            path.display(),
            img.channels().len()
        );
    }
}
