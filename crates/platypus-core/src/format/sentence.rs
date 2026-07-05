// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! The byte-exact sentence model. Mirrors the validated `roundtrip_hpd.py` logic:
//! split on `\n` keeping each terminator, ASCII-decode the payload, split on TAB.
//! Serialization re-joins on TAB and re-emits the exact terminator, so
//! `parse(raw).to_bytes() == raw` for every well-formed card file.

use crate::error::{Error, Result};

/// Exactly how a line was terminated on disk. Preserved so we can rebuild the
/// file byte-for-byte (including the lone bare-LF seen in real `profile.cfg`,
/// and a possible missing newline on the final line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// `\r\n` — the normal terminator.
    Crlf,
    /// `\n` — a bare line feed (rare, but it occurs on real cards).
    Lf,
    /// No terminator — only valid on the final line of a file.
    None,
}

impl LineEnding {
    pub fn as_bytes(self) -> &'static [u8] {
        match self {
            LineEnding::Crlf => b"\r\n",
            LineEnding::Lf => b"\n",
            LineEnding::None => b"",
        }
    }
}

/// One sentence: its tab-separated fields plus the terminator that followed it.
/// `fields[0]` is the command (record type). Empty fields are preserved exactly,
/// which is what makes the favorites "blanked MyId/ParentId" dialect round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub fields: Vec<String>,
    pub ending: LineEnding,
}

impl Line {
    /// The command / record-type token (`fields[0]`), e.g. `"Site"`.
    pub fn command(&self) -> &str {
        self.fields.first().map(String::as_str).unwrap_or("")
    }

    /// Field by index, or `None` if out of range.
    pub fn field(&self, idx: usize) -> Option<&str> {
        self.fields.get(idx).map(String::as_str)
    }
}

/// A whole parsed file: an ordered list of lines. Order and content are preserved
/// exactly; this type makes no semantic claims about the records (that's
/// `device::SdCardProfile`'s job).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub lines: Vec<Line>,
}

/// The header sentences every card file starts with. Used to pick a scanner
/// profile. `target_model` is e.g. `"BCDx36HP"`; `format_version` e.g. `"1.00"`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileHeader {
    pub target_model: Option<String>,
    pub format_version: Option<String>,
}

impl Document {
    /// Parse raw file bytes into a [`Document`]. Errors only on bytes that break
    /// the ASCII contract (so we never silently mangle an unexpected card).
    pub fn parse(raw: &[u8]) -> Result<Document> {
        let mut lines = Vec::new();
        let n = raw.len();
        let mut start = 0usize;

        while start < n {
            // Find the next `\n`; everything up to it (minus a preceding `\r`)
            // is the payload.
            let (payload_end, next, ending) = match raw[start..].iter().position(|&b| b == b'\n') {
                Some(rel) => {
                    let nl = start + rel;
                    if nl > start && raw[nl - 1] == b'\r' {
                        (nl - 1, nl + 1, LineEnding::Crlf)
                    } else {
                        (nl, nl + 1, LineEnding::Lf)
                    }
                }
                None => (n, n, LineEnding::None),
            };

            let payload = &raw[start..payload_end];
            validate_ascii(payload, start)?;
            // Safe: validate_ascii guarantees every byte is < 0x80.
            let text = std::str::from_utf8(payload).expect("validated ASCII");
            let fields = text.split('\t').map(str::to_owned).collect();

            lines.push(Line { fields, ending });
            start = next;
        }

        Ok(Document { lines })
    }

    /// Serialize back to bytes. Guaranteed `parse(raw).to_bytes() == raw`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for line in &self.lines {
            out.extend_from_slice(line.fields.join("\t").as_bytes());
            out.extend_from_slice(line.ending.as_bytes());
        }
        out
    }

    /// Extract the `TargetModel` / `FormatVersion` header sentences.
    pub fn header(&self) -> FileHeader {
        FileHeader {
            target_model: self.field_value("TargetModel"),
            format_version: self.field_value("FormatVersion"),
        }
    }

    /// Value (`fields[1]`) of the first line whose command matches `command`.
    pub fn field_value(&self, command: &str) -> Option<String> {
        self.lines
            .iter()
            .find(|l| l.command() == command)
            .and_then(|l| l.field(1))
            .map(str::to_owned)
    }

    /// Count of lines per command — handy for recon and schema validation.
    pub fn command_counts(&self) -> std::collections::BTreeMap<String, usize> {
        let mut counts = std::collections::BTreeMap::new();
        for line in &self.lines {
            *counts.entry(line.command().to_owned()).or_insert(0) += 1;
        }
        counts
    }
}

/// Reject anything that isn't printable ASCII, TAB, or CR (the bytes a payload
/// may legitimately contain; `\n` never reaches here — it's the delimiter).
fn validate_ascii(payload: &[u8], base_offset: usize) -> Result<()> {
    for (i, &b) in payload.iter().enumerate() {
        let ok = b == b'\t' || b == b'\r' || (0x20..=0x7E).contains(&b);
        if !ok {
            return Err(Error::NonAscii {
                offset: base_offset + i,
                byte: b,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrips(raw: &[u8]) {
        let doc = Document::parse(raw).unwrap();
        assert_eq!(doc.to_bytes(), raw, "round trip mismatch");
    }

    #[test]
    fn crlf_lines_roundtrip() {
        roundtrips(b"TargetModel\tBCDx36HP\r\nFormatVersion\t1.00\r\n");
    }

    #[test]
    fn bare_lf_is_preserved() {
        // The profile.cfg quirk: one bare-LF among CRLFs must survive untouched.
        roundtrips(b"A\tb\r\nC\td\nE\tf\r\n");
    }

    #[test]
    fn missing_final_newline_is_preserved() {
        roundtrips(b"A\tb\r\nC\td");
    }

    #[test]
    fn empty_fields_and_blank_lines_roundtrip() {
        // Blanked MyId/ParentId columns (the favorites dialect) + a blank line.
        roundtrips(b"Site\t\t\tPrimary\r\n\r\n");
    }

    #[test]
    fn header_and_counts() {
        let doc =
            Document::parse(b"TargetModel\tBCDx36HP\r\nFormatVersion\t1.00\r\nSite\t\t\tA\r\n")
                .unwrap();
        let h = doc.header();
        assert_eq!(h.target_model.as_deref(), Some("BCDx36HP"));
        assert_eq!(h.format_version.as_deref(), Some("1.00"));
        assert_eq!(doc.command_counts().get("Site"), Some(&1));
    }

    #[test]
    fn non_ascii_is_rejected() {
        let err = Document::parse(b"Site\t\xFF\r\n").unwrap_err();
        assert!(matches!(err, Error::NonAscii { byte: 0xFF, .. }));
    }

    #[test]
    fn embedded_nul_is_rejected() {
        // A NUL is below the printable range and isn't TAB/CR — reject it symmetric to 0xFF.
        let err = Document::parse(b"Site\t\x00\r\n").unwrap_err();
        assert!(matches!(err, Error::NonAscii { byte: 0x00, .. }));
    }

    #[test]
    fn control_byte_is_rejected() {
        // A bell (0x07) is a C0 control byte, outside 0x20..=0x7E — reject it too.
        let err = Document::parse(b"Site\t\x07\r\n").unwrap_err();
        assert!(matches!(err, Error::NonAscii { byte: 0x07, .. }));
    }

    #[test]
    fn empty_input_is_a_valid_empty_document() {
        // No bytes → no lines, and it still round-trips (to_bytes is empty).
        let doc = Document::parse(b"").unwrap();
        assert!(doc.lines.is_empty());
        assert_eq!(doc.to_bytes(), b"");
    }

    #[test]
    fn del_boundary_follows_the_printable_rule() {
        // The accept rule is 0x20..=0x7E: the last printable (0x7E '~') is allowed…
        roundtrips(b"Site\t~\r\n");
        // …but DEL (0x7F), one past the top of the range, is rejected.
        let err = Document::parse(b"Site\t\x7F\r\n").unwrap_err();
        assert!(matches!(err, Error::NonAscii { byte: 0x7F, .. }));
    }
}
