// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Serial **Remote Command** protocol codec (phase-2 live control).
//!
//! Pure encode/parse over the `BCDx36HP`-family remote-command protocol (Uniden
//! Remote Command Spec V2.00; see [`docs/radios/sds150.md`](../../../docs/radios/sds150.md)).
//! This layer knows *what the bytes mean* — it does **no I/O**. A transport (the
//! planned `serialport` crate) opens the USB virtual serial port and pumps the
//! [`Command::encode`] lines out / feeds response lines to [`parse_response`]; that
//! transport is the format-agnostic sibling of the SD-card writer in the device-class
//! design ([`docs/architecture.md`](../../../docs/architecture.md)).
//!
//! Wire format: ASCII, comma-delimited, **CR-terminated** (`\r`) — e.g. `LCR\r` →
//! `LCR,41.582,-85.834,15\r`. One response (`GCS`) is LF-terminated; the parser
//! trims both. The radio must be in **Serial Port** USB mode.

/// The command-line terminator the radio expects.
pub const TERMINATOR: char = '\r';

/// A command sent to the radio. `encode()` renders the wire line (terminator
/// included); values like a `KEY` code/mode are opaque strings this codec just frames
/// (their vocabulary is the spec's "key code" sheet, not our concern here).
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `MDL` — get the model name (the handshake; SDS150 → `SDS150GBT`).
    Model,
    /// `VER` — get the firmware version.
    Version,
    /// `VOL` — get the current volume level.
    GetVolume,
    /// `VOL,<level>` — set volume (`0`–`15` on the SDS150).
    SetVolume(u8),
    /// `SQL` — get the current squelch level.
    GetSquelch,
    /// `SQL,<level>` — set squelch (`0`–`15` on the SDS150).
    SetSquelch(u8),
    /// `LCR` — get the scanner's location + range.
    GetLocation,
    /// `LCR,<lat>,<lon>,<range>` — **set** the scanner's location + range live
    /// (degrees / miles). The location-first serial feature.
    SetLocation { lat: f64, lon: f64, range_mi: f64 },
    /// `KEY,<code>,<mode>` — press a keypad key (code/mode per the spec's key sheet).
    PushKey { code: String, mode: String },
    /// `STS` — get the current display status (the raw screen lines).
    Status,
    /// `GCS` — get charge/battery status.
    ChargeStatus,
    /// `KAL` — keep-alive (the radio sends **no** response).
    KeepAlive,
    /// `POF` — power the radio off.
    PowerOff,
}

impl Command {
    /// The wire line to send, terminator included.
    pub fn encode(&self) -> String {
        let body = match self {
            Command::Model => "MDL".to_string(),
            Command::Version => "VER".to_string(),
            Command::GetVolume => "VOL".to_string(),
            Command::SetVolume(l) => format!("VOL,{l}"),
            Command::GetSquelch => "SQL".to_string(),
            Command::SetSquelch(l) => format!("SQL,{l}"),
            Command::GetLocation => "LCR".to_string(),
            Command::SetLocation { lat, lon, range_mi } => {
                format!(
                    "LCR,{},{},{}",
                    fmt_f64(*lat),
                    fmt_f64(*lon),
                    fmt_f64(*range_mi)
                )
            }
            Command::PushKey { code, mode } => format!("KEY,{code},{mode}"),
            Command::Status => "STS".to_string(),
            Command::ChargeStatus => "GCS".to_string(),
            Command::KeepAlive => "KAL".to_string(),
            Command::PowerOff => "POF".to_string(),
        };
        format!("{body}{TERMINATOR}")
    }

    /// Whether the radio replies (keep-alive is fire-and-forget).
    pub fn expects_response(&self) -> bool {
        !matches!(self, Command::KeepAlive)
    }
}

/// A parsed response line from the radio.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// `MDL,<model>` — the model name.
    Model(String),
    /// `VER,<version>`.
    Version(String),
    /// `VOL,<level>` — the current volume.
    Volume(u8),
    /// `SQL,<level>` — the current squelch.
    Squelch(u8),
    /// `LCR,<lat>,<lon>,<range>` — the scanner's location + range.
    Location { lat: f64, lon: f64, range_mi: f64 },
    /// `GCS,…` — charge/battery telemetry.
    Charge(ChargeStatus),
    /// `STS,…` — the raw display-status fields (form flag + line chars/modes).
    Status(Vec<String>),
    /// A bare-command echo or `…,OK` — the acknowledgement to a set/action command.
    Ack,
    /// Anything we don't model yet (`GSI`/`PSI` XML, `STS` sub-parse, …): the echoed
    /// command + its raw comma fields, so callers can still inspect it.
    Raw {
        command: String,
        fields: Vec<String>,
    },
}

/// Battery / charge telemetry from a `GCS` response.
#[derive(Debug, Clone, PartialEq)]
pub struct ChargeStatus {
    /// `CST` charge state (0=no charge, 4=full, 6=charging, …).
    pub state: u8,
    /// Battery voltage, millivolts.
    pub millivolts: u32,
    /// Remaining capacity, percent.
    pub percent: u8,
    /// Current, mA (positive = charging, negative = discharging).
    pub current_ma: i32,
    /// Battery temperature, °C.
    pub temp_c: f64,
}

/// Parse one response line into a typed [`Response`]. Leading/trailing whitespace and
/// the `\r`/`\n` terminator are trimmed. Unrecognized shapes fall back to
/// [`Response::Raw`] rather than failing — a codec never panics on the wire.
pub fn parse_response(line: &str) -> Response {
    let line = line.trim_end_matches(['\r', '\n']).trim();

    // GCS carries `key=value` fields, not positional ones.
    if let Some(rest) = line.strip_prefix("GCS,") {
        if let Some(c) = parse_charge(rest) {
            return Response::Charge(c);
        }
        return raw("GCS", rest.split(','));
    }

    let mut parts = line.split(',');
    let command = parts.next().unwrap_or("");
    let fields: Vec<&str> = parts.collect();

    match (command, fields.as_slice()) {
        ("MDL", [m]) => Response::Model((*m).to_string()),
        ("VER", [v]) => Response::Version((*v).to_string()),
        ("VOL", [l]) => l
            .parse::<u8>()
            .ok()
            .map_or_else(|| raw(command, fields.iter().copied()), Response::Volume),
        ("SQL", [l]) => l
            .parse::<u8>()
            .ok()
            .map_or_else(|| raw(command, fields.iter().copied()), Response::Squelch),
        ("LCR", [lat, lon, rng]) => match (lat.parse(), lon.parse(), rng.parse()) {
            (Ok(lat), Ok(lon), Ok(range_mi)) => Response::Location { lat, lon, range_mi },
            _ => raw(command, fields.iter().copied()),
        },
        // A set/action ack: `CMD,OK` or a bare command echo (e.g. `VOL` after a set).
        (_, ["OK"]) => Response::Ack,
        ("VOL" | "SQL", []) => Response::Ack,
        ("STS", _) => Response::Status(fields.iter().map(|s| s.to_string()).collect()),
        _ => raw(command, fields.iter().copied()),
    }
}

/// Format an `f64` for the wire without spurious trailing zeros (Rust's `Display`).
fn fmt_f64(v: f64) -> String {
    format!("{v}")
}

fn raw<'a>(command: &str, fields: impl Iterator<Item = &'a str>) -> Response {
    Response::Raw {
        command: command.to_string(),
        fields: fields.map(|s| s.to_string()).collect(),
    }
}

/// Parse the `GCS` body `CST=4,VOLT=4184mV:100%,CURR=0000mA,TEMP= 27.65C`.
fn parse_charge(body: &str) -> Option<ChargeStatus> {
    let mut state = None;
    let mut millivolts = None;
    let mut percent = None;
    let mut current_ma = None;
    let mut temp_c = None;
    for field in body.split(',') {
        let (key, val) = field.split_once('=')?;
        match key.trim() {
            "CST" => state = val.trim().parse().ok(),
            "VOLT" => {
                // `4184mV:100%`
                let (mv, pct) = val.split_once(':')?;
                millivolts = mv.trim().trim_end_matches("mV").trim().parse().ok();
                percent = pct.trim().trim_end_matches('%').trim().parse().ok();
            }
            "CURR" => current_ma = val.trim().trim_end_matches("mA").trim().parse().ok(),
            "TEMP" => temp_c = val.trim().trim_end_matches('C').trim().parse().ok(),
            _ => {}
        }
    }
    Some(ChargeStatus {
        state: state?,
        millivolts: millivolts?,
        percent: percent?,
        current_ma: current_ma?,
        temp_c: temp_c?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_commands() {
        assert_eq!(Command::Model.encode(), "MDL\r");
        assert_eq!(Command::Version.encode(), "VER\r");
        assert_eq!(Command::GetVolume.encode(), "VOL\r");
        assert_eq!(Command::SetVolume(9).encode(), "VOL,9\r");
        assert_eq!(Command::SetSquelch(3).encode(), "SQL,3\r");
        assert_eq!(Command::GetLocation.encode(), "LCR\r");
        assert_eq!(
            Command::SetLocation {
                lat: 41.582,
                lon: -85.834,
                range_mi: 15.0
            }
            .encode(),
            "LCR,41.582,-85.834,15\r"
        );
        assert_eq!(
            Command::PushKey {
                code: "H".into(),
                mode: "P".into()
            }
            .encode(),
            "KEY,H,P\r"
        );
        assert_eq!(Command::ChargeStatus.encode(), "GCS\r");
        assert_eq!(Command::KeepAlive.encode(), "KAL\r");
        assert_eq!(Command::PowerOff.encode(), "POF\r");
    }

    #[test]
    fn keep_alive_expects_no_response() {
        assert!(!Command::KeepAlive.expects_response());
        assert!(Command::Model.expects_response());
    }

    #[test]
    fn parses_scalar_responses() {
        assert_eq!(
            parse_response("MDL,SDS150GBT\r"),
            Response::Model("SDS150GBT".into())
        );
        assert_eq!(
            parse_response("VER,Version 1.10.00\r"),
            Response::Version("Version 1.10.00".into())
        );
        assert_eq!(parse_response("VOL,7\r"), Response::Volume(7));
        assert_eq!(parse_response("SQL,0\r"), Response::Squelch(0));
        // Bare echo = set-ack; `CMD,OK` = action-ack.
        assert_eq!(parse_response("VOL\r"), Response::Ack);
        assert_eq!(parse_response("KEY,OK\r"), Response::Ack);
        assert_eq!(parse_response("POF,OK\r"), Response::Ack);
        assert_eq!(parse_response("LCR,OK\r"), Response::Ack);
    }

    #[test]
    fn parses_location() {
        assert_eq!(
            parse_response("LCR,41.582,-85.834,15\r"),
            Response::Location {
                lat: 41.582,
                lon: -85.834,
                range_mi: 15.0
            }
        );
        // Round-trips against the encoder (the radio echoes the same LCR line back).
        let cmd = Command::SetLocation {
            lat: 45.0,
            lon: -100.0,
            range_mi: 25.0,
        };
        assert_eq!(
            parse_response(&cmd.encode()),
            Response::Location {
                lat: 45.0,
                lon: -100.0,
                range_mi: 25.0
            }
        );
    }

    #[test]
    fn parses_charge_status() {
        let r = parse_response("GCS,CST=4,VOLT=4184mV:100%,CURR=0000mA,TEMP= 27.65C\n");
        assert_eq!(
            r,
            Response::Charge(ChargeStatus {
                state: 4,
                millivolts: 4184,
                percent: 100,
                current_ma: 0,
                temp_c: 27.65,
            })
        );
        // Discharging = negative current.
        let Response::Charge(c) =
            parse_response("GCS,CST=6,VOLT=3900mV:64%,CURR=-250mA,TEMP=30.0C\n")
        else {
            panic!("expected charge");
        };
        assert_eq!(c.current_ma, -250);
        assert_eq!(c.percent, 64);
    }

    #[test]
    fn status_and_unknown_fall_back() {
        // STS keeps its raw fields.
        let Response::Status(f) = parse_response("STS,011000,Line1 text,0,Line2,1\r") else {
            panic!("expected status");
        };
        assert_eq!(f.first().map(String::as_str), Some("011000"));
        assert_eq!(f.len(), 5);
        // An unmodeled command → Raw, never a panic.
        assert_eq!(
            parse_response("XYZ,a,b\r"),
            Response::Raw {
                command: "XYZ".into(),
                fields: vec!["a".into(), "b".into()]
            }
        );
    }
}
