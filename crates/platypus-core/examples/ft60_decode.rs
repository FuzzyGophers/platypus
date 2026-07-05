// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Decode a captured FT-60 clone image and print the channels + round-trip check.
//!
//! Usage:  cargo run -p platypus-core --example ft60_decode -- [IMAGE]   (default ft60.img)

use platypus_core::device::ft60::{Ft60Image, Tone};

fn tone_str(t: &Tone) -> String {
    match t {
        Tone::None => "—".to_string(),
        Tone::Ctcss(hz10) => format!("CTCSS {:.1}", *hz10 as f64 / 10.0),
        Tone::Dcs(code) => format!("DCS {code:03}"),
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ft60.img".to_string());
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {path}: {e}");
            std::process::exit(1);
        }
    };

    let image = match Ft60Image::decode(&bytes) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("decode failed: {e}");
            std::process::exit(1);
        }
    };

    // Round-trip safety gate.
    let re = image.encode();
    let rt_ok = re == bytes[..re.len()];
    println!(
        "round-trip decode→encode == input : {}",
        if rt_ok { "PASS ✓" } else { "FAIL ✗" }
    );

    let chans = image.channels();
    println!("used channels: {}\n", chans.len());
    println!(
        "{:>4}  {:<7}  {:>11}  {:<4}  {:<14}  {:<6}  {:<3}  banks",
        "#", "name", "rx MHz", "mode", "tone", "duplex", "skp"
    );
    for ch in &chans {
        let banks: String = ch.banks.iter().map(|b| (b'A' + b) as char).collect();
        println!(
            "{:>4}  {:<7}  {:>11.4}  {:<4}  {:<14}  {:<6}  {:<3}  {}",
            ch.slot + 1,
            ch.name,
            ch.rx_hz as f64 / 1_000_000.0,
            ch.mode,
            tone_str(&ch.tone),
            ch.duplex,
            ch.skip,
            banks,
        );
    }

    // Calibration aid: raw bytes for the first few used channels (mem record @0x0248, name
    // @0x4708) so we can confirm the freq/name decoding against the real radio.
    println!(
        "\n--- raw (first {} used, for calibration) ---",
        chans.len().min(8)
    );
    let raw = image.raw();
    for ch in chans.iter().take(8) {
        let c = ch.slot as usize;
        let mem = &raw[0x0248 + c * 16..0x0248 + (c + 1) * 16];
        let nm = &raw[0x4708 + c * 8..0x4708 + c * 8 + 6];
        let memhex: String = mem.iter().map(|b| format!("{b:02x} ")).collect();
        let nmhex: String = nm.iter().map(|b| format!("{b:02x} ")).collect();
        println!(
            "#{:>3}  mem[{}]  name[{}]",
            ch.slot + 1,
            memhex.trim(),
            nmhex.trim()
        );
    }
}
