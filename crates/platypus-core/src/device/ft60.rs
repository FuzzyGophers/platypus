// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Yaesu FT-60R (`AH017`) clone-image support — a `CloneImageProfile`-class radio.
//!
//! This radio has no filesystem: it is programmed by moving one fixed-size EEPROM image
//! over a serial clone cable. This module owns the **pure, I/O-free** facts and codec:
//!
//! - [`CloneSpec`] — the transport constants (baud, image size, block framing, ACK byte,
//!   model magic) a transport crate needs to run the clone handshake. No I/O here.
//! - [`Ft60Image`] — `decode`/`encode` of the 28,617-byte image ↔ a typed model, with a
//!   byte-exact round-trip gate and verbatim preservation of every region we don't model, plus
//!   surgical, change-gated [`Ft60Image::apply_channels`] edits for writing back.
//!
//! **Spec-derived.** Every constant/offset below is a *fact* about the radio, cross-referenced
//! from the FT-60 device doc ([`docs/radios/ft60.md`](../../../../docs/radios/ft60.md)) and the
//! manufacturer spec. Facts (offsets, sizes, protocol constants) are not copyrightable; this
//! codec is implemented independently — **no reference's code is copied**. Our core stays
//! GPL-2.0-only. See `CREDITS.md`.

use super::profile::{
    CloneCapacity, CloneFieldOptions, CloneImageProfile, FieldOption, ProgramSupport, RadioClass,
    RadioProfile, ToneValueKind,
};

/// The FT-60R as a registered radio profile (clone-image class). Its transport spec +
/// image detection live here; the binary image codec is [`Ft60Image`] below. This is the
/// template to copy when adding another clone-image radio — a new `device/<model>.rs`
/// implementing [`RadioProfile`] + [`CloneImageProfile`], plus a `register()` line.
#[derive(Debug, Default, Clone, Copy)]
pub struct Ft60;

impl Ft60 {
    pub fn new() -> Self {
        Ft60
    }
}

impl RadioProfile for Ft60 {
    fn id(&self) -> &'static str {
        "ft60r"
    }

    fn product_name(&self) -> &'static str {
        "FT-60R"
    }

    fn maker(&self) -> &'static str {
        "Yaesu"
    }

    fn transport(&self) -> &'static str {
        "serial clone"
    }

    fn class(&self) -> RadioClass {
        RadioClass::CloneImage
    }

    fn program_support(&self) -> ProgramSupport {
        // An analog FM/NFM/AM handheld: conventional memories only — no trunking, no digital.
        ProgramSupport::analog_conventional()
    }

    fn as_clone_image(&self) -> Option<&dyn CloneImageProfile> {
        Some(self)
    }
}

impl CloneImageProfile for Ft60 {
    fn clone_spec(&self) -> CloneSpec {
        CloneSpec::FT60
    }

    fn capacity(&self) -> CloneCapacity {
        CloneCapacity {
            channels: MEM_COUNT,
            banks: BANK_COUNT,
            name_len: NAME_LEN,
        }
    }

    fn field_options(&self) -> CloneFieldOptions {
        // Built from this module's fact constants — the same orderings the codec uses, so
        // a UI's picker index round-trips through the writer with no separate table.
        let list = |labels: &[&str]| -> Vec<FieldOption> {
            labels
                .iter()
                .enumerate()
                .map(|(i, l)| FieldOption::new(*l, i as u8))
                .collect()
        };
        // Tone modes: label from TMODES ("" shown as "Off"); code = TMODES index; the value
        // field each needs (CTCSS 1–3, DCS 4–5, both for the cross modes 6–7, none otherwise).
        let tone_modes = TMODES
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let value_kind = match i {
                    1..=3 => ToneValueKind::Ctcss,
                    4..=5 => ToneValueKind::Dcs,
                    6..=7 => ToneValueKind::Cross,
                    _ => ToneValueKind::None,
                };
                let label = if l.is_empty() { "Off" } else { l };
                FieldOption {
                    label: label.to_string(),
                    code: i as u8,
                    value_kind,
                }
            })
            .collect();
        // Steps: label formatted from STEPS_HZ ("5 kHz" / "12.5 kHz"); code = STEPS index.
        let steps = STEPS_HZ
            .iter()
            .enumerate()
            .map(|(i, hz)| {
                let khz = *hz as f64 / 1000.0;
                let label = if khz.fract() == 0.0 {
                    format!("{} kHz", khz as u32)
                } else {
                    format!("{khz} kHz")
                };
                FieldOption::new(label, i as u8)
            })
            .collect();
        // Duplexes: the four the form offers, each with its on-radio DUPLEX index (0 simplex,
        // 2 −, 3 +, 4 split). Indices 1 and 5 ("off") aren't user-selectable.
        let duplexes = vec![
            FieldOption::new("Simplex", 0),
            FieldOption::new("−", 2),
            FieldOption::new("+", 3),
            FieldOption::new("Split", 4),
        ];
        CloneFieldOptions {
            modes: list(&MODES),
            tone_modes,
            steps,
            powers: list(&POWER_LEVELS),
            duplexes,
        }
    }
}

/// The serial clone-transport constants for the FT-60. A transport crate uses these to run
/// the read/write handshake; this struct carries no I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneSpec {
    /// Serial line rate (8N1).
    pub baud: u32,
    /// Total decoded EEPROM image size, in bytes.
    pub image_size: usize,
    /// Leading header bytes carrying the model magic.
    pub header_len: usize,
    /// Payload block size the radio streams between ACKs.
    pub block_size: usize,
    /// Number of payload blocks.
    pub block_count: usize,
    /// The byte the PC sends to acknowledge each received block.
    pub ack: u8,
    /// The model-identifying magic at the start of the stream.
    pub magic: &'static [u8],
}

impl CloneSpec {
    /// The FT-60R (`AH017`) clone spec (facts from `docs/radios/ft60.md`). The exact wire
    /// framing is verified against hardware during bring-up; these are the documented values.
    pub const FT60: CloneSpec = CloneSpec {
        baud: 9600,
        image_size: 28_617,
        header_len: 8,
        block_size: 64,
        block_count: 448,
        ack: 0x06,
        magic: b"AH017",
    };

    /// Total bytes expected on the wire = header + all payload blocks.
    pub const fn wire_len(&self) -> usize {
        self.header_len + self.block_count * self.block_size
    }

    /// Does `header` begin with this radio's model magic? (Used to confirm a clone-read is
    /// talking to the expected model before trusting the payload.)
    pub fn header_matches(&self, header: &[u8]) -> bool {
        header.len() >= self.magic.len() && &header[..self.magic.len()] == self.magic
    }
}

// ---------------------------------------------------------------------------
// Image codec — decode the 28,617-byte clone image ↔ a typed model.
//
// Round-trip safety: `Ft60Image` keeps the whole raw image, so `encode()` is the identity
// (every byte, incl. regions we don't interpret, is preserved verbatim — the same "never
// overwrite what we don't understand" contract as the SDS150 F-List block). The channel
// *interpretation* (`channels()`) is a read-only view for the UI, separate from the byte
// preservation. Offsets/bitfield layout/enum orderings are facts from docs/radios/ft60.md;
// implemented independently (spec-derived).
// ---------------------------------------------------------------------------

// Fixed offsets into the image (facts).
const MEM_BASE: usize = 0x0248; // standard memories 000–999
const MEM_LEN: usize = 16; // bytes per `struct mem`
const MEM_COUNT: usize = 1000;
const NAME_BASE: usize = 0x4708; // channel alpha tags
const NAME_LEN: usize = 6; // 6 chars per name…
const NAME_STRIDE: usize = 8; // …stored in an 8-byte slot (6 chars + trailing 0x80 0x80)
const BANK_BASE: usize = 0x69C8; // 10 banks × 128-byte bitmap (1 bit/channel)
const BANK_STRIDE: usize = 128;
const BANK_COUNT: usize = 10;
const SKIP_BASE: usize = 0x6EC8; // 2 bits/channel

/// PMS (programmable memory scan) band-edge records — 100 of the same 16-byte `struct mem`,
/// contiguous right after the 1000 standard memories (`0x0248 + 1000*16 = 0x40C8`). They form
/// 50 lower/upper pairs (L01/U01…L50/U50), interleaved: pair *p* = records `2p` (lower) and
/// `2p+1` (upper). Band edges carry only a frequency + step (no name/bank/tone).
const PMS_BASE: usize = 0x40C8;
const PMS_COUNT: usize = 100; // 50 interleaved lower/upper pairs

/// Repeater-shift magnitude field: a big-endian `u16` at record bytes 11–12 (the top bit is
/// reserved, the low 15 bits are the offset), counted in 50 kHz steps. Verified against real
/// hardware: every channel stores `0x000C` = 12 → 600 kHz, the standard 2 m repeater shift.
const OFFSET_LO: usize = 11;
const OFFSET_UNIT_HZ: u64 = 50_000;

/// Industry-standard CTCSS tones (Hz ×10), indexed by the radio's `tone` field.
#[rustfmt::skip]
const CTCSS: [u16; 50] = [
     670,  693,  719,  744,  770,  797,  825,  854,  885,  915,
     948,  974, 1000, 1035, 1072, 1109, 1148, 1188, 1230, 1273,
    1318, 1365, 1413, 1462, 1514, 1567, 1598, 1622, 1655, 1679,
    1713, 1738, 1773, 1799, 1835, 1862, 1899, 1928, 1966, 1995,
    2035, 2065, 2107, 2181, 2257, 2291, 2336, 2418, 2503, 2541,
];

/// Industry-standard DCS codes (octal), indexed by the radio's `dtcs` field.
#[rustfmt::skip]
const DCS: [u16; 104] = [
     23,  25,  26,  31,  32,  36,  43,  47,  51,  53,  54,  65,  71,  72,  73,  74,
    114, 115, 116, 122, 125, 131, 132, 134, 143, 145, 152, 155, 156, 162, 165, 172,
    174, 205, 212, 223, 225, 226, 243, 244, 245, 246, 251, 252, 255, 261, 263, 265,
    266, 271, 274, 306, 311, 315, 325, 331, 332, 343, 346, 351, 356, 364, 365, 371,
    411, 412, 413, 423, 431, 432, 445, 446, 452, 454, 455, 462, 464, 465, 466, 503,
    506, 516, 523, 526, 532, 546, 565, 606, 612, 624, 627, 631, 632, 654, 662, 664,
    703, 712, 723, 731, 732, 734, 743, 754,
];

/// Squelch tone as interpreted from a memory record.
#[derive(Debug, Clone, PartialEq)]
pub enum Tone {
    None,
    /// CTCSS tone in Hz ×10 (e.g. 1000 = 100.0 Hz).
    Ctcss(u16),
    /// DCS/DTCS octal code.
    Dcs(u16),
    /// A cross tone mode (`Tone->DTCS` / `DTCS->Tone`) that carries **both** a CTCSS tone
    /// (Hz ×10) and a DCS code — the record stores each in its own field (byte 8 / byte 9), so
    /// the model must too or editing such a channel would drop one half.
    Cross {
        ctcss: u16,
        dcs: u16,
    },
}

/// One decoded FT-60 standard memory channel (a used slot). Frequencies in Hz.
#[derive(Debug, Clone, PartialEq)]
pub struct Ft60Channel {
    /// 0-based slot (0..1000; UI shows slot+1).
    pub slot: u16,
    /// 6-char name (best-effort decode of the radio's charset).
    pub name: String,
    pub rx_hz: u64,
    /// Repeater duplex: "", "-", "+", "split", "off".
    pub duplex: &'static str,
    /// Offset in Hz (for +/- duplex).
    pub offset_hz: u64,
    /// TX frequency in Hz — used for "split" duplex (bytes 5–7); 0 otherwise.
    pub tx_hz: u64,
    /// FM / NFM / AM.
    pub mode: &'static str,
    /// Tone-mode sub-kind, one of [`TMODES`] ("", "Tone", "TSQL", "TSQL-R", "DTCS", …). The
    /// CTCSS/DCS value lives in `tone`.
    pub tone_mode: &'static str,
    pub tone: Tone,
    /// TX power index: 0 = High, 1 = Mid, 2 = Low (byte 8 bits 6–7).
    pub power: u8,
    /// Tuning-step index into [`STEPS_HZ`] (0 = 5 kHz … 7 = 100 kHz).
    pub step: u8,
    /// Banks (0-based) this channel belongs to.
    pub banks: Vec<u8>,
    /// Skip flag: "" / "S" (skip) / "P" (preferred).
    pub skip: &'static str,
}

/// A decoded FT-60 clone image. Owns the raw bytes so `encode` is byte-exact.
#[derive(Debug, Clone)]
pub struct Ft60Image {
    raw: Vec<u8>,
}

impl Ft60Image {
    /// Validate + wrap a clone image. Requires the model magic and the full image size.
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let spec = CloneSpec::FT60;
        if bytes.len() < spec.image_size {
            return Err("image too short");
        }
        if !spec.header_matches(bytes) {
            return Err("not an FT-60 image (bad magic)");
        }
        Ok(Ft60Image {
            raw: bytes[..spec.image_size].to_vec(),
        })
    }

    /// The image bytes back, with the trailing checksum recomputed. For an unmodified image
    /// this is the identity (`decode → encode == input`) — the round-trip safety contract —
    /// because the stored checksum already equals `sum(bytes[..last]) & 0xFF` (verified against
    /// real hardware images). After [`apply_channels`] it carries the edits + a valid checksum.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.raw.clone();
        let n = out.len();
        out[n - 1] = checksum(&out);
        out
    }

    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// Interpret the 1000 standard memories into the used channels (read-only view).
    pub fn channels(&self) -> Vec<Ft60Channel> {
        let mut out = Vec::new();
        for c in 0..MEM_COUNT {
            let rec = &self.raw[MEM_BASE + c * MEM_LEN..MEM_BASE + (c + 1) * MEM_LEN];
            let b0 = rec[0];
            let used = b0 & 0x80 != 0;
            if !used {
                continue;
            }
            let narrow = b0 & 0x20 != 0;
            let am = b0 & 0x10 != 0;
            let duplex = DUPLEX.get((b0 & 0x0F) as usize).copied().unwrap_or("");

            let rx_hz = decode_freq(&rec[1..4]);
            let offset_hz = self.channel_offset(c);
            let tone = self.channel_tone(c);

            let mode = if am {
                "AM"
            } else if narrow {
                "NFM"
            } else {
                "FM"
            };

            out.push(Ft60Channel {
                slot: c as u16,
                name: self.channel_name(c),
                rx_hz,
                duplex,
                offset_hz,
                tx_hz: decode_freq(&rec[5..8]),
                mode,
                tone_mode: self.channel_tone_mode(c),
                tone,
                power: self.channel_power(c),
                step: self.channel_step(c),
                banks: self.channel_banks(c),
                skip: self.channel_skip(c),
            });
        }
        out
    }

    fn channel_name(&self, c: usize) -> String {
        let start = NAME_BASE + c * NAME_STRIDE;
        let raw = &self.raw[start..start + NAME_LEN];
        // Best-effort: the FT-60 uses a custom charset (calibrated against hardware). For now
        // render printable-looking bytes and trim; refined once the charset is confirmed.
        raw.iter()
            .map(|&b| charset_byte(b))
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn channel_banks(&self, c: usize) -> Vec<u8> {
        let mut banks = Vec::new();
        for b in 0..BANK_COUNT {
            let byte = self.raw[BANK_BASE + b * BANK_STRIDE + c / 8];
            if byte & (1 << (c % 8)) != 0 {
                banks.push(b as u8);
            }
        }
        banks
    }

    fn channel_skip(&self, c: usize) -> &'static str {
        let bits = (self.raw[SKIP_BASE + c / 4] >> ((c % 4) * 2)) & 0x03;
        match bits {
            1 => "S",
            2 => "P",
            _ => "",
        }
    }

    /// The tone currently programmed in slot `c` (the same interpretation `channels()` uses).
    fn channel_tone(&self, c: usize) -> Tone {
        let rec = MEM_BASE + c * MEM_LEN;
        let tmode = (self.raw[rec + 4] & 0x07) as usize;
        let tone_idx = (self.raw[rec + 8] & 0x3F) as usize;
        let dtcs_idx = (self.raw[rec + 9] & 0x7F) as usize;
        let ctcss = || CTCSS.get(tone_idx).copied().unwrap_or(0);
        let dcs = || DCS.get(dtcs_idx).copied().unwrap_or(0);
        match tmode {
            1..=3 => Tone::Ctcss(ctcss()),
            4..=5 => Tone::Dcs(dcs()),
            // Cross modes (6 = Tone->DTCS, 7 = DTCS->Tone) carry both values.
            6..=7 => Tone::Cross {
                ctcss: ctcss(),
                dcs: dcs(),
            },
            _ => Tone::None,
        }
    }

    /// The duplex string currently programmed in slot `c` (byte 0's low nibble).
    fn channel_duplex(&self, c: usize) -> &'static str {
        let idx = (self.raw[MEM_BASE + c * MEM_LEN] & 0x0F) as usize;
        DUPLEX.get(idx).copied().unwrap_or("")
    }

    /// The tone-mode string currently programmed in slot `c` (byte 4 low 3 bits).
    fn channel_tone_mode(&self, c: usize) -> &'static str {
        let idx = (self.raw[MEM_BASE + c * MEM_LEN + 4] & 0x07) as usize;
        TMODES.get(idx).copied().unwrap_or("")
    }

    /// The TX power index in slot `c` (byte 8 bits 6–7): 0 = High, 1 = Mid, 2 = Low.
    fn channel_power(&self, c: usize) -> u8 {
        (self.raw[MEM_BASE + c * MEM_LEN + 8] >> 6) & 0x03
    }

    /// The tuning-step index in slot `c` (byte 4 bits 4–6).
    fn channel_step(&self, c: usize) -> u8 {
        (self.raw[MEM_BASE + c * MEM_LEN + 4] >> 4) & 0x07
    }

    /// The repeater-shift magnitude in Hz for slot `c` (bytes 11–12, 50 kHz steps).
    fn channel_offset(&self, c: usize) -> u64 {
        let rec = MEM_BASE + c * MEM_LEN;
        let raw = (((self.raw[rec + OFFSET_LO] as u16) << 8)
            | self.raw[rec + OFFSET_LO + 1] as u16)
            & 0x7FFF;
        raw as u64 * OFFSET_UNIT_HZ
    }

    // -- Write side (Phase D) ------------------------------------------------
    //
    // Each setter is the exact inverse of the matching decoder and touches **only the bits it
    // owns** (read-modify-write), so every `unknown*` bit and every field we don't model is
    // preserved verbatim. Fields whose byte layout hides bits we don't fully invert (name pad,
    // the exact `tmode` sub-kind) are written **only when the value changed** vs. the decoded
    // value — so re-applying the decoded channels is a byte-for-byte no-op (the round-trip gate).

    /// Apply a desired channel set onto the image: slots present in `desired` are programmed
    /// (surgically), slots absent are cleared to empty. Recomputes the checksum. Re-applying
    /// `self.channels()` is a no-op (round-trip-safe); only genuinely-edited bytes move.
    pub fn apply_channels(&mut self, desired: &[Ft60Channel]) {
        let mut by_slot: Vec<Option<&Ft60Channel>> = vec![None; MEM_COUNT];
        for ch in desired {
            let s = ch.slot as usize;
            if s < MEM_COUNT {
                by_slot[s] = Some(ch);
            }
        }
        for (slot, entry) in by_slot.iter().enumerate() {
            match entry {
                Some(ch) => {
                    self.set_used(slot, true);
                    self.set_freq(slot, ch.rx_hz);
                    self.set_mode(slot, ch.mode);
                    self.set_duplex(slot, ch.duplex);
                    self.set_offset(slot, ch.offset_hz);
                    self.set_tx_freq(slot, ch.tx_hz);
                    self.set_tone(slot, ch.tone_mode, &ch.tone);
                    self.set_power(slot, ch.power);
                    self.set_step(slot, ch.step);
                    self.set_name(slot, &ch.name);
                    self.set_banks(slot, &ch.banks);
                    self.set_skip(slot, ch.skip);
                }
                None => self.clear_slot(slot),
            }
        }
        let n = self.raw.len();
        self.raw[n - 1] = checksum(&self.raw);
    }

    /// Set/clear the "channel used" bit (add/remove a slot).
    fn set_used(&mut self, c: usize, used: bool) {
        let rec = MEM_BASE + c * MEM_LEN;
        if used {
            self.raw[rec] |= 0x80;
        } else {
            self.raw[rec] &= !0x80;
        }
    }

    /// Write the RX frequency (bytes 1–3), the exact inverse of [`decode_freq`].
    fn set_freq(&mut self, c: usize, hz: u64) {
        let rec = MEM_BASE + c * MEM_LEN;
        write_bcd_freq(&mut self.raw[rec + 1..rec + 4], hz);
    }

    /// Write the TX frequency (bytes 5–7) — used for "split" duplex. Change-gated, so simplex
    /// channels (which decode to 0 here) are never touched.
    fn set_tx_freq(&mut self, c: usize, hz: u64) {
        let rec = MEM_BASE + c * MEM_LEN;
        if decode_freq(&self.raw[rec + 5..rec + 8]) == hz {
            return;
        }
        write_bcd_freq(&mut self.raw[rec + 5..rec + 8], hz);
    }

    /// Write the TX power index (byte 8 bits 6–7), preserving the CTCSS index. Change-gated.
    fn set_power(&mut self, c: usize, power: u8) {
        if self.channel_power(c) == power {
            return;
        }
        let rec = MEM_BASE + c * MEM_LEN;
        self.raw[rec + 8] = (self.raw[rec + 8] & 0x3F) | ((power & 0x03) << 6);
    }

    /// Write the tuning-step index (byte 4 bits 4–6), preserving the tmode + unknown bits.
    fn set_step(&mut self, c: usize, step: u8) {
        if self.channel_step(c) == step {
            return;
        }
        let rec = MEM_BASE + c * MEM_LEN;
        self.raw[rec + 4] = (self.raw[rec + 4] & !0x70) | ((step & 0x07) << 4);
    }

    /// Write the mode (the narrow-FM / AM bits of byte 0, `0x20`/`0x10`); preserves the
    /// in-use flag, the reserved bits, and the duplex nibble.
    fn set_mode(&mut self, c: usize, mode: &str) {
        let rec = MEM_BASE + c * MEM_LEN;
        let mut b = self.raw[rec] & !0x30;
        match mode {
            "AM" => b |= 0x10,
            "NFM" => b |= 0x20,
            _ => {}
        }
        self.raw[rec] = b;
    }

    /// Write the repeater duplex (byte 0's low nibble), preserving `used`/mode bits. Only writes
    /// when the duplex string changed — so the two "" encodings (index 0/1) aren't disturbed.
    fn set_duplex(&mut self, c: usize, duplex: &str) {
        if self.channel_duplex(c) == duplex {
            return;
        }
        let idx = DUPLEX.iter().position(|&d| d == duplex).unwrap_or(0) as u8;
        let rec = MEM_BASE + c * MEM_LEN;
        self.raw[rec] = (self.raw[rec] & 0xF0) | (idx & 0x0F);
    }

    /// Write the repeater-shift magnitude (bytes 11–12, 50 kHz steps), preserving the reserved
    /// top bit. Only writes when the offset changed (robust to the 50 kHz inference).
    fn set_offset(&mut self, c: usize, hz: u64) {
        if self.channel_offset(c) == hz {
            return;
        }
        let units = (hz / OFFSET_UNIT_HZ).min(0x7FFF) as u16;
        let rec = MEM_BASE + c * MEM_LEN;
        self.raw[rec + OFFSET_LO] =
            (self.raw[rec + OFFSET_LO] & 0x80) | ((units >> 8) as u8 & 0x7F);
        self.raw[rec + OFFSET_LO + 1] = (units & 0xFF) as u8;
    }

    /// Write the tone mode (byte 4 low 3 bits) + the CTCSS/DCS value (byte 8 low 6 / byte 9
    /// low 7). Each half is change-gated, so an unchanged channel keeps its exact tmode sub-kind
    /// and value bytes. Preserves the `power` bits (byte 8) and the reserved bits of byte 9.
    fn set_tone(&mut self, c: usize, tone_mode: &str, tone: &Tone) {
        let rec = MEM_BASE + c * MEM_LEN;
        let cur_tone = self.channel_tone(c);
        if self.channel_tone_mode(c) != tone_mode {
            let idx = TMODES.iter().position(|&m| m == tone_mode).unwrap_or(0) as u8;
            self.raw[rec + 4] = (self.raw[rec + 4] & 0xF8) | (idx & 0x07);
        }
        if &cur_tone != tone {
            let set_ctcss = |raw: &mut [u8], hz: u16| {
                let idx = CTCSS.iter().position(|&v| v == hz).unwrap_or(0) as u8;
                raw[rec + 8] = (raw[rec + 8] & 0xC0) | (idx & 0x3F);
            };
            let set_dcs = |raw: &mut [u8], code: u16| {
                let idx = DCS.iter().position(|&v| v == code).unwrap_or(0) as u8;
                raw[rec + 9] = (raw[rec + 9] & 0x80) | (idx & 0x7F);
            };
            match tone {
                Tone::None => {} // value bytes are irrelevant when the mode carries no tone
                Tone::Ctcss(hz) => set_ctcss(&mut self.raw, *hz),
                Tone::Dcs(code) => set_dcs(&mut self.raw, *code),
                // Cross modes store both a CTCSS (byte 8) and a DCS (byte 9) value.
                Tone::Cross { ctcss, dcs } => {
                    set_ctcss(&mut self.raw, *ctcss);
                    set_dcs(&mut self.raw, *dcs);
                }
            }
        }
    }

    /// Write the 6-char name (space-padded) + the `0x80 0x80` slot trailer. Only writes when
    /// the name changed, so unchanged slots keep their exact original padding bytes.
    fn set_name(&mut self, c: usize, name: &str) {
        if self.channel_name(c) == name {
            return;
        }
        let start = NAME_BASE + c * NAME_STRIDE;
        let mut bytes = [0x24u8; NAME_LEN]; // 0x24 = space (pad)
        for (i, ch) in name.chars().take(NAME_LEN).enumerate() {
            bytes[i] = encode_charset(ch);
        }
        self.raw[start..start + NAME_LEN].copy_from_slice(&bytes);
        self.raw[start + 6] = 0x80;
        self.raw[start + 7] = 0x80;
    }

    /// Set this channel's bank membership (the per-bank bitmaps).
    fn set_banks(&mut self, c: usize, banks: &[u8]) {
        for b in 0..BANK_COUNT {
            let idx = BANK_BASE + b * BANK_STRIDE + c / 8;
            let bit = 1u8 << (c % 8);
            if banks.contains(&(b as u8)) {
                self.raw[idx] |= bit;
            } else {
                self.raw[idx] &= !bit;
            }
        }
    }

    /// Set the 2-bit skip flag (`""`/`S`/`P`).
    fn set_skip(&mut self, c: usize, skip: &str) {
        let idx = SKIP_BASE + c / 4;
        let shift = (c % 4) * 2;
        let val: u8 = match skip {
            "S" => 1,
            "P" => 2,
            _ => 0,
        };
        self.raw[idx] = (self.raw[idx] & !(0x03 << shift)) | (val << shift);
    }

    /// Mark a slot empty: clear `used` + its bank and skip bits. (Leaves the record's other
    /// bytes as-is; the radio ignores unused slots.) A no-op on an already-empty slot.
    fn clear_slot(&mut self, c: usize) {
        self.set_used(c, false);
        self.set_banks(c, &[]);
        self.set_skip(c, "");
    }

    /// The 100 PMS band-edge records (`used` + frequency + step). Interleaved lower/upper pairs.
    pub fn pms_edges(&self) -> Vec<PmsEdge> {
        (0..PMS_COUNT)
            .map(|i| {
                let rec = PMS_BASE + i * MEM_LEN;
                PmsEdge {
                    index: i as u16,
                    used: self.raw[rec] & 0x80 != 0,
                    rx_hz: decode_freq(&self.raw[rec + 1..rec + 4]),
                    step: (self.raw[rec + 4] >> 4) & 0x07,
                }
            })
            .collect()
    }

    /// Apply edited PMS edges (surgical: only `used`, frequency, and step; other bytes stay
    /// verbatim, freq/step change-gated). Recomputes the checksum. Re-applying `pms_edges()` is
    /// a no-op (round-trip-safe).
    pub fn apply_pms(&mut self, edges: &[PmsEdge]) {
        for e in edges {
            let i = e.index as usize;
            if i >= PMS_COUNT {
                continue;
            }
            let rec = PMS_BASE + i * MEM_LEN;
            if e.used {
                self.raw[rec] |= 0x80;
            } else {
                self.raw[rec] &= !0x80;
            }
            if decode_freq(&self.raw[rec + 1..rec + 4]) != e.rx_hz {
                write_bcd_freq(&mut self.raw[rec + 1..rec + 4], e.rx_hz);
            }
            if (self.raw[rec + 4] >> 4) & 0x07 != e.step {
                self.raw[rec + 4] = (self.raw[rec + 4] & !0x70) | ((e.step & 0x07) << 4);
            }
        }
        let n = self.raw.len();
        self.raw[n - 1] = checksum(&self.raw);
    }

    /// The set-mode settings as current values, indexed by [`settings_specs`] order (each is the
    /// value in the low bits of its byte).
    pub fn settings(&self) -> Vec<u8> {
        settings_specs()
            .iter()
            .map(|s| self.raw[s.offset] & s.mask)
            .collect()
    }

    /// Apply set-mode settings by [`settings_specs`] order. Change-gated per field: only a byte
    /// whose value actually changed is rewritten, and only its own bits (the reserved high bits of
    /// each byte are preserved), so re-applying decoded settings is a byte-for-byte no-op.
    pub fn apply_settings(&mut self, values: &[u8]) {
        for (s, &val) in settings_specs().iter().zip(values) {
            let val = val & s.mask;
            if self.raw[s.offset] & s.mask != val {
                self.raw[s.offset] = (self.raw[s.offset] & !s.mask) | val;
            }
        }
        let n = self.raw.len();
        self.raw[n - 1] = checksum(&self.raw);
    }
}

/// One PMS band-edge record (a scan limit). `index` is 0..100; pair *p* = indices `2p`/`2p+1`
/// (lower/upper).
#[derive(Debug, Clone, PartialEq)]
pub struct PmsEdge {
    pub index: u16,
    pub used: bool,
    pub rx_hz: u64,
    /// Tuning-step index into [`STEPS_HZ`].
    pub step: u8,
}

/// The FT-60 image checksum: the trailing byte is the low 8 bits of the sum of every preceding
/// byte (verified against real hardware captures).
fn checksum(bytes: &[u8]) -> u8 {
    bytes[..bytes.len() - 1]
        .iter()
        .fold(0u32, |a, &b| a + b as u32) as u8
}

/// Facts: the radio's enum orderings.
const DUPLEX: [&str; 6] = ["", "", "-", "+", "split", "off"];

/// Operating modes, indexed by the channel's `mode` field (byte 8 bits 4–5).
pub const MODES: [&str; 3] = ["FM", "NFM", "AM"];

/// TX power levels, indexed by the channel's `power` field (byte 8 bits 6–7).
pub const POWER_LEVELS: [&str; 3] = ["High", "Mid", "Low"];

/// Tone modes (byte 4 low 3 bits). Indices 1–3 use the CTCSS value, 4–7 use the DCS value.
pub const TMODES: [&str; 8] = [
    "",
    "Tone",
    "TSQL",
    "TSQL-R",
    "DTCS",
    "DTCS->",
    "Tone->DTCS",
    "DTCS->Tone",
];

/// Channel tuning steps in Hz, indexed by [`Ft60Channel::step`] (byte 4 bits 4–6).
pub const STEPS_HZ: [u32; 8] = [
    5_000, 10_000, 12_500, 15_000, 20_000, 25_000, 50_000, 100_000,
];

/// One editable set-mode setting: the value lives in `mask`'s (low) bits of byte `offset`; the
/// reserved high bits are preserved on write. `options[value]` is the human label. Facts
/// (offsets/bit widths/orderings) — spec-derived, see [`docs/radios/ft60.md`](../../../../docs/radios/ft60.md).
pub struct SettingSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub offset: usize,
    pub mask: u8,
    pub options: Vec<String>,
}

/// The modeled set-mode settings block (`0x24`+). Each value is the low bits of a dedicated byte
/// (reserved high bits preserved), so decode/apply is a clean masked read/write.
pub fn settings_specs() -> Vec<SettingSpec> {
    let opt = |labels: &[&str]| labels.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let apo = std::iter::once("Off".to_string())
        .chain((1..=24).map(|x| format!("{:.1} h", x as f64 * 0.5)))
        .collect();
    let tot = std::iter::once("Off".to_string())
        .chain((1..=30).map(|x| format!("{x} min")))
        .collect();
    vec![
        SettingSpec {
            key: "apo",
            label: "Auto power-off",
            offset: 0x24,
            mask: 0xFF,
            options: apo,
        },
        SettingSpec {
            key: "tot",
            label: "Time-out timer",
            offset: 0x25,
            mask: 0x1F,
            options: tot,
        },
        SettingSpec {
            key: "rf_sql",
            label: "RF squelch",
            offset: 0x28,
            mask: 0x0F,
            options: opt(&[
                "Off", "S-1", "S-2", "S-3", "S-4", "S-5", "S-6", "S-7", "S-8", "S-Full",
            ]),
        },
        SettingSpec {
            key: "lock",
            label: "Control lock",
            offset: 0x2B,
            mask: 0x07,
            options: opt(&[
                "Key", "Dial", "Key+Dial", "PTT", "PTT+Key", "PTT+Dial", "All",
            ]),
        },
        SettingSpec {
            key: "dt_dly",
            label: "DTMF delay",
            offset: 0x2C,
            mask: 0x07,
            options: opt(&["50 ms", "100 ms", "250 ms", "450 ms", "750 ms", "1000 ms"]),
        },
        SettingSpec {
            key: "dt_spd",
            label: "DTMF speed",
            offset: 0x2D,
            mask: 0x01,
            options: opt(&["50 ms", "100 ms"]),
        },
        SettingSpec {
            key: "ar_bep",
            label: "ARTS beep",
            offset: 0x2E,
            mask: 0x03,
            options: opt(&["Off", "In range", "Always"]),
        },
        SettingSpec {
            key: "lamp",
            label: "Lamp",
            offset: 0x2F,
            mask: 0x03,
            options: opt(&["Key", "5 sec", "Toggle"]),
        },
        SettingSpec {
            key: "bell",
            label: "Bell",
            offset: 0x30,
            mask: 0x07,
            options: opt(&["Off", "1", "3", "5", "8", "Continuous"]),
        },
        SettingSpec {
            key: "rxsave",
            label: "Battery saver",
            offset: 0x31,
            mask: 0x07,
            options: opt(&["Off", "200 ms", "300 ms", "500 ms", "1 s", "2 s"]),
        },
    ]
}

/// Decode a frequency field → Hz. The first byte's **high nibble is a flag**, not a BCD
/// digit: the low nibble is the leading digit, and bit `0x80` is the +5 kHz step correction
/// (12.5/25 kHz channels). So the value is 5 BCD digits × 10 kHz, plus 5 kHz when flagged.
fn decode_freq(b: &[u8]) -> u64 {
    let nib = |x: u8| (10 * (x >> 4) + (x & 0x0F)) as u64;
    let digits = (b[0] & 0x0F) as u64 * 10_000 + nib(b[1]) * 100 + nib(b[2]);
    digits * 10_000 + if b[0] & 0x80 != 0 { 5_000 } else { 0 }
}

/// Inverse of [`decode_freq`] into a 3-byte field: 5 BCD digits × 10 kHz, `0x80` = +5 kHz.
/// Preserves the unknown flag bits (`0x70`) of the first byte.
fn write_bcd_freq(dst: &mut [u8], hz: u64) {
    let digits = (hz / 10_000) as u32;
    let flag5 = hz % 10_000 == 5_000;
    let leading = (digits / 10_000) as u8;
    let rem = digits % 10_000;
    let d32 = (rem / 100) as u8;
    let d10 = (rem % 100) as u8;
    dst[0] = (dst[0] & 0x70) | if flag5 { 0x80 } else { 0 } | (leading & 0x0F);
    dst[1] = ((d32 / 10) << 4) | (d32 % 10);
    dst[2] = ((d10 / 10) << 4) | (d10 % 10);
}

/// The FT-60 name symbol block: the glyphs for codes `0x25..=0x3F`, indexed by `byte - 0x25`.
/// **Established from real hardware** — names entered symbol-by-symbol in dial order captured
/// back as this consecutive code run. `o` (`0x27`) and `u` (`0x3C`) are the radio's own
/// small-glyph characters; `/` appears twice (`0x33` and `0x36`) in the radio font.
#[rustfmt::skip]
const NAME_SYMBOLS: [char; 27] = [
    '!', '`', 'o', '$', '%', '&', '\'', '(', ')', '*', // 0x25..=0x2E
    '+', ',', '-', '.', '/', '|', ';', '/', '=', '>',  // 0x2F..=0x38
    '?', '@', '[', 'u', ']', '^', '_',                 // 0x39..=0x3F
];

/// FT-60 name charset (a hardware fact): `0x00–09` = "0"–"9", `0x0A–0x23` = "A"–"Z",
/// `0x24` = space (pad), `0x25–0x3F` = the [`NAME_SYMBOLS`] punctuation block. Codes above the
/// charset render as space (the radio's editor never emits them).
fn charset_byte(b: u8) -> char {
    match b {
        0x00..=0x09 => (b'0' + b) as char,
        0x0A..=0x23 => (b'A' + (b - 0x0A)) as char,
        0x24 => ' ',
        0x25..=0x3F => NAME_SYMBOLS[(b - 0x25) as usize],
        _ => ' ',
    }
}

/// Inverse of [`charset_byte`]: map a name char to its FT-60 code. Digits, A–Z, space and the
/// punctuation block map back exactly; lowercase letters fold to uppercase (the radio has no
/// lowercase letters — its `o`/`u` glyphs are symbols, so a typed "o"/"u" means the letter);
/// anything unsupported becomes `0x24` (space/pad).
fn encode_charset(ch: char) -> u8 {
    match ch {
        '0'..='9' => ch as u8 - b'0',
        'A'..='Z' => 0x0A + (ch as u8 - b'A'),
        'a'..='z' => 0x0A + (ch.to_ascii_uppercase() as u8 - b'A'),
        ' ' => 0x24,
        _ => NAME_SYMBOLS
            .iter()
            .position(|&c| c == ch)
            .map(|i| 0x25 + i as u8)
            .unwrap_or(0x24),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ft60_spec_constants() {
        let s = CloneSpec::FT60;
        assert_eq!(s.baud, 9600);
        assert_eq!(s.image_size, 28_617);
        assert_eq!(s.ack, 0x06);
        assert_eq!(s.magic, b"AH017");
        assert_eq!(s.wire_len(), 8 + 448 * 64);
    }

    #[test]
    fn ft60_is_a_registered_clone_image_profile() {
        use super::super::profile::{ProfileRegistry, RadioClass};
        let reg = ProfileRegistry::with_builtins();
        let ft60 = reg
            .clone_image_profiles()
            .find(|p| p.product_name() == "FT-60R")
            .expect("FT-60 registered as a clone-image profile");
        assert_eq!(ft60.class(), RadioClass::CloneImage);
        assert_eq!(ft60.clone_spec(), CloneSpec::FT60);
        // Detection by the model magic.
        assert!(reg.detect_clone_image(b"AH017\x00\x00\x00").is_some());
        assert!(reg.detect_clone_image(b"XX017\x00\x00\x00").is_none());
    }

    #[test]
    fn field_options_match_the_fact_constants() {
        let opts = Ft60.field_options();
        // Modes / powers are the plain lists, indexed by code.
        assert_eq!(opts.modes.len(), MODES.len());
        for (i, o) in opts.modes.iter().enumerate() {
            assert_eq!(o.code as usize, i);
            assert_eq!(o.label, MODES[i]);
        }
        assert_eq!(
            opts.powers
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            POWER_LEVELS
        );
        // Tone modes: "" surfaces as "Off"; the value-kind split is 1–3 CTCSS / 4–5 DCS /
        // 6–7 Cross (both).
        assert_eq!(opts.tone_modes.len(), TMODES.len());
        assert_eq!(opts.tone_modes[0].label, "Off");
        assert_eq!(opts.tone_modes[0].value_kind, ToneValueKind::None);
        assert_eq!(opts.tone_modes[1].value_kind, ToneValueKind::Ctcss); // Tone
        assert_eq!(opts.tone_modes[4].value_kind, ToneValueKind::Dcs); // DTCS
        assert_eq!(opts.tone_modes[6].value_kind, ToneValueKind::Cross); // Tone->DTCS
        assert_eq!(opts.tone_modes[7].value_kind, ToneValueKind::Cross); // DTCS->Tone
                                                                         // Steps: formatted from STEPS_HZ; code = index.
        assert_eq!(opts.steps[0].label, "5 kHz");
        assert_eq!(opts.steps[2].label, "12.5 kHz");
        assert_eq!(opts.steps.last().unwrap().label, "100 kHz");
        // Duplexes: the curated four, carrying their on-radio DUPLEX indices.
        let dup: Vec<_> = opts
            .duplexes
            .iter()
            .map(|o| (o.label.as_str(), o.code))
            .collect();
        assert_eq!(dup, vec![("Simplex", 0), ("−", 2), ("+", 3), ("Split", 4)]);
    }

    #[test]
    fn header_match() {
        let s = CloneSpec::FT60;
        assert!(s.header_matches(b"AH017\x00\x00\x00"));
        assert!(!s.header_matches(b"XX017"));
        assert!(!s.header_matches(b"AH")); // too short
    }

    /// A synthetic image (no real radio data): AH017 header + one programmed slot 0
    /// (146.5200 FM, CTCSS 100.0, name "TEST12", banks A+C, skip). Everything else zero.
    fn synthetic() -> Vec<u8> {
        let spec = CloneSpec::FT60;
        let mut b = vec![0u8; spec.image_size];
        b[..spec.magic.len()].copy_from_slice(spec.magic);
        // memory record @ slot 0
        b[MEM_BASE] = 0x80; // used, FM, simplex
        b[MEM_BASE + 1] = 0x01;
        b[MEM_BASE + 2] = 0x46;
        b[MEM_BASE + 3] = 0x52; // 1·46·52 → 14652 ×10 kHz = 146.520 MHz
        b[MEM_BASE + 4] = 0x52; // step index 5 (25 kHz), tmode = 2 (TSQL)
        b[MEM_BASE + 8] = 0x4C; // power index 1 (Mid), tone index 12 = 100.0 Hz
                                // name "TEST12" (8-byte slot: 6 chars + 0x80 0x80)
        b[NAME_BASE..NAME_BASE + 6].copy_from_slice(&[0x1D, 0x0E, 0x1C, 0x1D, 0x01, 0x02]);
        b[NAME_BASE + 6] = 0x80;
        b[NAME_BASE + 7] = 0x80;
        b[BANK_BASE] = 0x01; // bank A, slot 0
        b[BANK_BASE + 2 * BANK_STRIDE] = 0x01; // bank C, slot 0
        b[SKIP_BASE] = 0x01; // slot 0 skip = "S"
        let n = b.len();
        b[n - 1] = checksum(&b); // valid trailing checksum, like a real image
        b
    }

    #[test]
    fn decode_synthetic_channel() {
        let img = Ft60Image::decode(&synthetic()).unwrap();
        let chans = img.channels();
        assert_eq!(chans.len(), 1, "only slot 0 is used");
        let c = &chans[0];
        assert_eq!(c.slot, 0);
        assert_eq!(c.name, "TEST12");
        assert_eq!(c.rx_hz, 146_520_000);
        assert_eq!(c.mode, "FM");
        assert_eq!(c.tone_mode, "TSQL");
        assert_eq!(c.tone, Tone::Ctcss(1000));
        assert_eq!(c.power, 1, "Mid");
        assert_eq!(c.step, 5, "25 kHz");
        assert_eq!(c.tx_hz, 0);
        assert_eq!(c.banks, vec![0, 2]);
        assert_eq!(c.skip, "S");
    }

    #[test]
    fn synthetic_round_trips_byte_for_byte() {
        let bytes = synthetic();
        let img = Ft60Image::decode(&bytes).unwrap();
        assert_eq!(img.encode(), bytes, "decode → encode must be identity");
    }

    #[test]
    fn checksum_is_sum_of_preceding_bytes() {
        let bytes = synthetic();
        let n = bytes.len();
        assert_eq!(bytes[n - 1], checksum(&bytes));
        // Encoding restores a correct checksum even if the stored one is wrong.
        let mut corrupt = bytes.clone();
        corrupt[n - 1] = corrupt[n - 1].wrapping_add(1);
        let img = Ft60Image::decode(&corrupt).unwrap();
        assert_eq!(img.encode(), bytes, "encode recomputes the checksum");
    }

    /// Re-applying the decoded channels must not move a single byte (the write-side round-trip
    /// gate — proves every setter is a perfect inverse of its decoder).
    #[test]
    fn apply_decoded_channels_is_a_noop() {
        let bytes = synthetic();
        let mut img = Ft60Image::decode(&bytes).unwrap();
        let chans = img.channels();
        img.apply_channels(&chans);
        assert_eq!(img.encode(), bytes, "apply(channels()) must be identity");
    }

    #[test]
    fn edits_apply_and_decode_back() {
        let mut img = Ft60Image::decode(&synthetic()).unwrap();
        let mut chans = img.channels();
        // Edit slot 0: rename, retune, widen to NFM, change tone to DCS 023, drop bank C,
        // clear skip.
        chans[0].name = "HELLO".to_string();
        chans[0].rx_hz = 442_050_000; // 70 cm, +5 kHz step exercised elsewhere
        chans[0].mode = "NFM";
        chans[0].tone_mode = "DTCS"; // mode + value change together (as the UI does)
        chans[0].tone = Tone::Dcs(23);
        chans[0].banks = vec![0]; // A only
        chans[0].skip = "";
        img.apply_channels(&chans);

        let back = img.channels();
        assert_eq!(back.len(), 1);
        let c = &back[0];
        assert_eq!(c.name, "HELLO");
        assert_eq!(c.rx_hz, 442_050_000);
        assert_eq!(c.mode, "NFM");
        assert_eq!(c.tone, Tone::Dcs(23));
        assert_eq!(c.banks, vec![0]);
        assert_eq!(c.skip, "");
        // Round-trips through decode again (stable) and keeps a valid checksum.
        let bytes = img.encode();
        assert_eq!(bytes[bytes.len() - 1], checksum(&bytes));
        assert_eq!(Ft60Image::decode(&bytes).unwrap().channels(), back);
    }

    #[test]
    fn name_charset_covers_symbols() {
        // Every charset code decodes to a glyph; the common punctuation re-encodes to the same
        // byte (clean bijection), so an edited name with symbols round-trips. The three
        // radio-specific/duplicate glyphs (`0x27` 'o', `0x3C` 'u' fold to letters; `0x36` is the
        // second '/') are decode-only and skipped here.
        for b in 0x00u8..=0x3F {
            if b == 0x27 || b == 0x36 || b == 0x3C {
                continue;
            }
            assert_eq!(
                encode_charset(charset_byte(b)),
                b,
                "byte {b:#04x} must round-trip"
            );
        }
        // Spot-check the symbol codes confirmed against hardware (dial-order capture).
        assert_eq!(charset_byte(0x31), '-');
        assert_eq!(charset_byte(0x33), '/');
        assert_eq!(charset_byte(0x3F), '_');

        // A channel name with punctuation survives decode → edit → encode → decode…
        let mut img = Ft60Image::decode(&synthetic()).unwrap();
        let mut chans = img.channels();
        chans[0].name = "A-B/C.".to_string();
        img.apply_channels(&chans);
        assert_eq!(img.channels()[0].name, "A-B/C.");
        // …and re-applying the decoded channels is a byte-for-byte no-op (the round-trip gate).
        let before = img.encode();
        img.apply_channels(&img.channels());
        assert_eq!(img.encode(), before);
    }

    #[test]
    fn duplex_and_offset_round_trip() {
        let mut img = Ft60Image::decode(&synthetic()).unwrap();
        let mut chans = img.channels();
        // Slot 0 starts simplex, 0 offset.
        assert_eq!(chans[0].duplex, "");
        assert_eq!(chans[0].offset_hz, 0);
        // Make it a −600 kHz repeater.
        chans[0].duplex = "-";
        chans[0].offset_hz = 600_000;
        img.apply_channels(&chans);

        let back = img.channels();
        assert_eq!(back[0].duplex, "-");
        assert_eq!(back[0].offset_hz, 600_000, "600 kHz = 12 × 50 kHz steps");
        // Encodes to the byte pattern real hardware uses (0x000C at bytes 11–12).
        let rec = MEM_BASE; // slot 0
        assert_eq!((img.raw()[rec + 11], img.raw()[rec + 12]), (0x00, 0x0C));
        // Re-applying is a no-op (round-trip gate holds with the new fields).
        let before = img.encode();
        img.apply_channels(&img.channels());
        assert_eq!(img.encode(), before);
    }

    #[test]
    fn power_step_tone_mode_and_split_round_trip() {
        let mut img = Ft60Image::decode(&synthetic()).unwrap();
        let mut chans = img.channels();
        // Change power High, step 12.5 kHz, tone mode to plain Tone, and make it a split channel.
        chans[0].power = 0; // High
        chans[0].step = 2; // 12.5 kHz
        chans[0].tone_mode = "Tone";
        chans[0].duplex = "split";
        chans[0].tx_hz = 147_000_000;
        img.apply_channels(&chans);

        let back = &img.channels()[0];
        assert_eq!(back.power, 0);
        assert_eq!(back.step, 2);
        assert_eq!(back.tone_mode, "Tone");
        assert_eq!(
            back.tone,
            Tone::Ctcss(1000),
            "value preserved across a mode-only change"
        );
        assert_eq!(back.duplex, "split");
        assert_eq!(back.tx_hz, 147_000_000);
        // Re-applying is a no-op (round-trip gate holds with all the new fields).
        let before = img.encode();
        img.apply_channels(&img.channels());
        assert_eq!(img.encode(), before);
    }

    /// The exact byte patterns confirmed by writing an edited image to a real FT-60 and reading
    /// it back byte-identical: power `0/1/2` = High/Mid/Low (byte 8 bits 6–7), and a 70 cm `+`
    /// repeater's 5 MHz offset = 100 × 50 kHz at bytes 11–12.
    #[test]
    fn power_and_offset_encode_to_hardware_bytes() {
        let mut img = Ft60Image::decode(&synthetic()).unwrap();
        let mut chans = img.channels();

        chans[0].power = 1; // Mid
        img.apply_channels(&chans);
        assert_eq!(img.raw()[MEM_BASE + 8] & 0xC0, 0x40, "Mid = bits 01");
        chans[0].power = 2; // Low
        img.apply_channels(&chans);
        assert_eq!(img.raw()[MEM_BASE + 8] & 0xC0, 0x80, "Low = bits 10");

        // 70 cm repeater: +5 MHz offset encodes to 100 units at bytes 11–12, duplex nibble '+'=3.
        chans[0].rx_hz = 442_000_000;
        chans[0].duplex = "+";
        chans[0].offset_hz = 5_000_000;
        img.apply_channels(&chans);
        assert_eq!(img.raw()[MEM_BASE] & 0x0F, 3, "duplex '+' = nibble 3");
        let units =
            (((img.raw()[MEM_BASE + 11] as u16) << 8) | img.raw()[MEM_BASE + 12] as u16) & 0x7FFF;
        assert_eq!(units, 100, "5 MHz = 100 × 50 kHz");

        // Still a byte-exact no-op on re-apply (the round-trip gate).
        let before = img.encode();
        img.apply_channels(&img.channels());
        assert_eq!(img.encode(), before);
    }

    /// A cross tone mode (`Tone->DTCS`) must carry BOTH the CTCSS and DCS value through a
    /// decode → edit → encode → decode cycle — the regression this fixes was the DCS half
    /// clobbering the CTCSS half.
    #[test]
    fn tone_cross_mode_carries_both_values() {
        let mut img = Ft60Image::decode(&synthetic()).unwrap();
        let mut chans = img.channels();
        // slot 0 starts TSQL / CTCSS 100.0. Switch to a cross mode carrying both values.
        chans[0].tone_mode = "Tone->DTCS";
        chans[0].tone = Tone::Cross {
            ctcss: 1000,
            dcs: 23,
        };
        img.apply_channels(&chans);

        // Byte 8 low 6 bits = CTCSS index 12 (100.0 Hz); byte 9 low 7 bits = DCS index 0 (023).
        let rec = MEM_BASE;
        assert_eq!(img.raw()[rec + 8] & 0x3F, 12, "CTCSS index kept");
        assert_eq!(img.raw()[rec + 9] & 0x7F, 0, "DCS index written");

        let back = &img.channels()[0];
        assert_eq!(back.tone_mode, "Tone->DTCS");
        assert_eq!(
            back.tone,
            Tone::Cross {
                ctcss: 1000,
                dcs: 23
            },
            "both halves survive"
        );
        // Re-applying the decoded channels is a byte-for-byte no-op (round-trip gate).
        let before = img.encode();
        img.apply_channels(&img.channels());
        assert_eq!(img.encode(), before);
    }

    #[test]
    fn settings_round_trip_and_edit() {
        let mut img = Ft60Image::decode(&synthetic()).unwrap();
        let specs = settings_specs();
        // Re-applying the decoded settings is a byte-for-byte no-op (round-trip gate).
        let before = img.encode();
        let vals = img.settings();
        assert_eq!(vals.len(), specs.len());
        img.apply_settings(&vals);
        assert_eq!(img.encode(), before, "apply(settings()) must be identity");

        // Edit lamp (byte 0x2F, low 2 bits) → "Toggle" (2). Only that byte's low bits move.
        let lamp = specs.iter().position(|s| s.key == "lamp").unwrap();
        let mut edited = img.settings();
        edited[lamp] = 2;
        img.apply_settings(&edited);
        assert_eq!(img.raw()[0x2F] & 0x03, 2);
        assert_eq!(img.settings()[lamp], 2);
        // Reserved high bits of 0x2F untouched; re-apply is a no-op.
        let after = img.encode();
        img.apply_settings(&img.settings());
        assert_eq!(img.encode(), after);
    }

    #[test]
    fn pms_edges_round_trip() {
        let mut img = Ft60Image::decode(&synthetic()).unwrap();
        let edges = img.pms_edges();
        assert_eq!(edges.len(), PMS_COUNT);
        // Re-applying is a no-op (round-trip gate for the PMS region).
        let before = img.encode();
        img.apply_pms(&edges);
        assert_eq!(img.encode(), before);
        // Program pair 0 (lower/upper edges) and read it back.
        let mut e = img.pms_edges();
        e[0] = PmsEdge {
            index: 0,
            used: true,
            rx_hz: 144_000_000,
            step: 5,
        };
        e[1] = PmsEdge {
            index: 1,
            used: true,
            rx_hz: 148_000_000,
            step: 5,
        };
        img.apply_pms(&e);
        let back = img.pms_edges();
        assert!(back[0].used && back[1].used);
        assert_eq!(back[0].rx_hz, 144_000_000);
        assert_eq!(back[1].rx_hz, 148_000_000);
    }

    #[test]
    fn add_and_remove_channels() {
        let mut img = Ft60Image::decode(&synthetic()).unwrap();
        let mut chans = img.channels();
        // Add a new channel in slot 5.
        chans.push(Ft60Channel {
            slot: 5,
            name: "NEW1".to_string(),
            rx_hz: 146_940_000,
            duplex: "",
            offset_hz: 0,
            tx_hz: 0,
            mode: "FM",
            tone_mode: "TSQL",
            tone: Tone::Ctcss(1000),
            power: 0,
            step: 0,
            banks: vec![1],
            skip: "",
        });
        img.apply_channels(&chans);
        assert_eq!(img.channels().len(), 2);

        // Now remove slot 0 (drop it from the desired set).
        let remaining: Vec<Ft60Channel> =
            img.channels().into_iter().filter(|c| c.slot != 0).collect();
        img.apply_channels(&remaining);
        let final_chans = img.channels();
        assert_eq!(final_chans.len(), 1);
        assert_eq!(final_chans[0].slot, 5);
        assert_eq!(final_chans[0].name, "NEW1");
    }

    #[test]
    fn decode_rejects_bad_input() {
        assert!(Ft60Image::decode(b"too short").is_err());
        let mut b = vec![0u8; CloneSpec::FT60.image_size];
        b[0] = b'X'; // wrong magic
        assert!(Ft60Image::decode(&b).is_err());
    }
}
