// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Scanner-model layer — the single place model differences live.
//!
//! Every radio is a [`RadioProfile`]; the class-specific contract is a sub-trait —
//! [`SdCardProfile`] (SD-card scanners like the SDS150) or [`CloneImageProfile`]
//! (serial clone-image radios like the FT-60R). [`ProfileRegistry`] holds them all and
//! detects the right one (an SD-card file header, or a clone-image magic).
//!
//! ## Adding a new radio
//!
//! 1. Add a module (like [`sds150`] or [`ft60`]) implementing [`RadioProfile`] + its
//!    class sub-trait.
//! 2. `register()` it (or add it to [`ProfileRegistry::with_builtins`]).
//!
//! Nothing in [`crate::format`] changes — the byte-exact core and its round-trip
//! gate are model-agnostic by construction.

pub mod ft60;
mod profile;
pub mod sds150;

pub use ft60::{CloneSpec, Ft60};
pub use profile::{
    ChannelColumns, CloneCapacity, CloneFieldOptions, CloneImageProfile, FavoritesDialect,
    FieldOption, GeoColumns, ModelKey, ProfileRegistry, RadioClass, RadioProfile, RecordSchema,
    SdCardProfile, SdLayout, ToneValueKind,
};
pub use sds150::Sds150;
