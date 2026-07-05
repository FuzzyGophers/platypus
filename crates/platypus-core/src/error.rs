// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Crate error type. Hand-rolled to keep the core dependency-free.

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A byte outside printable ASCII (and not TAB/CR) appeared in a payload.
    /// The file spec mandates ASCII; this flags a card we can't safely model.
    NonAscii { offset: usize, byte: u8 },
    /// No registered scanner profile matched a file's header (unknown
    /// `TargetModel`/`FormatVersion`).
    UnknownModel,
    /// A capability that lives in the platform/glue layer, not the I/O-light
    /// core, was invoked here (e.g. the RadioReference SOAP fetch — the core only
    /// maps already-fetched RR types into the canonical model). Carries a hint.
    NotInCore(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NonAscii { offset, byte } => write!(
                f,
                "non-ASCII byte 0x{byte:02X} at offset {offset} (file spec requires ASCII)"
            ),
            Error::UnknownModel => {
                write!(f, "no scanner profile matched the file header")
            }
            Error::NotInCore(hint) => {
                write!(f, "operation not available in platypus-core: {hint}")
            }
        }
    }
}

impl std::error::Error for Error {}
