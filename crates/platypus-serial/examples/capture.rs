// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Capture a raw FT-60 clone image from the radio and save it to a file.
//!
//! Usage:  cargo run -p platypus-serial --example capture -- [PORT] [OUT]
//!   PORT  serial device (default: first /dev/cu.usbserial*; else first candidate)
//!   OUT   output path   (default: ft60.img)
//!
//! On the radio: hold MONI at power-on → set `F8 CLONE` → press `[F/W]`, then press MONI to
//! SEND. This tool receives + saves the image, then prints the magic and length so we can
//! confirm the wire framing against the documented spec.

use std::time::Duration;

use platypus_core::device::CloneSpec;
use platypus_serial::{list_ports, read_ft60_image, CloneTimeouts, Progress, SerialPort};

struct Printer {
    last: usize,
}
impl Progress for Printer {
    fn update(&mut self, bytes: usize, total: usize) {
        // Throttle to ~every 2 KiB so the log stays readable.
        if bytes - self.last >= 2048 || bytes >= total {
            eprintln!("  received {bytes} / {total} bytes");
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
    let out = args.next().unwrap_or_else(|| "ft60.img".to_string());

    eprintln!("available ports: {:?}", list_ports());
    let Some(port_path) = pick_port(port_arg) else {
        eprintln!("no serial port found — plug in the clone cable");
        std::process::exit(1);
    };
    eprintln!("opening {port_path} @ 9600 8N1");

    let spec = CloneSpec::FT60;
    let mut port =
        match SerialPort::open(port_path.as_ref(), spec.baud, Duration::from_millis(1000)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("open failed: {e}");
                std::process::exit(1);
            }
        };

    eprintln!("waiting for the radio… arm CLONE (MONI+power, DIAL→CLONE, F/W), then hold PTT.");
    eprintln!("(sends only 0x06 ACK handshake bytes — no memory is written to the radio)");
    let mut prog = Printer { last: 0 };
    match read_ft60_image(&mut port, &spec, CloneTimeouts::default(), &mut prog) {
        Ok(image) => {
            if let Err(e) = std::fs::write(&out, &image) {
                eprintln!("failed to write {out}: {e}");
                std::process::exit(1);
            }
            let magic = String::from_utf8_lossy(&image[..spec.magic.len().min(image.len())]);
            let nonzero = image.iter().filter(|&&b| b != 0).count();
            let density = if image.is_empty() {
                0.0
            } else {
                100.0 * nonzero as f64 / image.len() as f64
            };
            let head: String = image.iter().take(32).map(|b| format!("{b:02x} ")).collect();
            eprintln!("\nDONE");
            eprintln!("  saved     {out}");
            eprintln!(
                "  bytes     {} (spec image_size {}, wire_len {})",
                image.len(),
                spec.image_size,
                spec.wire_len()
            );
            eprintln!(
                "  magic     {:?} {}",
                magic,
                if spec.header_matches(&image) {
                    "✓ AH017"
                } else {
                    "✗ (expected AH017)"
                }
            );
            eprintln!(
                "  nonzero   {nonzero}/{} ({density:.0}% dense — a real image is ~100%)",
                image.len()
            );
            eprintln!("  head      {head}");
            if image.len() < spec.image_size / 2 {
                eprintln!(
                    "\n  → This is too short + not AH017: the radio didn't stream its memory."
                );
                eprintln!(
                    "    Most likely the radio isn't actually clone-SENDING (or a cable issue)."
                );
            }
        }
        Err(e) => {
            eprintln!("clone read failed: {e}");
            std::process::exit(1);
        }
    }
}
