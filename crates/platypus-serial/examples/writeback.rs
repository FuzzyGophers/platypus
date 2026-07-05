// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! **DEV TOOL — writes to a real FT-60 radio.** Clones an FT-60 image OUT to the radio.
//! Bring-up tool for the clone-out handshake.
//!
//! Usage:  cargo run -p platypus-serial --example writeback -- [PORT] [IMAGE]
//!   PORT   serial device (default: first /dev/cu.usbserial*)
//!   IMAGE  image to write  (default: ft60.img — the one you captured)
//!
//! SAFETY: writing back the *same* image you read is safe — every byte equals what's already
//! on the radio, so even a partial/failed transfer can't corrupt data. Do NOT point this at an
//! arbitrary/edited image until the codec's round-trip gate covers your edits.
//!
//! On the radio: arm CLONE (MONI+power → DIAL→CLONE → F/W), then press MONI to RECEIVE — the
//! display shows `-WAIT-` / "Clone RX". THEN run this. Afterward, re-run the capture tool and
//! diff against the image to confirm the radio is byte-identical.

use std::time::Duration;

use platypus_core::device::ft60::Ft60Image;
use platypus_core::device::CloneSpec;
use platypus_serial::{list_ports, write_ft60_image, CloneTimeouts, Progress, SerialPort};

struct Printer {
    last: usize,
}
impl Progress for Printer {
    fn update(&mut self, bytes: usize, total: usize) {
        if bytes - self.last >= 2048 || bytes >= total {
            eprintln!("  sent {bytes} / {total} bytes");
            self.last = bytes;
        }
    }
}

fn pick_port(arg: Option<String>) -> Option<String> {
    if let Some(p) = arg {
        return Some(p);
    }
    let ports = list_ports();
    ports
        .iter()
        .find(|p| p.contains("usbserial") || p.contains("usbmodem"))
        .cloned()
        .or_else(|| ports.first().cloned())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let port_arg = args.next();
    let image_path = args.next().unwrap_or_else(|| "ft60.img".to_string());

    let bytes = match std::fs::read(&image_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {image_path}: {e}");
            std::process::exit(1);
        }
    };
    // Validate + round-trip the image before we send a single byte.
    let img = match Ft60Image::decode(&bytes) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{image_path} is not a valid FT-60 image: {e}");
            std::process::exit(1);
        }
    };
    if img.encode() != bytes[..img.encode().len()] {
        eprintln!("REFUSING: {image_path} does not round-trip — aborting to avoid corruption.");
        std::process::exit(1);
    }
    eprintln!(
        "image ok: {} bytes, round-trips, {} channels",
        bytes.len(),
        img.channels().len()
    );

    let Some(port_path) = pick_port(port_arg) else {
        eprintln!("no serial port found — plug in the clone cable");
        std::process::exit(1);
    };
    let spec = CloneSpec::FT60;
    let mut port =
        match SerialPort::open(port_path.as_ref(), spec.baud, Duration::from_millis(1000)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("open {port_path}: {e}");
                std::process::exit(1);
            }
        };

    eprintln!("\nArm the radio: CLONE (MONI+power, DIAL→CLONE, F/W), then press MONI → -WAIT-.");
    eprintln!("Writing to {port_path} in 3 s (Ctrl-C to abort)…");
    std::thread::sleep(Duration::from_secs(3));

    let mut prog = Printer { last: 0 };
    match write_ft60_image(
        &mut port,
        &spec,
        &bytes,
        CloneTimeouts::default(),
        &mut prog,
    ) {
        Ok(()) => {
            eprintln!("\nDONE — wrote {} bytes.", bytes.len());
            eprintln!("Verify: re-run the capture tool and `cmp` the result against {image_path}.");
        }
        Err(e) => {
            eprintln!("\nwrite failed: {e}");
            std::process::exit(1);
        }
    }
}
