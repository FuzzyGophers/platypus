// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! # platypus-core
//!
//! The open, cross-platform backend for Platypus — a location-first manager for
//! Uniden scanners (first target: SDS150). No UI, no platform assumptions.
//!
//! ## Layering (and where new scanner models plug in)
//!
//! The crate is deliberately split so that **adding a new scanner model never
//! touches the byte-exact format core**:
//!
//! - [`format`] — the **lossless, model-agnostic** sentence layer. A scanner file
//!   is tab-delimited ASCII, one sentence per line. [`format::Document`] parses
//!   raw bytes into tab-split lines with their exact terminators, and serializes
//!   back **byte-for-byte**. This is the round-trip safety net the project gates
//!   the writer on. It has zero knowledge of what `Site` or `TGID` *mean*.
//!
//! - [`device`] — the **model-specific** layer. A [`device::SdCardProfile`]
//!   describes one scanner family: its SD-card layout (folder/file names, incl.
//!   the real-but-misspelled `discvery.cfg` and the must-delete `app_data.cfg`),
//!   its record schemas, the favorites "blank-the-ID-columns" dialect rule, and
//!   its serial model id. A [`device::ProfileRegistry`] detects the right profile
//!   from a file's header.
//!
//! Adding, say, an SDS200 or BCD model later = implement one `SdCardProfile` and
//! `register()` it. The `format` core and its round-trip test are untouched.
//!
//! ```
//! use platypus_core::format::Document;
//! use platypus_core::device::ProfileRegistry;
//!
//! let raw = b"TargetModel\tBCDx36HP\r\nFormatVersion\t1.00\r\n";
//! let doc = Document::parse(raw).unwrap();
//! assert_eq!(doc.to_bytes(), raw);                 // byte-exact round trip
//!
//! let reg = ProfileRegistry::with_builtins();
//! let profile = reg.detect(&doc.header()).unwrap();
//! assert_eq!(profile.product_name(), "SDS150");
//! ```

pub mod card;
pub mod county_geo;
pub mod device;
pub mod display;
pub mod error;
pub mod extract;
pub mod favorites;
pub mod format;
pub mod model;
pub mod provider;
pub mod rr;
pub mod serial;
pub mod synthesize;

pub use error::{Error, Result};
