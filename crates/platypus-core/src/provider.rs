// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Import providers + the canonical, cross-source data model.
//!
//! The product goal is *many ways to add data × many ways to filter it*. This
//! module is the "add" seam and the hub everything funnels into:
//!
//! ```text
//! Provider (Sentinel files, RR Web Service, FCC ULS, CSV, …)
//!     → Dataset (canonical, source-agnostic)
//!         → filter (Dataset::select) → writer (later)
//! ```
//!
//! A [`Provider`] yields a [`Dataset`] — owned, source-agnostic records — so a
//! filter or writer never cares whether the data came from an HPDB card, a
//! RadioReference SOAP call, or a CSV. [`HpdbProvider`] is provider #1 (reads the
//! HPDB card files we already parse). The model is intentionally minimal now and
//! grows attribute-by-attribute (service type, mode, agency, frequencies) as the
//! richer filters and additional providers land — it does not hard-code any one
//! source's shape.

use std::collections::BTreeSet;

use crate::device::ProfileRegistry;
use crate::extract::Extraction;
use crate::format::Document;
use crate::model::{Geo, Record};
use crate::{Error, Result};

/// Coarse system classification, source-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemKind {
    Conventional,
    Trunk,
    Other,
}

impl SystemKind {
    fn from_command(command: &str) -> Self {
        match command {
            "Conventional" => SystemKind::Conventional,
            "Trunk" => SystemKind::Trunk,
            _ => SystemKind::Other,
        }
    }
}

/// A named geographic point inside a system (a site or coverage group).
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub name: String,
    pub geo: Geo,
}

/// Whether a channel is a conventional frequency or a trunked talkgroup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Frequency,
    Talkgroup,
}

/// One channel (a frequency or a talkgroup) with its filterable attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub name: String,
    pub kind: ChannelKind,
    pub freq_hz: Option<u64>,
    pub tgid: Option<String>,
    pub mode: Option<String>,
    pub tone: Option<String>,
    /// RadioReference service-type code (use `model::service_type_name`).
    pub service_type: Option<u16>,
}

/// One radio system in the canonical model. Owned (unlike the borrowed
/// `extract::System` view) so it can come from any provider and be stored.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemRecord {
    pub name: String,
    pub kind: SystemKind,
    /// Raw technology string (`P25Standard`, `MotoTrbo`, `Conventional`, …).
    pub tech: Option<String>,
    pub county_ids: Vec<u64>,
    pub state_ids: Vec<u64>,
    pub locations: Vec<Location>,
    pub channels: Vec<Channel>,
    /// Set of service-type codes present across this system's channels — for
    /// quick system-level filtering (e.g. "has any fire-dispatch channel").
    pub service_types: BTreeSet<u16>,
}

impl SystemRecord {
    pub fn is_in_county(&self, county_id: u64) -> bool {
        self.county_ids.contains(&county_id)
    }

    pub fn is_in_state(&self, state_id: u64) -> bool {
        self.state_ids.contains(&state_id)
    }

    /// True if any channel carries the given service-type code.
    pub fn has_service_type(&self, code: u16) -> bool {
        self.service_types.contains(&code)
    }

    /// True if the system's technology matches (case-sensitive raw match).
    pub fn tech_is(&self, tech: &str) -> bool {
        self.tech.as_deref() == Some(tech)
    }
}

/// A source-agnostic collection of systems — the hub between providers and
/// filters/writers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dataset {
    pub systems: Vec<SystemRecord>,
}

impl Dataset {
    pub fn len(&self) -> usize {
        self.systems.len()
    }

    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }

    /// Generic selection: any predicate over a system. Filters (county, radius,
    /// state, service type, …) are UI-composed from this — the model hard-codes
    /// none of them.
    pub fn select(&self, pred: impl Fn(&SystemRecord) -> bool) -> Vec<&SystemRecord> {
        self.systems.iter().filter(|s| pred(s)).collect()
    }
}

/// A source of scanner data. Implementors load their source into the canonical
/// [`Dataset`]; downstream code is provider-agnostic.
pub trait Provider {
    /// Human-readable source label (e.g. "HPDB card", "RadioReference").
    fn name(&self) -> &str;
    /// Load this source into the canonical model.
    fn load(&self) -> Result<Dataset>;
}

/// Provider #1: scanner HPDB card files (what Sentinel writes). Holds a parsed
/// [`Document`]; the caller does the I/O so the core stays I/O-light.
pub struct HpdbProvider {
    doc: Document,
    source: String,
}

impl HpdbProvider {
    pub fn from_document(doc: Document, source: impl Into<String>) -> Self {
        HpdbProvider {
            doc,
            source: source.into(),
        }
    }
}

impl Provider for HpdbProvider {
    fn name(&self) -> &str {
        &self.source
    }

    fn load(&self) -> Result<Dataset> {
        // Detect the model from the file header (works for any registered model).
        let registry = ProfileRegistry::with_builtins();
        let profile = registry
            .detect(&self.doc.header())
            .ok_or(Error::UnknownModel)?;

        let systems = Extraction::segment(&self.doc, profile)
            .systems()
            .map(|sys| {
                let tech = Record::new(sys.header(), profile)
                    .and_then(|r| r.tech())
                    .map(str::to_string);

                let mut locations = Vec::new();
                let mut channels = Vec::new();
                let mut service_types = BTreeSet::new();

                for line in sys.lines() {
                    let Some(rec) = Record::new(line, profile) else {
                        continue;
                    };
                    if let Some(geo) = rec.geo() {
                        locations.push(Location {
                            name: rec.name().unwrap_or("").to_string(),
                            geo,
                        });
                        continue;
                    }
                    let freq = rec.frequency_hz();
                    let tgid = rec.talkgroup();
                    if freq.is_some() || tgid.is_some() {
                        let service_type = rec.service_type_code();
                        if let Some(code) = service_type {
                            service_types.insert(code);
                        }
                        channels.push(Channel {
                            name: rec.name().unwrap_or("").to_string(),
                            kind: if tgid.is_some() {
                                ChannelKind::Talkgroup
                            } else {
                                ChannelKind::Frequency
                            },
                            freq_hz: freq,
                            tgid: tgid.map(str::to_string),
                            mode: rec.mode().map(str::to_string),
                            tone: rec.tone().map(str::to_string),
                            service_type,
                        });
                    }
                }

                SystemRecord {
                    name: sys.name().unwrap_or("").to_string(),
                    kind: SystemKind::from_command(sys.header().command()),
                    tech,
                    county_ids: sys.county_ids().into_iter().collect(),
                    state_ids: sys.state_ids().into_iter().collect(),
                    locations,
                    channels,
                    service_types,
                }
            })
            .collect();

        Ok(Dataset { systems })
    }
}
