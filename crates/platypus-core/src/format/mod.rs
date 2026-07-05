// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Lossless, model-agnostic sentence layer for Uniden scanner files.
//!
//! Every `.hpd` / `.cfg` / `.avd` is tab-delimited ASCII, one sentence per line,
//! with CRLF line endings (with at least one real-world bare-LF quirk observed in
//! `profile.cfg`). This layer parses bytes into [`Document`] (a list of tab-split
//! [`Line`]s, each carrying its exact terminator) and serializes back
//! **byte-for-byte** — the foundation of the writer's round-trip safety net.

mod sentence;

pub use sentence::{Document, FileHeader, Line, LineEnding};
