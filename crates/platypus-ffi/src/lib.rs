// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! C ABI bridge over `platypus-core`, for the SwiftUI macOS app.
//!
//! Design: keep the surface tiny and string-based. Handles are opaque pointers;
//! query results cross the boundary as **JSON strings** (hand-built, no serde) so
//! Swift can `Codable`-decode them without a generated struct ABI. Every returned
//! `char*` is heap-owned by Rust and must be released with
//! [`platypus_string_free`]; every handle with [`platypus_close_hpdb`].
//!
//! All functions are null-tolerant: a bad path / parse failure / null handle yields a null
//! (or `0`) return rather than a panic — the core is written to never panic on malformed
//! input. As defense-in-depth, every entry point wraps its body in [`ffi_guard`]
//! (`catch_unwind`), so an *unexpected* panic degrades to that function's failure return (null,
//! a non-null error string for the "null = success" card writers, or `0`) instead of unwinding
//! across the C ABI. The workspace sets `panic = "unwind"` for release (`Cargo.toml`) so this
//! recovery actually runs in shipped builds rather than aborting the process.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{c_char, c_double, c_void, CStr, CString};
use std::path::Path;
use std::ptr;

use std::sync::OnceLock;

use platypus_core::county_geo::{self, CentroidBuilder};
use platypus_core::device::{CloneSpec, ProfileRegistry, RadioClass, SdCardProfile, ToneValueKind};
use platypus_core::extract::{self, is_voice_channel, Extraction, System};
use platypus_core::format::{Document, Line, LineEnding};
use platypus_core::model::{haversine_miles, keyed_id, CountyIndex, Record};
use platypus_core::{card, favorites};

/// Opaque handle: a parsed HPDB document plus the profile to interpret it.
pub struct PlatypusHpdb {
    doc: Document,
    profile: &'static dyn SdCardProfile,
}

/// Opaque handle: a built favorites document, ready to preview and commit.
pub struct PlatypusFavorites {
    doc: Document,
    profile: &'static dyn SdCardProfile,
}

/// Process-wide scanner-profile registry. Model selection is **detected from each
/// file's `TargetModel`/`FormatVersion` header** — never hard-coded — so the FFI
/// supports every model registered in `platypus-core` automatically.
fn registry() -> &'static ProfileRegistry {
    static REG: OnceLock<ProfileRegistry> = OnceLock::new();
    REG.get_or_init(ProfileRegistry::with_builtins)
}

/// Detect the scanner profile for a parsed document from its header. None if no
/// registered model matches (unknown/unsupported card).
fn detect_profile(doc: &Document) -> Option<&'static dyn SdCardProfile> {
    registry().detect(&doc.header())
}

/// The FT-60 clone-transport spec, from the registered clone-image profile (falls back
/// to the built-in constant if — impossibly — none is registered).
fn ft60_spec() -> CloneSpec {
    registry()
        .clone_image()
        .map(|p| p.clone_spec())
        .unwrap_or(CloneSpec::FT60)
}

/// Every radio Platypus supports, from the profile registry — the single source of truth
/// the app's radio list derives from. JSON array of
/// `[{ id, name, maker, transport, class:"sdCard"|"cloneImage" }]`.
#[no_mangle]
pub extern "C" fn platypus_radios_json() -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || {
        let mut out = String::from("[");
        for (i, p) in registry().profiles().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"id\":");
            push_json_string(&mut out, p.id());
            out.push_str(",\"name\":");
            push_json_string(&mut out, p.product_name());
            out.push_str(",\"maker\":");
            push_json_string(&mut out, p.maker());
            out.push_str(",\"transport\":");
            push_json_string(&mut out, p.transport());
            out.push_str(",\"class\":");
            push_json_string(
                &mut out,
                match p.class() {
                    RadioClass::SdCardScanner => "sdCard",
                    RadioClass::CloneImage => "cloneImage",
                },
            );
            // Clone-image radios carry a fixed memory capacity (the UI surfaces it instead of
            // re-declaring the numbers). SD-card radios omit it (their limits come from the card).
            if let Some(ci) = p.as_clone_image() {
                let cap = ci.capacity();
                out.push_str(",\"channels\":");
                out.push_str(&cap.channels.to_string());
                out.push_str(",\"banks\":");
                out.push_str(&cap.banks.to_string());
                out.push_str(",\"nameLen\":");
                out.push_str(&cap.name_len.to_string());
            }
            out.push('}');
        }
        out.push(']');
        to_c_string(out)
    })
}

/// The FT-60 channel-form option sets, from the clone-image profile's `field_options` — the
/// single source of truth for the editor's pickers. JSON object
/// `{ modes, toneModes, steps, powers, duplexes }`, each an array of
/// `{ label, code[, valueKind:"none"|"ctcss"|"dcs"] }` (`valueKind` on tone modes only). The
/// UI shows `label`, stores/sends `code` — no attribute table lives in the app. Null if no
/// clone-image radio is registered.
#[no_mangle]
pub extern "C" fn platypus_ft60_options_json() -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || {
        let Some(opts) = registry().clone_image().map(|p| p.field_options()) else {
            return ptr::null_mut();
        };
        // Emit one `{label,code}` list; `tone` adds the value-kind each option needs.
        let emit = |out: &mut String,
                    key: &str,
                    list: &[platypus_core::device::FieldOption],
                    tone: bool| {
            out.push('"');
            out.push_str(key);
            out.push_str("\":[");
            for (i, o) in list.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str("{\"label\":");
                push_json_string(out, &o.label);
                out.push_str(",\"code\":");
                out.push_str(&o.code.to_string());
                if tone {
                    out.push_str(",\"valueKind\":");
                    push_json_string(
                        out,
                        match o.value_kind {
                            ToneValueKind::None => "none",
                            ToneValueKind::Ctcss => "ctcss",
                            ToneValueKind::Dcs => "dcs",
                            ToneValueKind::Cross => "cross",
                        },
                    );
                }
                out.push('}');
            }
            out.push(']');
        };
        let mut out = String::from("{");
        emit(&mut out, "modes", &opts.modes, false);
        out.push(',');
        emit(&mut out, "toneModes", &opts.tone_modes, true);
        out.push(',');
        emit(&mut out, "steps", &opts.steps, false);
        out.push(',');
        emit(&mut out, "powers", &opts.powers, false);
        out.push(',');
        emit(&mut out, "duplexes", &opts.duplexes, false);
        out.push('}');
        to_c_string(out)
    })
}

/// The RadioReference service-type codes → names, from `core::model::SERVICE_TYPES` — the
/// single source the app's `ServiceType` presentation table merges names from (it keeps only
/// the SF Symbol + color). JSON array of `[{ code, name }]`.
#[no_mangle]
pub extern "C" fn platypus_service_types_json() -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || {
        let mut out = String::from("[");
        for (i, (code, name)) in platypus_core::model::SERVICE_TYPES.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"code\":");
            out.push_str(&code.to_string());
            out.push_str(",\"name\":");
            push_json_string(&mut out, name);
            out.push('}');
        }
        out.push(']');
        to_c_string(out)
    })
}

// ---- lifecycle ----

/// Parse an HPDB `.hpd`/`.cfg` file at `path`. Returns null on any failure.
///
/// # Safety
/// `path` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_open_hpdb(path: *const c_char) -> *mut PlatypusHpdb {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(path) = cstr_to_str(path) else {
            return ptr::null_mut();
        };
        let Ok(bytes) = std::fs::read(path) else {
            return ptr::null_mut();
        };
        let Ok(doc) = Document::parse(&bytes) else {
            return ptr::null_mut();
        };
        let Some(profile) = detect_profile(&doc) else {
            return ptr::null_mut();
        };
        Box::into_raw(Box::new(PlatypusHpdb { doc, profile }))
    })
}

/// Free a handle from [`platypus_open_hpdb`]. Safe to call with null.
///
/// # Safety
/// `handle` must be a pointer previously returned by `platypus_open_hpdb`, or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_close_hpdb(handle: *mut PlatypusHpdb) {
    ffi_guard((), move || unsafe {
        if !handle.is_null() {
            drop(Box::from_raw(handle));
        }
    })
}

/// Free a string returned by any `*_json` function. Safe to call with null.
///
/// # Safety
/// `s` must be a pointer previously returned by this library, or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_string_free(s: *mut c_char) {
    ffi_guard((), move || unsafe {
        if !s.is_null() {
            drop(CString::from_raw(s));
        }
    })
}

// ---- queries (return owned JSON strings) ----

/// JSON array of every system in the document. Null on null handle.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_systems_json(handle: *const PlatypusHpdb) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(h) = handle.as_ref() else {
            return ptr::null_mut();
        };
        to_c_string(systems_json(&h.doc, h.profile))
    })
}

/// JSON array of systems tagged with `county_id`.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_systems_in_county_json(
    handle: *const PlatypusHpdb,
    county_id: u64,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(h) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let picked = extract::by_county(&h.doc, h.profile, county_id);
        to_c_string(systems_json(&picked, h.profile))
    })
}

/// JSON array of systems with a location within `miles` of (`lat`, `lon`).
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_systems_in_radius_json(
    handle: *const PlatypusHpdb,
    lat: c_double,
    lon: c_double,
    miles: c_double,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(h) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let picked = extract::within_radius(&h.doc, h.profile, lat, lon, miles);
        to_c_string(systems_json(&picked, h.profile))
    })
}

/// JSON catalog for the browse/filter/select UI: every system with its
/// channel-level detail (the 1B catalog needs service type, tech, encryption,
/// TGID/frequency — richer than `platypus_systems_json`). Null on null handle.
///
/// Shape: `[{ "id","name","kind","tech","counties":[u64],"siteCount":u64,
///   "channels":[{ "id","name","kind":"Talkgroup"|"Frequency","tgid":str|null,
///   "freqHz":u64|null,"mode":str|null,"serviceType":u16|null,"tone":str|null }] }]`
///   // `tone` = raw audio option (TONE=/NAC=/ColorCode=…)
///
/// IDs are stable within a load (`s<i>` per system, `s<i>c<j>` per channel) so the
/// UI can track a channel-level selection cart.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_catalog_json(handle: *const PlatypusHpdb) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(h) = handle.as_ref() else {
            return ptr::null_mut();
        };
        to_c_string(catalog_json(&h.doc, h.profile))
    })
}

// ---- full-USA library (aggregate of every state file) ----

/// Opaque handle: the whole loaded HPDB library — every `s_*.hpd` in a directory,
/// parsed and held in memory. Filtering runs over this (microseconds, benchmarked),
/// so no database is needed for browse speed.
pub struct PlatypusLibrary {
    docs: Vec<Document>,
    profile: &'static dyn SdCardProfile,
    /// Precomputed location-first county placement, per system key `(di, si)`. Built
    /// once at open (the geo-matching is done here, not per query).
    placement: HashMap<(usize, usize), SystemPlacement>,
}

/// Where a system's channels live, geographically. `by_county` maps a county id to
/// the voice-channel indices placed there; the key **0** is the no-county bucket
/// (channels with no county/geo — the state-level fallback). `states` are the
/// system's `AreaState` ids (the next rung of the fallback ladder).
struct SystemPlacement {
    states: Vec<u64>,
    by_county: BTreeMap<u64, Vec<usize>>,
}

/// A C progress callback: `(ctx, phase, done, total)`. Phase 1 = reading files,
/// phase 2 = indexing coverage.
pub type PlatypusProgressFn = Option<extern "C" fn(*mut c_void, u32, u32, u32)>;

/// Precompute county placement for every system: walk each system's groups, place
/// each group (and thus its channels) into the counties its coverage reaches
/// (`county_geo::group_counties`), bucketing no-county channels under key 0.
/// `on_doc(done, total)` is called after each document for progress reporting.
fn build_placement(
    docs: &[Document],
    profile: &dyn SdCardProfile,
    centroids: &county_geo::CountyCentroids,
    mut on_doc: impl FnMut(usize, usize),
) -> HashMap<(usize, usize), SystemPlacement> {
    let total = docs.len();
    let mut out = HashMap::new();
    for (di, doc) in docs.iter().enumerate() {
        for (si, sys) in Extraction::segment(doc, profile).systems().enumerate() {
            let states = sys.state_ids();
            let mut by_county: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
            let mut current: BTreeSet<u64> = BTreeSet::new();
            let mut ci = 0usize;
            for line in sys.lines() {
                let cmd = line.command();
                if matches!(cmd, "C-Group" | "T-Group") {
                    current = Record::new(line, profile)
                        .map(|rec| county_geo::group_counties(&rec, &states, centroids))
                        .unwrap_or_default();
                } else if is_voice_channel(cmd) {
                    let this = ci;
                    ci += 1;
                    if current.is_empty() {
                        by_county.entry(0).or_default().push(this);
                    } else {
                        for c in &current {
                            by_county.entry(*c).or_default().push(this);
                        }
                    }
                }
            }
            out.insert(
                (di, si),
                SystemPlacement {
                    states: states.into_iter().collect(),
                    by_county,
                },
            );
        }
        on_doc(di + 1, total);
    }
    out
}

/// Load every `s_*.hpd` under `dir` into one in-memory library (the full-USA load).
/// Null on a bad path or if the directory can't be read. Free with
/// [`platypus_library_close`].
///
/// `progress`, if non-null, is invoked as `progress(ctx, phase, done, total)` during
/// the load — **phase 1** = reading files, **phase 2** = indexing coverage — so the UI
/// can show a real percentage off the main thread instead of a frozen window.
///
/// # Safety
/// `dir` must be a valid NUL-terminated C string. `progress` (if set) must be a valid
/// function pointer and `ctx` must outlive the call.
#[no_mangle]
pub unsafe extern "C" fn platypus_library_open(
    dir: *const c_char,
    ctx: *mut c_void,
    progress: PlatypusProgressFn,
) -> *mut PlatypusLibrary {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let report = |phase: u32, done: usize, total: usize| {
            if let Some(f) = progress {
                f(ctx, phase, done as u32, total as u32);
            }
        };

        let Some(dir) = cstr_to_str(dir) else {
            return ptr::null_mut();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return ptr::null_mut();
        };
        // List the state files first so we know the total for the progress bar.
        let mut files: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                n.starts_with("s_") && n.ends_with(".hpd")
            })
            .collect();
        files.sort();

        let total = files.len();
        let mut docs = Vec::with_capacity(total);
        for (i, path) in files.iter().enumerate() {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(doc) = Document::parse(&bytes) {
                    docs.push(doc);
                }
            }
            report(1, i + 1, total);
        }

        // Detect the model from the first parsed file's header (a card is one model);
        // null if the folder has no parseable, supported HPDB file.
        let Some(profile) = docs.first().and_then(detect_profile) else {
            return ptr::null_mut();
        };

        // County master (`hpdb.cfg`, alongside the state files) gives county→state for
        // the centroids; absent ⇒ empty index (geo placement degrades, never panics).
        let counties = std::fs::read(Path::new(dir).join("hpdb.cfg"))
            .ok()
            .and_then(|b| Document::parse(&b).ok())
            .map(|d| CountyIndex::from_hpdb(&d, profile))
            .unwrap_or_default();

        // Build county centroids from conventional C-Groups, then precompute placement
        // (the slow geo-matching) — reported as phase 2.
        let mut cb = CentroidBuilder::new();
        for doc in &docs {
            cb.add_doc(doc, profile);
        }
        let centroids = cb.finish(&counties);
        let placement = build_placement(&docs, profile, &centroids, |done, total| {
            report(2, done, total)
        });

        Box::into_raw(Box::new(PlatypusLibrary {
            docs,
            profile,
            placement,
        }))
    })
}

/// Free a library handle. Safe with null.
///
/// # Safety
/// `handle` must come from [`platypus_library_open`], or be null.
#[no_mangle]
pub unsafe extern "C" fn platypus_library_close(handle: *mut PlatypusLibrary) {
    ffi_guard((), move || unsafe {
        if !handle.is_null() {
            drop(Box::from_raw(handle));
        }
    })
}

/// JSON `{files, systems, channels}` for a load summary.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_library_stats_json(
    handle: *const PlatypusLibrary,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(lib) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let mut systems = 0usize;
        let mut channels = 0usize;
        for doc in &lib.docs {
            for sys in Extraction::segment(doc, lib.profile).systems() {
                systems += 1;
                channels += count_channels(&sys, lib.profile, &Filter::ALL);
            }
        }
        to_c_string(format!(
            "{{\"files\":{},\"systems\":{},\"channels\":{}}}",
            lib.docs.len(),
            systems,
            channels
        ))
    })
}

/// Filtered catalog: **system-level** rows matching the filter (no channels — the
/// UI lazy-loads those per system via [`platypus_library_channels_json`]). Each
/// row: `{id,name,kind,tech,counties,siteCount,channelCount}` where `channelCount`
/// is the number of channels passing the current filter. A system appears only if
/// it passes the tech filter and has ≥1 matching channel.
///
/// Filter params (all optional — empty string / false = no constraint):
/// - `services_csv`: service-type codes, e.g. `"3,8"` (channel-level).
/// - `techs_csv`: tech categories, e.g. `"P25,DMR,Analog"` (system-level, fuzzy).
/// - `search`: case-insensitive match on system name or channel name.
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_library_catalog_json(
    handle: *const PlatypusLibrary,
    services_csv: *const c_char,
    techs_csv: *const c_char,
    search: *const c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(lib) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let filter = Filter::from_c(services_csv, techs_csv, search);

        let mut out = String::from("[");
        let mut first = true;
        for (di, doc) in lib.docs.iter().enumerate() {
            for (si, sys) in Extraction::segment(doc, lib.profile).systems().enumerate() {
                if !filter.system_tech_ok(&sys) {
                    continue;
                }
                let passing = passing_channels(&sys, lib.profile, &filter);
                if passing.is_empty() {
                    continue;
                }
                // Covered counties + statewide flag from the precomputed placement,
                // intersected with the channels that pass the current filter.
                let placement = lib.placement.get(&(di, si));
                let states: &[u64] = placement.map(|p| p.states.as_slice()).unwrap_or(&[]);
                let mut counties: Vec<u64> = Vec::new();
                let mut statewide = false;
                if let Some(p) = placement {
                    for (&county, cis) in &p.by_county {
                        if !cis.iter().any(|c| passing.contains(c)) {
                            continue;
                        }
                        if county == 0 {
                            statewide = true;
                        } else {
                            counties.push(county);
                        }
                    }
                }
                if !first {
                    out.push(',');
                }
                first = false;
                push_catalog_row(
                    &mut out,
                    di,
                    si,
                    &sys,
                    states,
                    &counties,
                    statewide,
                    passing.len(),
                );
            }
        }
        out.push(']');
        to_c_string(out)
    })
}

/// Channels of one system (by `id` = `"d<di>s<si>"` from a catalog row), filtered
/// the same way as the catalog. This is the lazy-load on expand. Empty array on a
/// bad id.
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_library_channels_json(
    handle: *const PlatypusLibrary,
    system_id: *const c_char,
    services_csv: *const c_char,
    search: *const c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(lib) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let Some(id) = cstr_to_str(system_id) else {
            return to_c_string("[]".into());
        };
        let Some((di, si)) = parse_system_id(id) else {
            return to_c_string("[]".into());
        };
        let Some(doc) = lib.docs.get(di) else {
            return to_c_string("[]".into());
        };
        let ext = Extraction::segment(doc, lib.profile);
        let Some(sys) = ext.systems().nth(si) else {
            return to_c_string("[]".into());
        };
        // Tech is a system-level filter; once a system is shown it has passed, so the
        // per-system channel list doesn't re-apply it (null techs).
        let filter = Filter::from_c(services_csv, ptr::null(), search);

        let mut out = String::from("[");
        push_system_channels(&mut out, di, si, &sys, lib.profile, &filter, None);
        out.push(']');
        to_c_string(out)
    })
}

/// County-scoped channels of one system: only the channels whose group is placed in
/// `county` (location-first). Use this when the system was opened from inside a
/// county, so a statewide system (e.g. SAFE-T) shows just that county's talkgroups.
/// `county == 0` returns the **no-county** bucket (the state-level statewide-residual
/// channels). Channel ids keep their system-wide index so the cart/writer are
/// unaffected. Empty array on a bad id / unknown placement.
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_library_county_channels_json(
    handle: *const PlatypusLibrary,
    system_id: *const c_char,
    county: u64,
    services_csv: *const c_char,
    search: *const c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(lib) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let Some(id) = cstr_to_str(system_id) else {
            return to_c_string("[]".into());
        };
        let Some((di, si)) = parse_system_id(id) else {
            return to_c_string("[]".into());
        };
        let Some(doc) = lib.docs.get(di) else {
            return to_c_string("[]".into());
        };
        let ext = Extraction::segment(doc, lib.profile);
        let Some(sys) = ext.systems().nth(si) else {
            return to_c_string("[]".into());
        };
        // The voice-channel indices placed in this county (0 = no-county bucket).
        let allowed: BTreeSet<usize> = lib
            .placement
            .get(&(di, si))
            .and_then(|p| p.by_county.get(&county))
            .map(|v| v.iter().copied().collect())
            .unwrap_or_default();

        let filter = Filter::from_c(services_csv, ptr::null(), search);
        let mut out = String::from("[");
        push_system_channels(&mut out, di, si, &sys, lib.profile, &filter, Some(&allowed));
        out.push(']');
        to_c_string(out)
    })
}

/// **Radius-scoped** channels of one system: only the voice channels whose parent
/// department (`C-Group`/`T-Group`) has its geo center within `miles` of
/// (`lat`, `lon`). The location-first "add only what's near here" for the map view —
/// e.g. add a statewide P25 system from a point and get just that area's talkgroups,
/// not every county's. Channel ids match `platypus_library_channels_json`
/// (`d<i>s<j>c<k>`), so the result feeds straight into the append/build path. Groups
/// with no geo (0,0) are excluded (not location-specific). Empty array on a bad id.
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_library_radius_channels_json(
    handle: *const PlatypusLibrary,
    system_id: *const c_char,
    lat: c_double,
    lon: c_double,
    miles: c_double,
    services_csv: *const c_char,
    search: *const c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(lib) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let Some(id) = cstr_to_str(system_id) else {
            return to_c_string("[]".into());
        };
        let Some((di, si)) = parse_system_id(id) else {
            return to_c_string("[]".into());
        };
        let Some(doc) = lib.docs.get(di) else {
            return to_c_string("[]".into());
        };
        let ext = Extraction::segment(doc, lib.profile);
        let Some(sys) = ext.systems().nth(si) else {
            return to_c_string("[]".into());
        };

        // Voice-channel indices whose current department's geo center is within range.
        // The `ci` count matches `push_system_channels` (TGID/C-Freq, in order).
        let mut allowed: BTreeSet<usize> = BTreeSet::new();
        let mut ci = 0usize;
        let mut dept_in_range = false;
        for line in sys.lines() {
            let cmd = line.command();
            if matches!(cmd, "C-Group" | "T-Group") {
                dept_in_range = Record::new(line, lib.profile)
                    .and_then(|r| r.geo())
                    .is_some_and(|g| {
                        g.lat != 0.0
                            && g.lon != 0.0
                            && haversine_miles(lat, lon, g.lat, g.lon) <= miles
                    });
            } else if is_voice_channel(cmd) {
                let this = ci;
                ci += 1;
                if dept_in_range {
                    allowed.insert(this);
                }
            }
        }

        let filter = Filter::from_c(services_csv, ptr::null(), search);
        let mut out = String::from("[");
        push_system_channels(&mut out, di, si, &sys, lib.profile, &filter, Some(&allowed));
        out.push(']');
        to_c_string(out)
    })
}

/// Geo-located systems within `miles` of (`lat`, `lon`) for the **map** view. Each
/// system that has a geo-bearing record (site/group) in range and ≥1 filter-passing
/// channel is returned with the in-range point nearest the center as its pin, the
/// point's coverage `rangeMi`, and the system's dominant service-type code (for the
/// pin color). Reuses the catalog filter params. Tap a pin → fetch its channels by
/// `id` (`platypus_library_channels_json`).
///
/// Shape: `[{ "id":"d<i>s<j>","name","kind","tech":str|null,"serviceType":u16|null,
///   "lat":f64,"lon":f64,"rangeMi":f64 }]`
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_library_geo_json(
    handle: *const PlatypusLibrary,
    lat: c_double,
    lon: c_double,
    miles: c_double,
    services_csv: *const c_char,
    techs_csv: *const c_char,
    search: *const c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(lib) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let filter = Filter::from_c(services_csv, techs_csv, search);

        let mut out = String::from("[");
        let mut first = true;
        for (di, doc) in lib.docs.iter().enumerate() {
            for (si, sys) in Extraction::segment(doc, lib.profile).systems().enumerate() {
                if !filter.system_tech_ok(&sys) {
                    continue;
                }
                // Nearest in-range geo point of this system.
                let mut best: Option<(f64, platypus_core::model::Geo)> = None;
                for line in sys.lines() {
                    if let Some(g) = Record::new(line, lib.profile).and_then(|r| r.geo()) {
                        if g.lat == 0.0 && g.lon == 0.0 {
                            continue;
                        }
                        let d = haversine_miles(lat, lon, g.lat, g.lon);
                        if d <= miles && best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, g));
                        }
                    }
                }
                let Some((_, geo)) = best else { continue };
                let passing = passing_channels(&sys, lib.profile, &filter);
                if passing.is_empty() {
                    continue;
                }

                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str("{\"id\":");
                push_json_string(&mut out, &format!("d{di}s{si}"));
                out.push_str(",\"name\":");
                push_json_string(&mut out, sys.name().unwrap_or(""));
                out.push_str(",\"kind\":");
                push_json_string(&mut out, sys.header().command());
                out.push_str(",\"tech\":");
                match sys.tech() {
                    Some(t) => push_json_string(&mut out, t),
                    None => out.push_str("null"),
                }
                out.push_str(",\"serviceType\":");
                match dominant_service_type(&sys, lib.profile, &passing) {
                    Some(c) => out.push_str(&c.to_string()),
                    None => out.push_str("null"),
                }
                out.push_str(",\"lat\":");
                push_f64(&mut out, geo.lat);
                out.push_str(",\"lon\":");
                push_f64(&mut out, geo.lon);
                out.push_str(",\"rangeMi\":");
                push_f64(&mut out, geo.range_mi);
                out.push('}');
            }
        }
        out.push(']');
        to_c_string(out)
    })
}

/// The most common service-type code among a system's filter-passing channels.
fn dominant_service_type(
    sys: &System,
    profile: &dyn SdCardProfile,
    passing: &BTreeSet<usize>,
) -> Option<u16> {
    let mut counts: HashMap<u16, usize> = HashMap::new();
    let mut ci = 0usize;
    for line in sys.lines() {
        if !is_voice_channel(line.command()) {
            continue;
        }
        let this = ci;
        ci += 1;
        if !passing.contains(&this) {
            continue;
        }
        if let Some(code) = Record::new(line, profile).and_then(|r| r.service_type_code()) {
            *counts.entry(code).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c)
}

/// If `volume_root` is a mounted scanner card — it has a recognized model folder whose
/// HPDB directory holds state files (`s_*.hpd`) — return that HPDB directory's absolute
/// path (caller frees with [`platypus_string_free`]); null otherwise. Lets the app
/// auto-discover a connected card under `/Volumes` instead of making the user navigate.
///
/// # Safety
/// `volume_root` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_card_hpdb_dir(volume_root: *const c_char) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(vol) = cstr_to_str(volume_root) else {
            return ptr::null_mut();
        };
        let vol = Path::new(vol);
        for profile in registry().sd_card_profiles() {
            let layout = profile.sd_layout();
            let hpdb = vol.join(layout.model_folder).join(layout.hpdb_dir);
            let has_state = std::fs::read_dir(&hpdb).is_ok_and(|rd| {
                rd.flatten().any(|e| {
                    let n = e.file_name();
                    let n = n.to_str().unwrap_or("");
                    n.starts_with("s_") && n.ends_with(".hpd")
                })
            });
            if has_state {
                if let Some(s) = hpdb.to_str() {
                    return to_c_string(s.to_string());
                }
            }
        }
        ptr::null_mut()
    })
}

// ---- card favorites management (read existing lists, scanner limits, delete) ----

/// Detect the scanner profile of a mounted card by reading its `f_list.cfg` header.
fn detect_card(mount: &Path) -> Option<&'static dyn SdCardProfile> {
    for profile in registry().sd_card_profiles() {
        if let Ok(bytes) = std::fs::read(card::f_list_path(mount, profile)) {
            if let Ok(doc) = Document::parse(&bytes) {
                if let Some(p) = registry().detect(&doc.header()) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// The favorites lists currently on a mounted card, plus the detected scanner model
/// and its limits — for seeing/managing the existing configuration. `card_mount` is
/// the volume root (the parent of the model folder). Null if no supported card is
/// found there.
///
/// Shape: `{ "model":str, "modelId":str|null, "maxFavorites":u32,
///   "lists":[{ "slot":u32,"name":str,"filename":str,"systems":u64,
///              "channels":u64,"bytes":u64,"monitor":bool,
///              "quickKey":u32|null,"numberTag":u32|null }] }`
/// (`maxFavorites` 0 = unknown; `monitor`/`quickKey`/`numberTag` = F-List settings;
/// null quick key / number tag = "Off").
///
/// # Safety
/// `card_mount` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_card_favorites_json(card_mount: *const c_char) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(mount) = cstr_to_str(card_mount) else {
            return ptr::null_mut();
        };
        let mount = Path::new(mount);
        let Some(profile) = detect_card(mount) else {
            return ptr::null_mut();
        };
        let Ok(lists) = card::read_favorites_lists(mount, profile) else {
            return ptr::null_mut();
        };

        let mut out = String::from("{\"model\":");
        push_json_string(&mut out, profile.product_name());
        out.push_str(",\"modelId\":");
        match profile.serial_model_id() {
            Some(id) => push_json_string(&mut out, id),
            None => out.push_str("null"),
        }
        let limits = profile.limits();
        out.push_str(",\"maxFavorites\":");
        out.push_str(&limits.max_favorites_lists.to_string());
        out.push_str(",\"maxListBytes\":");
        out.push_str(&limits.max_favorite_list_bytes.to_string());
        out.push_str(",\"quickKeys\":");
        out.push_str(&limits.quick_keys.to_string());
        // The card model's enumerated per-channel value options (e.g. alert light/pattern/tone/
        // volume) — the single source the favorites editor's menus read; the app adds only
        // presentation (the color swatch). `{ field: [values] }`, one entry per field that has
        // an enumeration (free-form/numeric fields are omitted).
        out.push_str(",\"channelValueOptions\":{");
        let mut first = true;
        for field in profile.channel_value_fields() {
            let options = profile.channel_value_options(field);
            if options.is_empty() {
                continue;
            }
            if !first {
                out.push(',');
            }
            first = false;
            push_json_string(&mut out, field);
            out.push(':');
            out.push('[');
            for (i, v) in options.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_json_string(&mut out, v);
            }
            out.push(']');
        }
        out.push('}');
        out.push_str(",\"lists\":[");
        for (i, l) in lists.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"slot\":");
            out.push_str(&l.slot.to_string());
            out.push_str(",\"name\":");
            push_json_string(&mut out, &l.name);
            out.push_str(",\"filename\":");
            push_json_string(&mut out, &l.filename);
            out.push_str(",\"systems\":");
            out.push_str(&l.systems.to_string());
            out.push_str(",\"channels\":");
            out.push_str(&l.channels.to_string());
            out.push_str(",\"bytes\":");
            out.push_str(&l.bytes.to_string());
            out.push_str(",\"monitor\":");
            out.push_str(if l.monitor { "true" } else { "false" });
            out.push_str(",\"quickKey\":");
            match l.quick_key {
                Some(n) => out.push_str(&n.to_string()),
                None => out.push_str("null"),
            }
            out.push_str(",\"numberTag\":");
            match l.number_tag {
                Some(n) => out.push_str(&n.to_string()),
                None => out.push_str("null"),
            }
            out.push('}');
        }
        out.push_str("]}");
        to_c_string(out)
    })
}

/// Sort the card's favorites lists **alphabetically by name** (the order shown on the
/// scanner). Returns null on success, or an error string. Caller must still eject.
///
/// # Safety
/// `card_mount` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_card_sort_lists(card_mount: *const c_char) -> *mut c_char {
    ffi_guard(to_c_string("internal error".to_string()), move || unsafe {
        let err = |m: &str| to_c_string(m.to_string());
        let Some(mount) = cstr_to_str(card_mount) else {
            return err("invalid card path");
        };
        let mount = Path::new(mount);
        let Some(profile) = detect_card(mount) else {
            return err("no supported scanner card found at that path");
        };
        match card::sort_favorites_lists(mount, profile) {
            Ok(()) => ptr::null_mut(),
            Err(e) => err(&e.to_string()),
        }
    })
}

/// Reorder the card's favorites lists into an explicit slot order (`slots_csv`, e.g.
/// `"3,1,2"`); slots not listed keep their order at the end. For manual (drag/up-down)
/// reordering. Returns null on success, or an error string. Caller must still eject.
///
/// # Safety
/// `card_mount`/`slots_csv` must be valid NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn platypus_card_reorder_lists(
    card_mount: *const c_char,
    slots_csv: *const c_char,
) -> *mut c_char {
    ffi_guard(to_c_string("internal error".to_string()), move || unsafe {
        let err = |m: &str| to_c_string(m.to_string());
        let Some(mount) = cstr_to_str(card_mount) else {
            return err("invalid card path");
        };
        let mount = Path::new(mount);
        let Some(profile) = detect_card(mount) else {
            return err("no supported scanner card found at that path");
        };
        let slots: Vec<u32> = cstr_to_str(slots_csv)
            .unwrap_or("")
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        match card::reorder_favorites_lists(mount, profile, &slots) {
            Ok(()) => ptr::null_mut(),
            Err(e) => err(&e.to_string()),
        }
    })
}

/// Sort the systems of a favorites list **alphabetically**, returning a **new**
/// handle. Save with [`platypus_favorites_commit`]. Null on a null handle.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_sort(
    handle: *const PlatypusFavorites,
) -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = handle.as_ref() else {
            return ptr::null_mut();
        };
        Box::into_raw(Box::new(PlatypusFavorites {
            doc: favorites::sort_systems(&f.doc, f.profile),
            profile: f.profile,
        }))
    })
}

/// Delete a favorites list (slot file + `f_list.cfg` entry + `app_data.cfg`) from a
/// mounted card. Returns **null on success**, or an error message (free it) on
/// failure. The caller must still **eject** the volume afterward.
///
/// # Safety
/// `card_mount` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_card_delete_slot(
    card_mount: *const c_char,
    slot: u32,
) -> *mut c_char {
    ffi_guard(to_c_string("internal error".to_string()), move || unsafe {
        let err = |m: &str| to_c_string(m.to_string());
        let Some(mount) = cstr_to_str(card_mount) else {
            return err("invalid card path");
        };
        let mount = Path::new(mount);
        let Some(profile) = detect_card(mount) else {
            return err("no supported scanner card found at that path");
        };
        match card::delete_favorites_slot(mount, profile, slot) {
            Ok(()) => ptr::null_mut(),
            Err(e) => err(&e.to_string()),
        }
    })
}

/// JSON array of states from an `hpdb.cfg` — `[{id,name,abbr,country}]`, the top of
/// the Country → State → County hierarchy. Parses `StateInfo` lines directly.
/// Stateless (no handle). Null on failure.
///
/// # Safety
/// `path` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_states_json(path: *const c_char) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(path) = cstr_to_str(path) else {
            return ptr::null_mut();
        };
        let Ok(bytes) = std::fs::read(path) else {
            return ptr::null_mut();
        };
        let Ok(doc) = Document::parse(&bytes) else {
            return ptr::null_mut();
        };

        let mut out = String::from("[");
        let mut first = true;
        for line in &doc.lines {
            if line.command() != "StateInfo" {
                continue;
            }
            let Some(id) = line.field(1).and_then(keyed_id) else {
                continue;
            };
            let country = line.field(2).and_then(keyed_id).unwrap_or(0);
            let name = line.field(3).unwrap_or("");
            let abbr = line.field(4).unwrap_or("");
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str("{\"id\":");
            out.push_str(&id.to_string());
            out.push_str(",\"name\":");
            push_json_string(&mut out, name);
            out.push_str(",\"abbr\":");
            push_json_string(&mut out, abbr);
            out.push_str(",\"country\":");
            out.push_str(&country.to_string());
            out.push('}');
        }
        out.push(']');
        to_c_string(out)
    })
}

/// JSON array of counties from an `hpdb.cfg` at `path` — `[{id,name,state}]`.
/// Stateless (no handle). Null on failure.
///
/// # Safety
/// `path` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_counties_json(path: *const c_char) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(path) = cstr_to_str(path) else {
            return ptr::null_mut();
        };
        let Ok(bytes) = std::fs::read(path) else {
            return ptr::null_mut();
        };
        let Ok(doc) = Document::parse(&bytes) else {
            return ptr::null_mut();
        };
        let Some(profile) = detect_profile(&doc) else {
            return ptr::null_mut();
        };
        let index = CountyIndex::from_hpdb(&doc, profile);

        let mut out = String::from("[");
        for (i, c) in index.counties().iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"id\":");
            out.push_str(&c.id.to_string());
            out.push_str(",\"name\":");
            push_json_string(&mut out, &c.name);
            out.push_str(",\"state\":");
            out.push_str(
                &c.state_id
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "null".into()),
            );
            out.push('}');
        }
        out.push(']');
        to_c_string(out)
    })
}

// ---- favorites pipeline (filter → build → preview → commit) ----

/// Build a favorites document from a source HPDB by selection + options. The whole
/// pipeline we validated on hardware, callable from any UI:
/// - `selector`: `"all"`, `"county:712"`, `"tech:P25"`, or `"name:SAFE-T"`.
/// - `near_miles > 0`: also prune sites + talkgroup groups to within that many miles
///   of (`near_lat`, `near_lon`) — the location-first filter. `<= 0` skips it.
/// - `departments_on`: the DQKs quick-key preference (both values work).
/// - `band_plan`: add a band plan to kept P25 sites (rare; needed by some simulcasts).
///
/// Returns an opaque handle (free with [`platypus_favorites_free`]), or null on a
/// bad selection / null source.
///
/// # Safety
/// `src` must be valid or null; `selector` a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_build(
    src: *const PlatypusHpdb,
    selector: *const c_char,
    near_lat: c_double,
    near_lon: c_double,
    near_miles: c_double,
    departments_on: bool,
    band_plan: bool,
) -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(src) = src.as_ref() else {
            return ptr::null_mut();
        };
        let Some(selector) = cstr_to_str(selector) else {
            return ptr::null_mut();
        };
        let profile = src.profile;

        let Some(mut selection) = select(&src.doc, profile, selector) else {
            return ptr::null_mut();
        };
        if near_miles > 0.0 {
            selection =
                extract::filter_within_radius(&selection, profile, near_lat, near_lon, near_miles);
        }

        let mut doc = favorites::build_favorites(&selection, profile, departments_on);
        if band_plan {
            doc = favorites::with_synthesized_bandplan(&doc, profile);
        }
        Box::into_raw(Box::new(PlatypusFavorites { doc, profile }))
    })
}

/// Build favorites from an explicit set of channel ids (the selection cart) over a
/// loaded [`PlatypusLibrary`]. `channel_ids_csv` is a comma-separated list of catalog
/// ids (`"d<di>s<si>c<ci>"`). Channels are grouped by source file + system, subset
/// (keeping only the chosen voice channels plus the scaffolding needed to tune them:
/// header, sites, control freqs, parent groups), merged under one preamble, then run
/// through the validated favorites build. Returns a handle (preview/commit via the
/// other favorites functions), or null on bad input / empty selection.
///
/// # Safety
/// `lib` valid or null; `channel_ids_csv` a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_from_channels(
    lib: *const PlatypusLibrary,
    channel_ids_csv: *const c_char,
    departments_on: bool,
    band_plan: bool,
) -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(lib) = lib.as_ref() else {
            return ptr::null_mut();
        };
        let Some(csv) = cstr_to_str(channel_ids_csv) else {
            return ptr::null_mut();
        };
        match build_favorites_doc(lib, csv, departments_on, band_plan) {
            Some(doc) => Box::into_raw(Box::new(PlatypusFavorites {
                doc,
                profile: lib.profile,
            })),
            None => ptr::null_mut(),
        }
    })
}

/// Build a favorites `Document` from library channel ids (`"d<di>s<si>c<ci>"`).
/// Shared by the from-channels builder and the edit-time append. None if empty.
fn build_favorites_doc(
    lib: &PlatypusLibrary,
    csv: &str,
    departments_on: bool,
    band_plan: bool,
) -> Option<Document> {
    let mut grouped: BTreeMap<usize, BTreeMap<usize, BTreeSet<usize>>> = BTreeMap::new();
    for id in csv.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((di, si, ci)) = parse_channel_id(id) {
            grouped
                .entry(di)
                .or_default()
                .entry(si)
                .or_default()
                .insert(ci);
        }
    }
    if grouped.is_empty() {
        return None;
    }

    let profile = lib.profile;
    let mut lines = Vec::new();
    let mut preamble_set = false;
    for (di, si_map) in &grouped {
        let Some(doc) = lib.docs.get(*di) else {
            continue;
        };
        let ext = Extraction::segment(doc, profile);
        if !preamble_set {
            lines.extend_from_slice(ext.preamble_lines());
            preamble_set = true;
        }
        lines.extend(ext.subset_system_lines(si_map));
    }
    let source = Document { lines };
    let mut doc = favorites::build_favorites(&source, profile, departments_on);
    if band_plan {
        doc = favorites::with_synthesized_bandplan(&doc, profile);
    }
    Some(doc)
}

/// Parse a catalog channel id `"d<di>s<si>c<ci>"` into `(doc, system, channel)`.
fn parse_channel_id(id: &str) -> Option<(usize, usize, usize)> {
    let rest = id.strip_prefix('d')?;
    let (di, rest) = rest.split_once('s')?;
    let (si, ci) = rest.split_once('c')?;
    Some((di.parse().ok()?, si.parse().ok()?, ci.parse().ok()?))
}

/// Parse a favorites channel id `"s<si>c<ci>"` (within one list) into `(system, channel)`.
fn parse_fav_channel_id(id: &str) -> Option<(usize, usize)> {
    let rest = id.strip_prefix('s')?;
    let (si, ci) = rest.split_once('c')?;
    Some((si.parse().ok()?, ci.parse().ok()?))
}

// ---- favorites editing (open an existing list, remove channels, append, rename) ----

/// A new, empty favorites list (just the model header) — for building a list from
/// scratch (the "+ New list" flow). Grow it with
/// [`platypus_favorites_append_from_library`], then name + commit. Null if no scanner
/// model is registered.
///
/// # Safety
/// Takes no arguments; free the returned handle with [`platypus_favorites_free`].
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_new() -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || {
        let Some(profile) = registry().sd_card_profiles().next() else {
            return ptr::null_mut();
        };
        let key = profile.model_key();
        let line = |fields: Vec<String>| Line {
            fields,
            ending: LineEnding::Crlf,
        };
        let doc = Document {
            lines: vec![
                line(vec!["TargetModel".to_string(), key.target_model]),
                line(vec!["FormatVersion".to_string(), key.format_version]),
            ],
        };
        Box::into_raw(Box::new(PlatypusFavorites { doc, profile }))
    })
}

/// Open an existing favorites list from a card slot as an editable handle (parse the
/// `f_<slot>.hpd` file). Null if the card/model/slot can't be read. Free with
/// [`platypus_favorites_free`]; inspect via [`platypus_favorites_channels_json`].
///
/// # Safety
/// `card_mount` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_open(
    card_mount: *const c_char,
    slot: u32,
) -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(mount) = cstr_to_str(card_mount) else {
            return ptr::null_mut();
        };
        let mount = Path::new(mount);
        let Some(profile) = detect_card(mount) else {
            return ptr::null_mut();
        };
        let Ok(bytes) = std::fs::read(card::favorites_path(mount, profile, slot)) else {
            return ptr::null_mut();
        };
        let Ok(doc) = Document::parse(&bytes) else {
            return ptr::null_mut();
        };
        Box::into_raw(Box::new(PlatypusFavorites { doc, profile }))
    })
}

/// The systems + channels in a favorites list (same shape as `platypus_catalog_json`:
/// channel ids are `"s<si>c<ci>"`), for the edit UI.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_channels_json(
    handle: *const PlatypusFavorites,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = handle.as_ref() else {
            return ptr::null_mut();
        };
        to_c_string(catalog_json(&f.doc, f.profile))
    })
}

/// A favorites list as a **scan/avoid tree**: systems → departments (groups) →
/// channels, each carrying its `avoid` flag, so the UI can show and toggle what the
/// radio actually scans. Channel ids match `platypus_favorites_channels_json`
/// (`"s<si>c<ci>"`, system-wide voice index); group ids are `"s<si>g<gi>"`.
///
/// Shape: `[{ "id","name","kind","tech":str|null,"avoid":bool,
///   "groups":[{ "id","name","avoid":bool,
///     "channels":[{ "id","name","tgid":str|null,"freqHz":u64|null,"mode":str|null,
///       "serviceType":u16|null,"tone":str|null,"avoid":bool,"priority":bool,
///       "settings":{ <field>:str … } }] }] }]`  // `tone` = raw audio option; settings = editable fields
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_tree_json(
    handle: *const PlatypusFavorites,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = handle.as_ref() else {
            return ptr::null_mut();
        };
        to_c_string(favorites_tree_json(&f.doc, f.profile))
    })
}

/// Set the **Avoid** flag on one record (scan ⇄ skip), returning a **new** handle.
/// `target` is `"s<si>"` (system), `"s<si>g<gi>"` (department), or `"s<si>c<ci>"`
/// (channel). Null on a null handle / unparseable target.
///
/// # Safety
/// `handle` valid or null; `target` a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_set_avoid(
    handle: *const PlatypusFavorites,
    target: *const c_char,
    avoid: bool,
) -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let Some((si, level, idx)) = cstr_to_str(target).and_then(parse_avoid_target) else {
            return ptr::null_mut();
        };
        Box::into_raw(Box::new(PlatypusFavorites {
            doc: favorites::set_avoid(&f.doc, f.profile, si, level, idx, avoid),
            profile: f.profile,
        }))
    })
}

/// Set a channel's **Priority Channel** flag, returning a **new** handle. `target`
/// is a channel id `"s<si>c<ci>"` (system/department targets are ignored — priority
/// is per-channel). Null on a null handle / unparseable target.
///
/// # Safety
/// `handle` valid or null; `target` a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_set_priority(
    handle: *const PlatypusFavorites,
    target: *const c_char,
    on: bool,
) -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let Some((si, level, idx)) = cstr_to_str(target).and_then(parse_avoid_target) else {
            return ptr::null_mut();
        };
        if level != favorites::AvoidLevel::Channel {
            return ptr::null_mut();
        }
        Box::into_raw(Box::new(PlatypusFavorites {
            doc: favorites::set_priority(&f.doc, f.profile, si, idx, on),
            profile: f.profile,
        }))
    })
}

/// Set an editable per-channel **value** field (e.g. `field` = "delay") on a channel,
/// returning a **new** handle. `target` is a channel id `"s<si>c<ci>"`; `value` is the
/// raw field value. Null on a null handle / unparseable (non-channel) target.
///
/// # Safety
/// `handle` valid or null; `target`/`field`/`value` valid NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_set_channel_value(
    handle: *const PlatypusFavorites,
    target: *const c_char,
    field: *const c_char,
    value: *const c_char,
) -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let (Some((si, level, idx)), Some(field), Some(value)) = (
            cstr_to_str(target).and_then(parse_avoid_target),
            cstr_to_str(field),
            cstr_to_str(value),
        ) else {
            return ptr::null_mut();
        };
        if level != favorites::AvoidLevel::Channel {
            return ptr::null_mut();
        }
        Box::into_raw(Box::new(PlatypusFavorites {
            doc: favorites::set_channel_value(&f.doc, f.profile, si, idx, field, value),
            profile: f.profile,
        }))
    })
}

/// `"s2"` → system 2; `"s2g1"` → group 1; `"s2c5"` → channel 5.
fn parse_avoid_target(s: &str) -> Option<(usize, favorites::AvoidLevel, usize)> {
    let rest = s.strip_prefix('s')?;
    if let Some((si, gi)) = rest.split_once('g') {
        Some((
            si.parse().ok()?,
            favorites::AvoidLevel::Group,
            gi.parse().ok()?,
        ))
    } else if let Some((si, ci)) = rest.split_once('c') {
        Some((
            si.parse().ok()?,
            favorites::AvoidLevel::Channel,
            ci.parse().ok()?,
        ))
    } else {
        Some((rest.parse().ok()?, favorites::AvoidLevel::System, 0))
    }
}

/// Read a record's avoid flag (`"On"` = avoided), per the profile's avoid column.
fn record_avoided(line: &Line, profile: &dyn SdCardProfile) -> bool {
    profile
        .avoid_column(line.command())
        .and_then(|c| line.field(c))
        .is_some_and(|v| v.eq_ignore_ascii_case("On"))
}

/// Read a channel's Priority Channel flag (`"On"` = priority), per the profile.
fn record_priority(line: &Line, profile: &dyn SdCardProfile) -> bool {
    profile
        .priority_column(line.command())
        .and_then(|c| line.field(c))
        .is_some_and(|v| v.eq_ignore_ascii_case("On"))
}

/// Build the scan/avoid tree JSON (see `platypus_favorites_tree_json`).
fn favorites_tree_json(doc: &Document, profile: &dyn SdCardProfile) -> String {
    let ext = Extraction::segment(doc, profile);
    let mut out = String::from("[");
    for (si, sys) in ext.systems().enumerate() {
        if si > 0 {
            out.push(',');
        }
        let header = sys.header();
        out.push_str("{\"id\":");
        push_json_string(&mut out, &format!("s{si}"));
        out.push_str(",\"name\":");
        push_json_string(&mut out, sys.name().unwrap_or(""));
        out.push_str(",\"kind\":");
        push_json_string(&mut out, header.command());
        out.push_str(",\"tech\":");
        match sys.tech() {
            Some(t) => push_json_string(&mut out, t),
            None => out.push_str("null"),
        }
        out.push_str(",\"avoid\":");
        out.push_str(if record_avoided(header, profile) {
            "true"
        } else {
            "false"
        });
        out.push_str(",\"groups\":[");

        let mut gi = 0usize; // department index
        let mut ci = 0usize; // system-wide voice-channel index (matches channels_json)
        let mut group_first = true;
        let mut chan_first = true;
        let mut group_open = false;

        let open_group = |out: &mut String,
                          gi: &mut usize,
                          group_first: &mut bool,
                          chan_first: &mut bool,
                          id: String,
                          name: &str,
                          avoid: bool| {
            if !*group_first {
                out.push(',');
            }
            *group_first = false;
            out.push_str("{\"id\":");
            push_json_string(out, &id);
            out.push_str(",\"name\":");
            push_json_string(out, name);
            out.push_str(",\"avoid\":");
            out.push_str(if avoid { "true" } else { "false" });
            out.push_str(",\"channels\":[");
            *chan_first = true;
            *gi += 1;
        };

        for line in sys.lines() {
            let cmd = line.command();
            if matches!(cmd, "C-Group" | "T-Group") {
                if group_open {
                    out.push_str("]}");
                }
                let name = Record::new(line, profile)
                    .and_then(|r| r.name())
                    .unwrap_or("");
                let id = format!("s{si}g{gi}");
                let av = record_avoided(line, profile);
                open_group(
                    &mut out,
                    &mut gi,
                    &mut group_first,
                    &mut chan_first,
                    id,
                    name,
                    av,
                );
                group_open = true;
            } else if is_voice_channel(cmd) {
                if !group_open {
                    // Channels before any department → a synthetic unnamed group.
                    let id = format!("s{si}g{gi}");
                    open_group(
                        &mut out,
                        &mut gi,
                        &mut group_first,
                        &mut chan_first,
                        id,
                        "",
                        false,
                    );
                    group_open = true;
                }
                let this = ci;
                ci += 1;
                let Some(rec) = Record::new(line, profile) else {
                    continue;
                };
                if !chan_first {
                    out.push(',');
                }
                chan_first = false;
                out.push_str("{\"id\":");
                push_json_string(&mut out, &format!("s{si}c{this}"));
                out.push_str(",\"name\":");
                push_json_string(&mut out, rec.name().unwrap_or(""));
                out.push_str(",\"tgid\":");
                match rec.talkgroup() {
                    Some(t) => push_json_string(&mut out, t),
                    None => out.push_str("null"),
                }
                out.push_str(",\"freqHz\":");
                match rec.frequency_hz() {
                    Some(hz) => out.push_str(&hz.to_string()),
                    None => out.push_str("null"),
                }
                out.push_str(",\"mode\":");
                match rec.mode() {
                    Some(m) => push_json_string(&mut out, m),
                    None => out.push_str("null"),
                }
                out.push_str(",\"serviceType\":");
                match rec.service_type_code() {
                    Some(code) => out.push_str(&code.to_string()),
                    None => out.push_str("null"),
                }
                out.push_str(",\"tone\":");
                match rec.audio_option() {
                    Some(t) => push_json_string(&mut out, t),
                    None => out.push_str("null"),
                }
                out.push_str(",\"avoid\":");
                out.push_str(if record_avoided(line, profile) {
                    "true"
                } else {
                    "false"
                });
                out.push_str(",\"priority\":");
                out.push_str(if record_priority(line, profile) {
                    "true"
                } else {
                    "false"
                });
                // Editable per-channel value settings the model exposes for this
                // record type, as a { field: value } map (record-type-specific — e.g.
                // C-Freq has modulation/attenuator, TGID has audioType).
                out.push_str(",\"settings\":{");
                let mut sfirst = true;
                for field in profile.channel_value_fields() {
                    if let Some(v) = profile
                        .channel_value_column(cmd, field)
                        .and_then(|c| line.field(c))
                    {
                        if !sfirst {
                            out.push(',');
                        }
                        sfirst = false;
                        push_json_string(&mut out, field);
                        out.push(':');
                        push_json_string(&mut out, v);
                    }
                }
                out.push_str("}}");
            }
        }
        if group_open {
            out.push_str("]}");
        }
        out.push_str("]}");
    }
    out.push(']');
    out
}

/// Remove channels (by `"s<si>c<ci>"` id) from a favorites list, returning a **new**
/// handle. Systems left with no channels are dropped. Null on bad input.
///
/// # Safety
/// `handle` valid or null; `channel_ids_csv` a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_remove(
    handle: *const PlatypusFavorites,
    channel_ids_csv: *const c_char,
) -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let removed_csv = cstr_to_str(channel_ids_csv).unwrap_or("");
        let mut removed: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for id in removed_csv
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some((si, ci)) = parse_fav_channel_id(id) {
                removed.entry(si).or_default().insert(ci);
            }
        }

        // keep = every system's voice channels minus the removed ones; drop empty systems.
        let ext = Extraction::segment(&f.doc, f.profile);
        let mut keep: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for (si, sys) in ext.systems().enumerate() {
            let total = sys
                .lines()
                .iter()
                .filter(|l| is_voice_channel(l.command()))
                .count();
            let gone = removed.get(&si).cloned().unwrap_or_default();
            let survivors: BTreeSet<usize> = (0..total).filter(|c| !gone.contains(c)).collect();
            if !survivors.is_empty() {
                keep.insert(si, survivors);
            }
        }
        let doc = ext.subset_channels(&keep);
        Box::into_raw(Box::new(PlatypusFavorites {
            doc,
            profile: f.profile,
        }))
    })
}

/// Append library channels (`"d<di>s<si>c<ci>"`) to a favorites list, returning a
/// **new** handle (for "add to an existing list"). The added systems are built into
/// the favorites dialect and concatenated after the existing ones (no dedupe yet).
/// Null on bad input.
///
/// # Safety
/// `fav`/`lib` valid or null; `channel_ids_csv` a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_append_from_library(
    fav: *const PlatypusFavorites,
    lib: *const PlatypusLibrary,
    channel_ids_csv: *const c_char,
    departments_on: bool,
) -> *mut PlatypusFavorites {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = fav.as_ref() else {
            return ptr::null_mut();
        };
        let Some(lib) = lib.as_ref() else {
            return ptr::null_mut();
        };
        let Some(csv) = cstr_to_str(channel_ids_csv) else {
            return ptr::null_mut();
        };
        let Some(added) = build_favorites_doc(lib, csv, departments_on, false) else {
            return ptr::null_mut();
        };
        // Merge with dedupe: new channels join a matching system instead of duplicating it.
        Box::into_raw(Box::new(PlatypusFavorites {
            doc: favorites::merge_favorites(&f.doc, &added, f.profile),
            profile: f.profile,
        }))
    })
}

/// Free a favorites handle. Safe with null.
///
/// # Safety
/// `handle` must come from [`platypus_favorites_build`], or be null.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_free(handle: *mut PlatypusFavorites) {
    ffi_guard((), move || unsafe {
        if !handle.is_null() {
            drop(Box::from_raw(handle));
        }
    })
}

/// JSON summary for a write preview: `{systems, sites, dqks, bandPlans, bytes}`.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_summary_json(
    handle: *const PlatypusFavorites,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let count = |cmd: &str| f.doc.lines.iter().filter(|l| l.command() == cmd).count();
        let systems = Extraction::segment(&f.doc, f.profile).system_count();
        to_c_string(format!(
            "{{\"systems\":{},\"sites\":{},\"dqks\":{},\"bandPlans\":{},\"bytes\":{}}}",
            systems,
            count("Site"),
            count("DQKs_Status"),
            count("BandPlan_P25"),
            f.doc.to_bytes().len()
        ))
    })
}

/// The systems in a built favorites list (same shape as `platypus_systems_json`),
/// for the preview UI.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_systems_json(
    handle: *const PlatypusFavorites,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(f) = handle.as_ref() else {
            return ptr::null_mut();
        };
        to_c_string(systems_json(&f.doc, f.profile))
    })
}

/// Commit a built favorites list to a mounted card: write `f_<slot>.hpd`, register
/// it in `f_list.cfg`, delete `app_data.cfg`, all `fsync`'d. Returns **null on
/// success**, or an error message (free with [`platypus_string_free`]) on failure.
/// The caller must still **eject** the volume afterward (platform-specific).
///
/// # Safety
/// `handle` must be valid or null; `card_mount`/`label` valid NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_commit(
    handle: *const PlatypusFavorites,
    card_mount: *const c_char,
    slot: u32,
    label: *const c_char,
) -> *mut c_char {
    ffi_guard(to_c_string("internal error".to_string()), move || unsafe {
        let err = |m: &str| to_c_string(m.to_string());
        let Some(f) = handle.as_ref() else {
            return err("null favorites handle");
        };
        let (Some(mount), Some(label)) = (cstr_to_str(card_mount), cstr_to_str(label)) else {
            return err("invalid card path or label");
        };
        match card::commit_favorites(Path::new(mount), f.profile, slot, label, &f.doc) {
            Ok(()) => ptr::null_mut(),
            Err(e) => to_c_string(e.to_string()),
        }
    })
}

/// Write **only** this favorites list's slot file `f_<slot>.hpd` (fsync'd) — no
/// `f_list.cfg` / `app_data.cfg` change. The content half of the batched save:
/// write each changed list's slot, then one [`platypus_card_apply_layout`] pass.
/// Returns **null on success**, else an error (free it). Caller must **eject**.
///
/// # Safety
/// `handle` valid or null; `card_mount` a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn platypus_favorites_write_slot(
    handle: *const PlatypusFavorites,
    card_mount: *const c_char,
    slot: u32,
) -> *mut c_char {
    ffi_guard(to_c_string("internal error".to_string()), move || unsafe {
        let err = |m: &str| to_c_string(m.to_string());
        let Some(f) = handle.as_ref() else {
            return err("null favorites handle");
        };
        let Some(mount) = cstr_to_str(card_mount) else {
            return err("invalid card path");
        };
        match card::write_favorites_slot(Path::new(mount), f.profile, slot, &f.doc) {
            Ok(()) => ptr::null_mut(),
            Err(e) => to_c_string(e.to_string()),
        }
    })
}

/// Apply the full favorites layout in one structural pass: `entries` is the complete
/// ordered list of `slot \t name [\t monitor \t quickkey \t numbertag]` lines
/// separated by `\n` (TAB and LF can't occur in a card list name). The optional
/// trailing fields set that list's **Monitor** (`On`/`Off`), **Quick key**
/// (`Off`/`0`-`99`), and **NumberTag** (`Off`/`0`-`99`); an empty/absent field leaves
/// it unchanged. Those become the entire `f_list.cfg`; any previously-registered slot
/// not present is deleted; `app_data.cfg` is deleted. Every surviving list's other
/// F-List fields are preserved verbatim. Returns **null on success**, else an error
/// (free it). Caller must **eject**.
///
/// # Safety
/// `card_mount`/`entries` valid NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn platypus_card_apply_layout(
    card_mount: *const c_char,
    entries: *const c_char,
) -> *mut c_char {
    ffi_guard(to_c_string("internal error".to_string()), move || unsafe {
        let err = |m: &str| to_c_string(m.to_string());
        let Some(mount) = cstr_to_str(card_mount) else {
            return err("invalid card path");
        };
        let mount = Path::new(mount);
        let Some(profile) = detect_card(mount) else {
            return err("no supported scanner card found at that path");
        };
        // Non-empty trailing field = set; empty/absent = leave the existing value.
        let opt = |s: Option<&str>| s.filter(|v| !v.is_empty()).map(|v| v.to_string());
        let lists: Vec<card::ListLayout> = cstr_to_str(entries)
            .unwrap_or("")
            .split('\n')
            .filter(|l| !l.is_empty())
            .filter_map(|l| {
                let mut parts = l.split('\t');
                let slot: u32 = parts.next()?.trim().parse().ok()?;
                let name = parts.next()?.to_string();
                let monitor = match parts.next() {
                    Some("On") => Some(true),
                    Some("Off") => Some(false),
                    _ => None,
                };
                let quick_key = opt(parts.next());
                let number_tag = opt(parts.next());
                Some(card::ListLayout {
                    slot,
                    name,
                    monitor,
                    quick_key,
                    number_tag,
                })
            })
            .collect();
        match card::apply_favorites_layout(mount, profile, &lists) {
            Ok(()) => ptr::null_mut(),
            Err(e) => err(&e.to_string()),
        }
    })
}

/// Build a full-dialect selection from a source doc by selector string.
fn select(doc: &Document, profile: &dyn SdCardProfile, selector: &str) -> Option<Document> {
    let ext = Extraction::segment(doc, profile);
    if selector == "all" {
        return Some(ext.select(|_| true));
    }
    let (kind, value) = selector.split_once(':')?;
    Some(match kind {
        "county" => {
            let id: u64 = value.parse().ok()?;
            ext.select(|s| s.is_in_county(id))
        }
        "tech" => ext.select(|s| s.tech().is_some_and(|t| t.contains(value))),
        "name" => ext.select(|s| s.name().is_some_and(|n| n.contains(value))),
        _ => return None,
    })
}

// ---- JSON building (hand-rolled, dependency-free) ----

fn systems_json(doc: &Document, profile: &dyn SdCardProfile) -> String {
    let ext = Extraction::segment(doc, profile);
    let mut out = String::from("[");
    for (i, sys) in ext.systems().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_system_json(&mut out, &sys, profile);
    }
    out.push(']');
    out
}

fn push_system_json(out: &mut String, sys: &System, profile: &dyn SdCardProfile) {
    out.push_str("{\"name\":");
    push_json_string(out, sys.name().unwrap_or(""));
    out.push_str(",\"kind\":");
    push_json_string(out, sys.header().command());

    out.push_str(",\"counties\":[");
    for (i, c) in sys.county_ids().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&c.to_string());
    }
    out.push(']');

    out.push_str(",\"locations\":[");
    let mut first = true;
    for line in sys.lines() {
        let Some(rec) = Record::new(line, profile) else {
            continue;
        };
        let Some(geo) = rec.geo() else { continue };
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str("{\"name\":");
        push_json_string(out, rec.name().unwrap_or(""));
        out.push_str(",\"lat\":");
        push_f64(out, geo.lat);
        out.push_str(",\"lon\":");
        push_f64(out, geo.lon);
        out.push_str(",\"range\":");
        push_f64(out, geo.range_mi);
        out.push('}');
    }
    out.push_str("]}");
}

/// Channel-level catalog JSON (see `platypus_catalog_json`). Built from the same
/// `System` + `Record` views the canonical provider uses, so it stays in step with
/// the decoded attributes (tech, mode, service type, frequency, talkgroup).
fn catalog_json(doc: &Document, profile: &dyn SdCardProfile) -> String {
    let ext = Extraction::segment(doc, profile);
    let mut out = String::from("[");
    for (si, sys) in ext.systems().enumerate() {
        if si > 0 {
            out.push(',');
        }
        push_catalog_system(&mut out, si, &sys, profile);
    }
    out.push(']');
    out
}

fn push_catalog_system(out: &mut String, si: usize, sys: &System, profile: &dyn SdCardProfile) {
    let tech = Record::new(sys.header(), profile).and_then(|r| r.tech());
    let site_count = sys.lines().iter().filter(|l| l.command() == "Site").count();

    out.push_str("{\"id\":\"s");
    out.push_str(&si.to_string());
    out.push_str("\",\"name\":");
    push_json_string(out, sys.name().unwrap_or(""));
    out.push_str(",\"kind\":");
    push_json_string(out, sys.header().command());
    out.push_str(",\"tech\":");
    match tech {
        Some(t) => push_json_string(out, t),
        None => out.push_str("null"),
    }
    out.push_str(",\"counties\":[");
    for (i, c) in sys.county_ids().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&c.to_string());
    }
    out.push_str("],\"siteCount\":");
    out.push_str(&site_count.to_string());

    out.push_str(",\"channels\":[");
    let mut ci = 0usize;
    for line in sys.lines() {
        if !is_voice_channel(line.command()) {
            continue;
        }
        let Some(rec) = Record::new(line, profile) else {
            continue;
        };
        let freq = rec.frequency_hz();
        let tgid = rec.talkgroup();
        if ci > 0 {
            out.push(',');
        }
        out.push_str("{\"id\":\"s");
        out.push_str(&si.to_string());
        out.push('c');
        out.push_str(&ci.to_string());
        out.push_str("\",\"name\":");
        push_json_string(out, rec.name().unwrap_or(""));
        out.push_str(",\"kind\":");
        out.push_str(if tgid.is_some() {
            "\"Talkgroup\""
        } else {
            "\"Frequency\""
        });
        out.push_str(",\"tgid\":");
        match tgid {
            Some(t) => push_json_string(out, t),
            None => out.push_str("null"),
        }
        out.push_str(",\"freqHz\":");
        match freq {
            Some(hz) => out.push_str(&hz.to_string()),
            None => out.push_str("null"),
        }
        out.push_str(",\"mode\":");
        match rec.mode() {
            Some(m) => push_json_string(out, m),
            None => out.push_str("null"),
        }
        out.push_str(",\"serviceType\":");
        match rec.service_type_code() {
            Some(code) => out.push_str(&code.to_string()),
            None => out.push_str("null"),
        }
        out.push('}');
        ci += 1;
    }
    out.push_str("]}");
}

// ---- library filtering ----

/// A parsed catalog filter. All fields are "no constraint" when empty.
struct Filter {
    services: Vec<u16>,
    techs: Vec<String>,
    search: String, // already lowercased
}

impl Filter {
    const ALL: Filter = Filter {
        services: Vec::new(),
        techs: Vec::new(),
        search: String::new(),
    };

    /// Build from the raw C params (null-tolerant).
    unsafe fn from_c(
        services_csv: *const c_char,
        techs_csv: *const c_char,
        search: *const c_char,
    ) -> Filter {
        let services = cstr_to_str(services_csv)
            .unwrap_or("")
            .split(',')
            .filter_map(|s| s.trim().parse::<u16>().ok())
            .collect();
        let techs = cstr_to_str(techs_csv)
            .unwrap_or("")
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let search = cstr_to_str(search).unwrap_or("").trim().to_lowercase();
        Filter {
            services,
            techs,
            search,
        }
    }

    /// System-level tech match (fuzzy; empty = all). Categories alias to the raw
    /// tech vocabulary: `DMR`→MotoTrbo, `Analog`→Conventional, others by substring.
    fn system_tech_ok(&self, sys: &System) -> bool {
        if self.techs.is_empty() {
            return true;
        }
        let tech = sys.tech().unwrap_or("").to_lowercase();
        self.techs.iter().any(|cat| match cat.as_str() {
            "analog" | "conventional" => tech.contains("conventional"),
            "dmr" => tech.contains("dmr") || tech.contains("mototrbo"),
            other => tech.contains(other),
        })
    }

    /// Channel-level pass. `name_search_hit` lets a system-name match satisfy the
    /// text filter for all of its channels.
    fn channel_ok(&self, service: Option<u16>, name: &str, name_search_hit: bool) -> bool {
        if !self.services.is_empty() && !service.is_some_and(|c| self.services.contains(&c)) {
            return false;
        }
        if !self.search.is_empty()
            && !name_search_hit
            && !name.to_lowercase().contains(&self.search)
        {
            return false;
        }
        true
    }
}

/// Whether a system's own name satisfies the text search (then all its channels do).
fn name_hits(sys: &System, filter: &Filter) -> bool {
    filter.search.is_empty()
        || sys
            .name()
            .unwrap_or("")
            .to_lowercase()
            .contains(&filter.search)
}

/// Count the channels in a system that pass the filter.
fn count_channels(sys: &System, profile: &dyn SdCardProfile, filter: &Filter) -> usize {
    let hit = name_hits(sys, filter);
    let mut n = 0;
    for line in sys.lines() {
        if !is_voice_channel(line.command()) {
            continue;
        }
        let Some(rec) = Record::new(line, profile) else {
            continue;
        };
        if filter.channel_ok(rec.service_type_code(), rec.name().unwrap_or(""), hit) {
            n += 1;
        }
    }
    n
}

/// The voice-channel indices in a system that pass the filter.
fn passing_channels(sys: &System, profile: &dyn SdCardProfile, filter: &Filter) -> BTreeSet<usize> {
    let hit = name_hits(sys, filter);
    let mut out = BTreeSet::new();
    let mut ci = 0usize;
    for line in sys.lines() {
        if !is_voice_channel(line.command()) {
            continue;
        }
        let this = ci;
        ci += 1;
        if let Some(rec) = Record::new(line, profile) {
            if filter.channel_ok(rec.service_type_code(), rec.name().unwrap_or(""), hit) {
                out.insert(this);
            }
        }
    }
    out
}

/// Emit one catalog row (system-level, no channels). `counties` are the **covered**
/// counties (location-first placement) and `states` the `AreaState` ids;
/// `statewide` is set when the system has channels with no county (state-level
/// fallback). The UI builds the Country→State→County hierarchy from these.
#[allow(clippy::too_many_arguments)]
fn push_catalog_row(
    out: &mut String,
    di: usize,
    si: usize,
    sys: &System,
    states: &[u64],
    counties: &[u64],
    statewide: bool,
    channel_count: usize,
) {
    out.push_str("{\"id\":");
    push_json_string(out, &format!("d{di}s{si}"));
    out.push_str(",\"name\":");
    push_json_string(out, sys.name().unwrap_or(""));
    out.push_str(",\"kind\":");
    push_json_string(out, sys.header().command());
    out.push_str(",\"tech\":");
    match sys.tech() {
        Some(t) => push_json_string(out, t),
        None => out.push_str("null"),
    }
    out.push_str(",\"counties\":[");
    for (i, c) in counties.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&c.to_string());
    }
    out.push_str("],\"states\":[");
    for (i, s) in states.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&s.to_string());
    }
    out.push_str("],\"statewide\":");
    out.push_str(if statewide { "true" } else { "false" });
    out.push_str(",\"siteCount\":");
    out.push_str(
        &sys.lines()
            .iter()
            .filter(|l| l.command() == "Site")
            .count()
            .to_string(),
    );
    out.push_str(",\"channelCount\":");
    out.push_str(&channel_count.to_string());
    out.push('}');
}

/// Emit the filter-passing channels of one system (the lazy-load body). When
/// `allowed` is `Some`, only voice channels whose index is in the set are emitted
/// (county-scoped); `None` emits all passing channels.
fn push_system_channels(
    out: &mut String,
    di: usize,
    si: usize,
    sys: &System,
    profile: &dyn SdCardProfile,
    filter: &Filter,
    allowed: Option<&BTreeSet<usize>>,
) {
    let hit = name_hits(sys, filter);
    let mut ci = 0usize;
    let mut first = true;
    for line in sys.lines() {
        if !is_voice_channel(line.command()) {
            continue;
        }
        let Some(rec) = Record::new(line, profile) else {
            continue;
        };
        let freq = rec.frequency_hz();
        let tgid = rec.talkgroup();
        let this = ci;
        ci += 1;
        if allowed.is_some_and(|a| !a.contains(&this)) {
            continue;
        }
        if !filter.channel_ok(rec.service_type_code(), rec.name().unwrap_or(""), hit) {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str("{\"id\":");
        push_json_string(out, &format!("d{di}s{si}c{this}"));
        out.push_str(",\"name\":");
        push_json_string(out, rec.name().unwrap_or(""));
        out.push_str(",\"kind\":");
        out.push_str(if tgid.is_some() {
            "\"Talkgroup\""
        } else {
            "\"Frequency\""
        });
        out.push_str(",\"tgid\":");
        match tgid {
            Some(t) => push_json_string(out, t),
            None => out.push_str("null"),
        }
        out.push_str(",\"freqHz\":");
        match freq {
            Some(hz) => out.push_str(&hz.to_string()),
            None => out.push_str("null"),
        }
        out.push_str(",\"mode\":");
        match rec.mode() {
            Some(m) => push_json_string(out, m),
            None => out.push_str("null"),
        }
        out.push_str(",\"serviceType\":");
        match rec.service_type_code() {
            Some(code) => out.push_str(&code.to_string()),
            None => out.push_str("null"),
        }
        out.push_str(",\"tone\":");
        match rec.audio_option() {
            Some(t) => push_json_string(out, t),
            None => out.push_str("null"),
        }
        out.push('}');
    }
}

/// Parse a `"d<di>s<si>"` catalog row id back to indices.
fn parse_system_id(id: &str) -> Option<(usize, usize)> {
    let rest = id.strip_prefix('d')?;
    let (di, si) = rest.split_once('s')?;
    Some((di.parse().ok()?, si.parse().ok()?))
}

// ---------------------------------------------------------------------------
// FT-60 clone-image read (serial). The transport lives in `platypus-serial`
// (nix/termios); this bridges it to the C ABI with progress + cancel callbacks
// and returns a decoded-image handle whose channels cross as JSON. Read-only.
// ---------------------------------------------------------------------------

/// Opaque handle: a decoded FT-60 clone image.
pub struct PlatypusFt60 {
    image: platypus_core::device::ft60::Ft60Image,
}

/// A C cancel poll: returns nonzero to abort the transfer.
pub type PlatypusCancelFn = Option<extern "C" fn(*mut c_void) -> u8>;

struct FfiProgress {
    ctx: *mut c_void,
    progress: PlatypusProgressFn,
    cancel: PlatypusCancelFn,
}
impl platypus_serial::Progress for FfiProgress {
    fn update(&mut self, bytes: usize, total: usize) {
        if let Some(f) = self.progress {
            f(self.ctx, 1, bytes as u32, total as u32);
        }
    }
    fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|f| f(self.ctx) != 0)
    }
}

/// Candidate serial ports as a JSON array of device paths (`/dev/cu.*`).
#[no_mangle]
pub extern "C" fn platypus_serial_ports_json() -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || {
        let mut out = String::from("[");
        for (i, p) in platypus_serial::list_ports().iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            push_json_string(&mut out, p);
        }
        out.push(']');
        to_c_string(out)
    })
}

/// Read + decode an FT-60 clone image over serial `port`. Returns a handle, or null on
/// failure/cancel (with `*err_out` set to a heap message the caller frees). Non-destructive
/// — only the `0x06` clone-handshake ACK is written, never radio memory.
///
/// # Safety
/// `port` must be a valid C string; `err_out` a valid pointer or null; the callbacks + `ctx`
/// must remain valid for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_read(
    port: *const c_char,
    ctx: *mut c_void,
    progress: PlatypusProgressFn,
    cancel: PlatypusCancelFn,
    err_out: *mut *mut c_char,
) -> *mut PlatypusFt60 {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let set_err = |msg: String| {
            if !err_out.is_null() {
                *err_out = to_c_string(msg);
            }
        };
        let Some(port) = cstr_to_str(port) else {
            set_err("invalid port path".to_string());
            return ptr::null_mut();
        };
        let spec = ft60_spec();
        let mut sp = match platypus_serial::SerialPort::open(
            Path::new(port),
            spec.baud,
            std::time::Duration::from_millis(1000),
        ) {
            Ok(p) => p,
            Err(e) => {
                set_err(format!("couldn't open {port}: {e}"));
                return ptr::null_mut();
            }
        };
        let mut prog = FfiProgress {
            ctx,
            progress,
            cancel,
        };
        let bytes = match platypus_serial::read_ft60_image(
            &mut sp,
            &spec,
            platypus_serial::CloneTimeouts::default(),
            &mut prog,
        ) {
            Ok(b) => b,
            Err(e) => {
                set_err(e.to_string());
                return ptr::null_mut();
            }
        };
        match platypus_core::device::ft60::Ft60Image::decode(&bytes) {
            Ok(image) => Box::into_raw(Box::new(PlatypusFt60 { image })),
            Err(e) => {
                set_err(e.to_string());
                ptr::null_mut()
            }
        }
    })
}

/// Save a read handle's raw clone image as a `.img` **backup** — the restore point captured on
/// every radio Read. Writes `<dir>/<stem>.img`, creating `dir` and `fsync`ing the file so the
/// bytes are durably on disk (the same "an `Ok` isn't enough until it's flushed" discipline as the
/// card path). The persistence lives here in the core FFI, not the app, so every front-end backs
/// up identically. Returns the backup file path (caller frees with [`platypus_string_free`]), or
/// null + `*err_out` on failure.
///
/// # Safety
/// `handle` valid or null; `dir`/`stem` valid C strings; `err_out` valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_backup(
    handle: *const PlatypusFt60,
    dir: *const c_char,
    stem: *const c_char,
    err_out: *mut *mut c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        use std::io::Write;
        let set_err = |msg: String| {
            if !err_out.is_null() {
                *err_out = to_c_string(msg);
            }
        };
        let Some(h) = handle.as_ref() else {
            set_err("no image to back up".to_string());
            return ptr::null_mut();
        };
        let (Some(dir), Some(stem)) = (cstr_to_str(dir), cstr_to_str(stem)) else {
            set_err("invalid backup path".to_string());
            return ptr::null_mut();
        };
        let dir = Path::new(dir);
        if let Err(e) = std::fs::create_dir_all(dir) {
            set_err(format!("couldn't create backup folder: {e}"));
            return ptr::null_mut();
        }
        let path = dir.join(format!("{stem}.img"));
        let write = || -> std::io::Result<()> {
            let mut f = std::fs::File::create(&path)?;
            f.write_all(h.image.raw())?;
            f.sync_all()
        };
        match write() {
            Ok(()) => to_c_string(path.to_string_lossy().into_owned()),
            Err(e) => {
                set_err(format!("backup write failed: {e}"));
                ptr::null_mut()
            }
        }
    })
}

/// The decoded standard memories as a JSON array — the shape the FT-60 editor consumes.
/// Every enumerated field crosses as its on-radio **code** (symmetric with the write struct
/// `PlatypusFt60Channel`), so the app never string-matches an attribute: `[{ slot, name,
/// freqHz, modeCode, toneModeCode, toneMode:"off"|"ctcss"|"dcs"|"cross", toneValue, toneValue2,
/// duplexCode, offsetHz, txHz, power, step, skip:0|1|2, banks:[u8] }]`. The codes index the
/// option lists from [`platypus_ft60_options_json`]. `toneMode` is the value-kind for
/// reconstructing the squelch value; for `"cross"` (`Tone->DTCS`/`DTCS->Tone`) `toneValue` is the
/// CTCSS Hz×10 and `toneValue2` the DCS code. `skip` 0=none/1=S/2=P.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_memories_json(handle: *const PlatypusFt60) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        use platypus_core::device::ft60::{Tone, MODES, TMODES};
        let Some(h) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let mut out = String::from("[");
        for (i, ch) in h.image.channels().iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"slot\":");
            out.push_str(&ch.slot.to_string());
            out.push_str(",\"name\":");
            push_json_string(&mut out, &ch.name);
            out.push_str(",\"freqHz\":");
            out.push_str(&ch.rx_hz.to_string());
            // Mode as its MODES index (0=FM/1=NFM/2=AM).
            out.push_str(",\"modeCode\":");
            out.push_str(
                &MODES
                    .iter()
                    .position(|m| *m == ch.mode)
                    .unwrap_or(0)
                    .to_string(),
            );
            // `toneValue2` carries the DCS half of a cross mode (CTCSS in `toneValue`); 0 otherwise.
            let (tmode, tval, tval2) = match ch.tone {
                Tone::None => ("off", 0i64, 0i64),
                Tone::Ctcss(hz10) => ("ctcss", hz10 as i64, 0),
                Tone::Dcs(code) => ("dcs", code as i64, 0),
                Tone::Cross { ctcss, dcs } => ("cross", ctcss as i64, dcs as i64),
            };
            // Tone-mode sub-kind as its TMODES index, plus the value-kind + value(s) for the squelch.
            out.push_str(",\"toneModeCode\":");
            out.push_str(
                &TMODES
                    .iter()
                    .position(|m| *m == ch.tone_mode)
                    .unwrap_or(0)
                    .to_string(),
            );
            out.push_str(",\"toneMode\":");
            push_json_string(&mut out, tmode);
            out.push_str(",\"toneValue\":");
            out.push_str(&tval.to_string());
            out.push_str(",\"toneValue2\":");
            out.push_str(&tval2.to_string());
            // Duplex as the writer's code (0 simplex, 2 −, 3 +, 4 split); other states collapse to
            // simplex, matching the form's option set.
            let duplex_code = match ch.duplex {
                "-" => 2,
                "+" => 3,
                "split" => 4,
                _ => 0,
            };
            out.push_str(",\"duplexCode\":");
            out.push_str(&duplex_code.to_string());
            out.push_str(",\"offsetHz\":");
            out.push_str(&ch.offset_hz.to_string());
            out.push_str(",\"txHz\":");
            out.push_str(&ch.tx_hz.to_string());
            out.push_str(",\"power\":");
            out.push_str(&ch.power.to_string());
            out.push_str(",\"step\":");
            out.push_str(&ch.step.to_string());
            // skip as a code so the tri-state (""/S/P) survives the round-trip: 0 / 1 / 2.
            let skip_code = match ch.skip {
                "S" => 1,
                "P" => 2,
                _ => 0,
            };
            out.push_str(",\"skip\":");
            out.push_str(&skip_code.to_string());
            out.push_str(",\"banks\":[");
            for (j, b) in ch.banks.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                out.push_str(&b.to_string());
            }
            out.push_str("]}");
        }
        out.push(']');
        to_c_string(out)
    })
}

/// Free an FT-60 image handle.
///
/// # Safety
/// `handle` must be from [`platypus_ft60_read`] or null, and unused afterward.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_free(handle: *mut PlatypusFt60) {
    ffi_guard((), move || unsafe {
        if !handle.is_null() {
            drop(Box::from_raw(handle));
        }
    })
}

/// Borrow the raw image bytes of a read handle (valid until the handle is freed). Sets
/// `*out_len` and returns a pointer; the caller should copy them out before freeing. Null +
/// `*out_len = 0` on a null handle.
///
/// # Safety
/// `handle` must be valid or null; `out_len` a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_image_bytes(
    handle: *const PlatypusFt60,
    out_len: *mut usize,
) -> *const u8 {
    ffi_guard(ptr::null(), move || unsafe {
        let Some(h) = handle.as_ref() else {
            if !out_len.is_null() {
                *out_len = 0;
            }
            return ptr::null();
        };
        let raw = h.image.raw();
        if !out_len.is_null() {
            *out_len = raw.len();
        }
        raw.as_ptr()
    })
}

/// Write (clone-out) `bytes` to the FT-60 over serial `port`. The radio must be in CLONE
/// **receive** (`-WAIT-`). Returns 1 on success, 0 on failure/cancel with `*err_out` set to a
/// heap message (free with [`platypus_string_free`]). **WARNING: this writes radio memory** —
/// pass only an image that round-trips (the bytes from a read, or an edit past the gate).
///
/// # Safety
/// `port` a valid C string; `bytes` valid for `len`; `err_out` valid or null; the callbacks +
/// `ctx` must outlive the call.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_write(
    port: *const c_char,
    bytes: *const u8,
    len: usize,
    ctx: *mut c_void,
    progress: PlatypusProgressFn,
    cancel: PlatypusCancelFn,
    err_out: *mut *mut c_char,
) -> u8 {
    ffi_guard(0, move || unsafe {
        let set_err = |msg: String| {
            if !err_out.is_null() {
                *err_out = to_c_string(msg);
            }
        };
        let Some(port) = cstr_to_str(port) else {
            set_err("invalid port path".to_string());
            return 0;
        };
        if bytes.is_null() || len == 0 {
            set_err("no image to write".to_string());
            return 0;
        }
        let image = std::slice::from_raw_parts(bytes, len);
        let spec = ft60_spec();
        let mut sp = match platypus_serial::SerialPort::open(
            Path::new(port),
            spec.baud,
            std::time::Duration::from_millis(1000),
        ) {
            Ok(p) => p,
            Err(e) => {
                set_err(format!("couldn't open {port}: {e}"));
                return 0;
            }
        };
        let mut prog = FfiProgress {
            ctx,
            progress,
            cancel,
        };
        match platypus_serial::write_ft60_image(
            &mut sp,
            &spec,
            image,
            platypus_serial::CloneTimeouts::default(),
            &mut prog,
        ) {
            Ok(()) => 1,
            Err(e) => {
                set_err(e.to_string());
                0
            }
        }
    })
}

/// A channel to program, laid out for the C ABI (avoids JSON parsing in the no-serde FFI).
/// `mode`: 0=FM 1=NFM 2=AM. `tone_mode`: index into TMODES
/// `["","Tone","TSQL","TSQL-R","DTCS","DTCS->","Tone->DTCS","DTCS->Tone"]`; `tone_value` = CTCSS
/// Hz×10 (modes 1–3) or the DCS code (modes 4–7). `duplex`: index into
/// `["","","-","+","split","off"]` (0=simplex, 2=−, 3=+, 4=split). `offset_hz`: repeater-shift
/// magnitude; `tx_hz`: TX frequency for split. `power`: 0=High 1=Mid 2=Low. `step`: index into
/// STEPS. `skip`: 0=`""` 1=`S` 2=`P`. `banks`: bit *b* set → member of bank *b*. `name`: up to
/// 6 chars, NUL-padded.
#[repr(C)]
pub struct PlatypusFt60Channel {
    pub slot: u16,
    pub rx_hz: u64,
    pub offset_hz: u32,
    pub tx_hz: u32,
    pub mode: u8,
    pub tone_mode: u8,
    pub tone_value: u16,
    /// Second squelch value for the cross tone modes (`Tone->DTCS`/`DTCS->Tone`): the DCS code,
    /// while `tone_value` holds the CTCSS Hz×10. Ignored for single-value tone modes.
    pub tone_value2: u16,
    pub duplex: u8,
    pub power: u8,
    pub step: u8,
    pub skip: u8,
    pub banks: u16,
    pub name: [c_char; 8],
}

/// Apply an edited channel set onto `base` and return a new handle carrying the edits + a valid
/// checksum (a pure transform — no I/O). Read the result with [`platypus_ft60_image_bytes`],
/// then clone it out with [`platypus_ft60_write`]. Re-applying the channels a read produced is a
/// byte-for-byte no-op (the round-trip gate). Null on a bad base image (`*err_out` set).
///
/// # Safety
/// `base` must be valid for `base_len`; `chans` valid for `chans_len` (or null when 0);
/// `err_out` a valid pointer or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_apply(
    base: *const u8,
    base_len: usize,
    chans: *const PlatypusFt60Channel,
    chans_len: usize,
    err_out: *mut *mut c_char,
) -> *mut PlatypusFt60 {
    ffi_guard(ptr::null_mut(), move || unsafe {
        use platypus_core::device::ft60::{Ft60Channel, Ft60Image, Tone};
        let set_err = |msg: String| {
            if !err_out.is_null() {
                *err_out = to_c_string(msg);
            }
        };
        if base.is_null() || base_len == 0 {
            set_err("no base image".to_string());
            return ptr::null_mut();
        }
        let base = std::slice::from_raw_parts(base, base_len);
        let mut image = match Ft60Image::decode(base) {
            Ok(i) => i,
            Err(e) => {
                set_err(e.to_string());
                return ptr::null_mut();
            }
        };
        let list: &[PlatypusFt60Channel] = if chans.is_null() || chans_len == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(chans, chans_len)
        };
        let mut desired = Vec::with_capacity(list.len());
        for c in list {
            let name = {
                let bytes: Vec<u8> = c
                    .name
                    .iter()
                    .take_while(|&&b| b != 0)
                    .map(|&b| b as u8)
                    .collect();
                String::from_utf8_lossy(&bytes).into_owned()
            };
            let mode: &'static str = match c.mode {
                1 => "NFM",
                2 => "AM",
                _ => "FM",
            };
            // tone_mode is the TMODES index (0-7); 1–3 use the CTCSS value, 4–5 the DCS value, 6–7
            // (the cross modes) use both (CTCSS in tone_value, DCS in tone_value2).
            let tone_mode: &'static str = platypus_core::device::ft60::TMODES
                .get(c.tone_mode as usize)
                .copied()
                .unwrap_or("");
            let tone = match c.tone_mode {
                1..=3 => Tone::Ctcss(c.tone_value),
                4..=5 => Tone::Dcs(c.tone_value),
                6..=7 => Tone::Cross {
                    ctcss: c.tone_value,
                    dcs: c.tone_value2,
                },
                _ => Tone::None,
            };
            let skip: &'static str = match c.skip {
                1 => "S",
                2 => "P",
                _ => "",
            };
            let duplex: &'static str = match c.duplex {
                2 => "-",
                3 => "+",
                4 => "split",
                5 => "off",
                _ => "",
            };
            let banks: Vec<u8> = (0..10u8).filter(|b| c.banks & (1 << b) != 0).collect();
            desired.push(Ft60Channel {
                slot: c.slot,
                name,
                rx_hz: c.rx_hz,
                duplex,
                offset_hz: c.offset_hz as u64,
                tx_hz: c.tx_hz as u64,
                mode,
                tone_mode,
                tone,
                power: c.power,
                step: c.step,
                banks,
                skip,
            });
        }
        image.apply_channels(&desired);
        Box::into_raw(Box::new(PlatypusFt60 { image }))
    })
}

/// One PMS band-edge record for [`platypus_ft60_apply_pms`]. `index` is the 0-based record
/// (interleaved: `2p` = lower / `2p+1` = upper of pair `p`); `used` 0 clears the edge (frequency
/// ignored). `freq_hz` is the edge frequency; `step` the tuning-step index.
#[repr(C)]
pub struct PlatypusFt60PmsEdge {
    pub index: u16,
    pub used: u8,
    pub freq_hz: u64,
    pub step: u8,
}

/// Apply edited PMS band-edge records onto `base` and return a new handle carrying the edits + a
/// valid checksum (a pure transform — no I/O), parallel to [`platypus_ft60_apply`]. Only the
/// records you pass are touched (partial-apply); read the result with
/// [`platypus_ft60_image_bytes`] and clone it out with [`platypus_ft60_write`]. Re-applying the
/// edges a read produced is a byte-for-byte no-op (the round-trip gate). Null on a bad base image
/// (`*err_out` set).
///
/// # Safety
/// `base` must be valid for `base_len`; `edges` valid for `edges_len` (or null when 0);
/// `err_out` a valid pointer or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_apply_pms(
    base: *const u8,
    base_len: usize,
    edges: *const PlatypusFt60PmsEdge,
    edges_len: usize,
    err_out: *mut *mut c_char,
) -> *mut PlatypusFt60 {
    ffi_guard(ptr::null_mut(), move || unsafe {
        use platypus_core::device::ft60::{Ft60Image, PmsEdge};
        let set_err = |msg: String| {
            if !err_out.is_null() {
                *err_out = to_c_string(msg);
            }
        };
        if base.is_null() || base_len == 0 {
            set_err("no base image".to_string());
            return ptr::null_mut();
        }
        let base = std::slice::from_raw_parts(base, base_len);
        let mut image = match Ft60Image::decode(base) {
            Ok(i) => i,
            Err(e) => {
                set_err(e.to_string());
                return ptr::null_mut();
            }
        };
        let list: &[PlatypusFt60PmsEdge] = if edges.is_null() || edges_len == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(edges, edges_len)
        };
        let desired: Vec<PmsEdge> = list
            .iter()
            .map(|e| PmsEdge {
                index: e.index,
                used: e.used != 0,
                rx_hz: e.freq_hz,
                step: e.step,
            })
            .collect();
        image.apply_pms(&desired);
        Box::into_raw(Box::new(PlatypusFt60 { image }))
    })
}

/// The programmed PMS band-edge memories as a JSON array — the shape the FT-60 editor's scan-edge
/// section consumes: `[{ index, freqHz, step }]` (only `used` edges; index 0..100, interleaved
/// lower/upper pairs). Empty `[]` when none are set.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_pms_json(handle: *const PlatypusFt60) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(h) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let mut out = String::from("[");
        let mut first = true;
        for e in h.image.pms_edges().iter().filter(|e| e.used) {
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str("{\"index\":");
            out.push_str(&e.index.to_string());
            out.push_str(",\"freqHz\":");
            out.push_str(&e.rx_hz.to_string());
            out.push_str(",\"step\":");
            out.push_str(&e.step.to_string());
            out.push('}');
        }
        out.push(']');
        to_c_string(out)
    })
}

/// The FT-60 set-mode settings as a JSON array (spec order) — the shape the settings editor
/// consumes: `[{ key, label, value, options:[str] }]`. `value` indexes `options`. Null on a null
/// handle.
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_settings_json(handle: *const PlatypusFt60) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(h) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let specs = platypus_core::device::ft60::settings_specs();
        let values = h.image.settings();
        let mut out = String::from("[");
        for (i, (s, v)) in specs.iter().zip(values).enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"key\":");
            push_json_string(&mut out, s.key);
            out.push_str(",\"label\":");
            push_json_string(&mut out, s.label);
            out.push_str(",\"value\":");
            out.push_str(&v.to_string());
            out.push_str(",\"options\":[");
            for (j, o) in s.options.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                push_json_string(&mut out, o);
            }
            out.push_str("]}");
        }
        out.push(']');
        to_c_string(out)
    })
}

/// Apply edited set-mode settings onto `base` (values in `settings_specs` order) and return a new
/// handle carrying the edits + a valid checksum — parallel to [`platypus_ft60_apply`]. Read the
/// result with [`platypus_ft60_image_bytes`] and clone it out with [`platypus_ft60_write`].
/// Change-gated, so re-applying a read's settings is a byte-for-byte no-op. Null on a bad base
/// image (`*err_out` set).
///
/// # Safety
/// `base` valid for `base_len`; `values` valid for `values_len` (or null when 0); `err_out` valid
/// or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_ft60_apply_settings(
    base: *const u8,
    base_len: usize,
    values: *const u8,
    values_len: usize,
    err_out: *mut *mut c_char,
) -> *mut PlatypusFt60 {
    ffi_guard(ptr::null_mut(), move || unsafe {
        use platypus_core::device::ft60::Ft60Image;
        let set_err = |msg: String| {
            if !err_out.is_null() {
                *err_out = to_c_string(msg);
            }
        };
        if base.is_null() || base_len == 0 {
            set_err("no base image".to_string());
            return ptr::null_mut();
        }
        let base = std::slice::from_raw_parts(base, base_len);
        let mut image = match Ft60Image::decode(base) {
            Ok(i) => i,
            Err(e) => {
                set_err(e.to_string());
                return ptr::null_mut();
            }
        };
        let vals: &[u8] = if values.is_null() || values_len == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(values, values_len)
        };
        image.apply_settings(vals);
        Box::into_raw(Box::new(PlatypusFt60 { image }))
    })
}

/// Append a JSON-escaped string literal (including the surrounding quotes). The
/// data is validated ASCII, so we only need to escape `"`, `\`, and controls.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_f64(out: &mut String, v: f64) {
    if v.is_finite() {
        out.push_str(&v.to_string());
    } else {
        out.push_str("null");
    }
}

// ---- pointer helpers ----

/// Borrow a C string as `&str`, or `None` if null / not UTF-8.
unsafe fn cstr_to_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        CStr::from_ptr(p).to_str().ok()
    }
}

/// Move a Rust `String` into a heap C string for the caller to free.
fn to_c_string(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Run an FFI entry point's body under `catch_unwind`, degrading an unexpected panic to
/// `fallback` instead of unwinding across the C ABI (which would abort the whole Swift app).
/// Release builds use `panic = "unwind"` (`Cargo.toml`) so this actually recovers; the core is
/// still written not to panic on bad input, so this is defense-in-depth. The `fallback` must be a
/// value the caller reads as failure for that function — `null` for the JSON/handle getters, a
/// non-null error string for the "null = success" card writers, `0` for the write status.
fn ffi_guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn sample(rel: &str) -> CString {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../samples/synthetic")
            .join(rel);
        CString::new(p.to_str().unwrap()).unwrap()
    }

    #[test]
    fn ffi_guard_degrades_panic_to_fallback() {
        // A panic inside an entry-point body must be caught and turned into the fallback
        // (null / the given value), never unwind across the C ABI. Suppress the panic hook so
        // the expected panic doesn't spam the test log.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let null: *mut c_char = ffi_guard(ptr::null_mut(), || panic!("boom"));
        let zero: u8 = ffi_guard(0, || panic!("boom"));
        std::panic::set_hook(prev);
        assert!(null.is_null(), "panicking body yields the null fallback");
        assert_eq!(zero, 0, "panicking body yields the 0 fallback");
        // The happy path still returns the real value.
        let ok = ffi_guard(ptr::null_mut(), || to_c_string("ok".to_string()));
        assert!(!ok.is_null());
        unsafe { platypus_string_free(ok) };
    }

    #[test]
    fn open_and_list_systems() {
        unsafe {
            let h = platypus_open_hpdb(sample("s_000090.hpd").as_ptr());
            assert!(!h.is_null());
            let json_ptr = platypus_systems_json(h);
            assert!(!json_ptr.is_null());
            let json = CStr::from_ptr(json_ptr).to_str().unwrap();
            // 4 systems; names present; geo locations present.
            assert!(json.contains("\"kind\":\"Conventional\""));
            assert!(json.contains("\"kind\":\"Trunk\""));
            assert!(json.contains("\"lat\":45.5"));
            assert_eq!(json.matches("\"counties\":").count(), 4);
            platypus_string_free(json_ptr);
            platypus_close_hpdb(h);
        }
    }

    #[test]
    fn catalog_has_channel_detail() {
        unsafe {
            let h = platypus_open_hpdb(sample("s_000090.hpd").as_ptr());
            let json_ptr = platypus_catalog_json(h);
            let json = CStr::from_ptr(json_ptr).to_str().unwrap();
            // System-level detail.
            assert!(json.contains("\"id\":\"s0\""));
            assert!(json.contains("\"siteCount\":"));
            assert!(json.contains("\"tech\":"));
            // Channel-level detail with stable ids and decoded attributes.
            assert!(json.contains("\"id\":\"s0c0\""));
            assert!(
                json.contains("\"kind\":\"Talkgroup\"") || json.contains("\"kind\":\"Frequency\"")
            );
            assert!(json.contains("\"serviceType\":"));
            platypus_string_free(json_ptr);
            platypus_close_hpdb(h);
        }
    }

    fn synthetic_dir() -> CString {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/synthetic");
        CString::new(p.to_str().unwrap()).unwrap()
    }

    #[test]
    fn library_aggregate_load_filter_and_lazy_channels() {
        unsafe {
            let lib = platypus_library_open(synthetic_dir().as_ptr(), ptr::null_mut(), None);
            assert!(!lib.is_null());

            // stats reflect every s_*.hpd in the dir (synthetic has s_000090.hpd).
            let sj = platypus_library_stats_json(lib);
            let stats = CStr::from_ptr(sj).to_str().unwrap();
            assert!(stats.contains("\"files\":1"));
            assert!(stats.contains("\"systems\":4"));
            platypus_string_free(sj);

            let empty = CString::new("").unwrap();
            // Unfiltered catalog: 4 system rows with channel counts + stable ids.
            let cj =
                platypus_library_catalog_json(lib, empty.as_ptr(), empty.as_ptr(), empty.as_ptr());
            let cat = CStr::from_ptr(cj).to_str().unwrap();
            assert_eq!(cat.matches("\"id\":\"d0s").count(), 4);
            assert!(cat.contains("\"channelCount\":"));
            platypus_string_free(cj);

            // Lazy channels for the first system.
            let id = CString::new("d0s0").unwrap();
            let chj =
                platypus_library_channels_json(lib, id.as_ptr(), empty.as_ptr(), empty.as_ptr());
            let chans = CStr::from_ptr(chj).to_str().unwrap();
            assert!(chans.contains("\"id\":\"d0s0c0\""));
            platypus_string_free(chj);

            // Tech filter narrows to trunked P25 systems only.
            let p25 = CString::new("P25").unwrap();
            let fj =
                platypus_library_catalog_json(lib, empty.as_ptr(), p25.as_ptr(), empty.as_ptr());
            let filtered = CStr::from_ptr(fj).to_str().unwrap();
            assert!(filtered.matches("\"id\":").count() <= 4);
            platypus_string_free(fj);

            platypus_library_close(lib);
        }
    }

    #[test]
    fn favorites_from_channels_subsets_and_commits() {
        unsafe {
            let lib = platypus_library_open(synthetic_dir().as_ptr(), ptr::null_mut(), None);
            let empty = CString::new("").unwrap();
            // The first system's channels include the stable id d0s0c0.
            let sid = CString::new("d0s0").unwrap();
            let chj =
                platypus_library_channels_json(lib, sid.as_ptr(), empty.as_ptr(), empty.as_ptr());
            assert!(CStr::from_ptr(chj).to_str().unwrap().contains("d0s0c0"));
            platypus_string_free(chj);

            // Build favorites from that one channel; it should yield one system.
            let ids = CString::new("d0s0c0").unwrap();
            let fav = platypus_favorites_from_channels(lib, ids.as_ptr(), false, false);
            assert!(!fav.is_null());
            let sj = platypus_favorites_summary_json(fav);
            assert!(CStr::from_ptr(sj)
                .to_str()
                .unwrap()
                .contains("\"systems\":1"));
            platypus_string_free(sj);

            // Commit to a throwaway card.
            let base = std::env::temp_dir().join(format!("platypus-ch-{}", std::process::id()));
            std::fs::create_dir_all(base.join("BCDx36HP")).unwrap();
            let mount = CString::new(base.to_str().unwrap()).unwrap();
            let label = CString::new("Cart List").unwrap();
            let err = platypus_favorites_commit(fav, mount.as_ptr(), 8, label.as_ptr());
            assert!(err.is_null(), "commit should succeed");
            assert!(base.join("BCDx36HP/favorites_lists/f_000008.hpd").exists());
            std::fs::remove_dir_all(&base).ok();

            platypus_favorites_free(fav);
            platypus_library_close(lib);
        }
    }

    #[test]
    fn favorites_edit_open_remove_append_commit() {
        unsafe {
            let base = std::env::temp_dir().join(format!("platypus-edit-{}", std::process::id()));
            std::fs::create_dir_all(base.join("BCDx36HP")).unwrap();
            let mount = CString::new(base.to_str().unwrap()).unwrap();

            // Seed slot 3 with a favorites list built from the synthetic library.
            let lib = platypus_library_open(synthetic_dir().as_ptr(), ptr::null_mut(), None);
            let all = CString::new("").unwrap();
            let cat = platypus_library_catalog_json(lib, all.as_ptr(), all.as_ptr(), all.as_ptr());
            // grab a couple library channel ids
            let cat_s = CStr::from_ptr(cat).to_str().unwrap().to_string();
            platypus_string_free(cat);
            let some = platypus_library_channels_json(
                lib,
                CString::new("d0s0").unwrap().as_ptr(),
                all.as_ptr(),
                all.as_ptr(),
            );
            assert!(CStr::from_ptr(some).to_str().unwrap().contains("d0s0c0"));
            platypus_string_free(some);
            assert!(!cat_s.is_empty());

            let seed = platypus_favorites_from_channels(
                lib,
                CString::new("d0s0c0").unwrap().as_ptr(),
                false,
                false,
            );
            let label = CString::new("Edit Me").unwrap();
            assert!(platypus_favorites_commit(seed, mount.as_ptr(), 3, label.as_ptr()).is_null());
            platypus_favorites_free(seed);

            // Open it, list channels (favorites ids s<si>c<ci>).
            let fav = platypus_favorites_open(mount.as_ptr(), 3);
            assert!(!fav.is_null());
            let cj = platypus_favorites_channels_json(fav);
            assert!(CStr::from_ptr(cj)
                .to_str()
                .unwrap()
                .contains("\"id\":\"s0c0\""));
            platypus_string_free(cj);

            // Remove that channel -> the system drops (no channels left).
            let pruned = platypus_favorites_remove(fav, CString::new("s0c0").unwrap().as_ptr());
            let sj = platypus_favorites_summary_json(pruned);
            assert!(CStr::from_ptr(sj)
                .to_str()
                .unwrap()
                .contains("\"systems\":0"));
            platypus_string_free(sj);

            // Append a library channel back.
            let appended = platypus_favorites_append_from_library(
                pruned,
                lib,
                CString::new("d0s0c0").unwrap().as_ptr(),
                false,
            );
            let sj2 = platypus_favorites_summary_json(appended);
            assert!(CStr::from_ptr(sj2)
                .to_str()
                .unwrap()
                .contains("\"systems\":1"));
            platypus_string_free(sj2);

            // Save back (rename) and re-read.
            let rn = CString::new("Renamed").unwrap();
            assert!(platypus_favorites_commit(appended, mount.as_ptr(), 3, rn.as_ptr()).is_null());
            let card = platypus_card_favorites_json(mount.as_ptr());
            assert!(CStr::from_ptr(card)
                .to_str()
                .unwrap()
                .contains("\"name\":\"Renamed\""));
            platypus_string_free(card);

            platypus_favorites_free(fav);
            platypus_favorites_free(pruned);
            platypus_favorites_free(appended);
            platypus_library_close(lib);
            std::fs::remove_dir_all(&base).ok();
        }
    }

    #[test]
    fn detect_card_hpdb_dir() {
        unsafe {
            // Build a throwaway "volume root" with the SDS150 model-folder layout
            // (`BCDx36HP/HPDB/s_*.hpd`) from synthetic content.
            let base =
                std::env::temp_dir().join(format!("platypus-carddetect-{}", std::process::id()));
            let hpdb = base.join("BCDx36HP/HPDB");
            std::fs::create_dir_all(&hpdb).unwrap();
            let syn = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../samples/synthetic/s_000090.hpd");
            std::fs::copy(&syn, hpdb.join("s_000090.hpd")).unwrap();

            let root = CString::new(base.to_str().unwrap()).unwrap();
            let dir = platypus_card_hpdb_dir(root.as_ptr());
            assert!(!dir.is_null());
            let s = CStr::from_ptr(dir).to_str().unwrap();
            assert!(s.ends_with("BCDx36HP/HPDB"));
            platypus_string_free(dir);

            // A directory that isn't a card volume root returns null.
            let nope = CString::new(hpdb.to_str().unwrap()).unwrap();
            assert!(platypus_card_hpdb_dir(nope.as_ptr()).is_null());

            std::fs::remove_dir_all(&base).ok();
        }
    }

    #[test]
    fn card_favorites_read_and_delete() {
        unsafe {
            // Build a throwaway card with two favorites via the commit FFI.
            let base =
                std::env::temp_dir().join(format!("platypus-card-ffi-{}", std::process::id()));
            std::fs::create_dir_all(base.join("BCDx36HP")).unwrap();
            let src = platypus_open_hpdb(sample("s_000090.hpd").as_ptr());
            let sel = CString::new("all").unwrap();
            let fav = platypus_favorites_build(src, sel.as_ptr(), 0.0, 0.0, 0.0, false, false);
            let mount = CString::new(base.to_str().unwrap()).unwrap();
            let a = CString::new("Alpha").unwrap();
            let b = CString::new("Bravo").unwrap();
            assert!(platypus_favorites_commit(fav, mount.as_ptr(), 1, a.as_ptr()).is_null());
            assert!(platypus_favorites_commit(fav, mount.as_ptr(), 2, b.as_ptr()).is_null());

            // Read the lists + model + limits back.
            let j = platypus_card_favorites_json(mount.as_ptr());
            assert!(!j.is_null());
            let json = CStr::from_ptr(j).to_str().unwrap();
            assert!(json.contains("\"model\":\"SDS150\""));
            assert!(json.contains("\"maxFavorites\":256"));
            assert!(json.contains("\"quickKeys\":100"));
            assert!(json.contains("\"name\":\"Alpha\""));
            assert!(json.contains("\"name\":\"Bravo\""));
            platypus_string_free(j);

            // Delete slot 1; only Bravo remains.
            assert!(platypus_card_delete_slot(mount.as_ptr(), 1).is_null());
            let j2 = platypus_card_favorites_json(mount.as_ptr());
            let json2 = CStr::from_ptr(j2).to_str().unwrap();
            assert!(!json2.contains("Alpha"));
            assert!(json2.contains("Bravo"));
            platypus_string_free(j2);

            platypus_favorites_free(fav);
            platypus_close_hpdb(src);
            std::fs::remove_dir_all(&base).ok();
        }
    }

    #[test]
    fn library_null_safe() {
        unsafe {
            assert!(platypus_library_stats_json(ptr::null()).is_null());
            assert!(platypus_library_catalog_json(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null()
            )
            .is_null());
            platypus_library_close(ptr::null_mut()); // no-op
        }
    }

    #[test]
    fn county_filter_via_ffi() {
        unsafe {
            let h = platypus_open_hpdb(sample("s_000090.hpd").as_ptr());
            let json_ptr = platypus_systems_in_county_json(h, 9003); // Cedar: 1 system
            let json = CStr::from_ptr(json_ptr).to_str().unwrap();
            assert!(json.contains("\"name\":"));
            assert_eq!(json.matches("\"counties\":").count(), 1);
            platypus_string_free(json_ptr);
            platypus_close_hpdb(h);
        }
    }

    #[test]
    fn counties_listing() {
        unsafe {
            let json_ptr = platypus_counties_json(sample("hpdb.cfg").as_ptr());
            let json = CStr::from_ptr(json_ptr).to_str().unwrap();
            assert!(json.contains("\"name\":\"Alpha\""));
            assert!(json.contains("\"id\":9001"));
            platypus_string_free(json_ptr);
        }
    }

    #[test]
    fn favorites_pipeline_build_preview_commit() {
        unsafe {
            let src = platypus_open_hpdb(sample("s_000090.hpd").as_ptr());
            // Build: county 9001 (2 systems), DQKs off, no band plan.
            let selector = CString::new("county:9001").unwrap();
            let fav = platypus_favorites_build(src, selector.as_ptr(), 0.0, 0.0, 0.0, false, false);
            assert!(!fav.is_null());

            // Preview summary.
            let sj = platypus_favorites_summary_json(fav);
            let s = CStr::from_ptr(sj).to_str().unwrap();
            assert!(s.contains("\"systems\":2"));
            assert!(s.contains("\"dqks\":2"));
            assert!(s.contains("\"bandPlans\":0"));
            platypus_string_free(sj);

            // Commit to a throwaway card.
            let base = std::env::temp_dir().join(format!("platypus-ffi-{}", std::process::id()));
            std::fs::create_dir_all(base.join("BCDx36HP")).unwrap();
            let mount = CString::new(base.to_str().unwrap()).unwrap();
            let label = CString::new("Test List").unwrap();
            let err = platypus_favorites_commit(fav, mount.as_ptr(), 7, label.as_ptr());
            assert!(err.is_null(), "commit should succeed (null = ok)");
            assert!(base.join("BCDx36HP/favorites_lists/f_000007.hpd").exists());
            let flist =
                std::fs::read_to_string(base.join("BCDx36HP/favorites_lists/f_list.cfg")).unwrap();
            assert!(flist.contains("Test List"));
            std::fs::remove_dir_all(&base).ok();

            platypus_favorites_free(fav);
            platypus_close_hpdb(src);
        }
    }

    #[test]
    fn favorites_null_safe() {
        unsafe {
            let bad = CString::new("nope").unwrap();
            assert!(platypus_favorites_build(
                ptr::null(),
                bad.as_ptr(),
                0.0,
                0.0,
                0.0,
                false,
                false
            )
            .is_null());
            assert!(platypus_favorites_summary_json(ptr::null()).is_null());
            platypus_favorites_free(ptr::null_mut()); // no-op
        }
    }

    #[test]
    fn null_inputs_are_safe() {
        unsafe {
            assert!(platypus_systems_json(ptr::null()).is_null());
            assert!(platypus_open_hpdb(ptr::null()).is_null());
            platypus_close_hpdb(ptr::null_mut()); // no-op
            platypus_string_free(ptr::null_mut()); // no-op
        }
    }

    // ---- static JSON emitters (no handle / no card) ----

    #[test]
    fn static_json_emitters() {
        unsafe {
            // Service types: the RadioReference code→name table (Ham = 13).
            let sj = platypus_service_types_json();
            let s = CStr::from_ptr(sj).to_str().unwrap();
            assert!(s.starts_with('[') && s.ends_with(']'));
            assert!(s.contains("\"code\":13"));
            assert!(s.contains("\"name\":\"Ham\""));
            // Every entry is a {code,name} object; the table is non-trivially long.
            assert!(s.matches("\"code\":").count() >= 20);
            assert_eq!(
                s.matches("\"code\":").count(),
                s.matches("\"name\":").count()
            );
            platypus_string_free(sj);

            // Radios: both registered models, each with a class discriminator.
            let rj = platypus_radios_json();
            let r = CStr::from_ptr(rj).to_str().unwrap();
            assert!(r.contains("\"id\":\"sds150\""));
            assert!(r.contains("\"id\":\"ft60r\""));
            assert!(r.contains("\"class\":\"sdCard\""));
            assert!(r.contains("\"class\":\"cloneImage\""));
            // The clone-image radio carries a fixed capacity (channels/banks/nameLen).
            assert!(r.contains("\"channels\":"));
            assert!(r.contains("\"banks\":"));
            assert!(r.contains("\"nameLen\":"));
            platypus_string_free(rj);

            // FT-60 option sets: five keyed lists, each {label,code}; tone modes add valueKind.
            let oj = platypus_ft60_options_json();
            let o = CStr::from_ptr(oj).to_str().unwrap();
            for key in [
                "\"modes\":[",
                "\"toneModes\":[",
                "\"steps\":[",
                "\"powers\":[",
                "\"duplexes\":[",
            ] {
                assert!(o.contains(key), "missing option key {key}");
            }
            assert!(o.contains("\"label\":"));
            assert!(o.contains("\"code\":"));
            // valueKind is emitted for tone modes only, one of the three kinds.
            assert!(o.contains("\"valueKind\":"));
            assert!(
                o.contains("\"valueKind\":\"none\"")
                    || o.contains("\"valueKind\":\"ctcss\"")
                    || o.contains("\"valueKind\":\"dcs\"")
            );
            platypus_string_free(oj);
        }
    }

    // ---- FT-60 clone-image FFI surface (built from a valid in-memory image) ----

    /// A minimal *valid* FT-60 clone image: the full `image_size` length with the `AH017`
    /// model magic at the head. `Ft60Image::decode` checks only length ≥ `image_size` and the
    /// magic — it does **not** verify the trailing checksum — so a zero-filled buffer with the
    /// magic decodes to an image with no used channels.
    fn ft60_valid_image() -> Vec<u8> {
        let size = CloneSpec::FT60.image_size;
        let mut b = vec![0u8; size];
        b[..CloneSpec::FT60.magic.len()].copy_from_slice(CloneSpec::FT60.magic);
        // Trailing byte = low 8 bits of the sum of every preceding byte (the image checksum),
        // so this buffer already satisfies the round-trip (apply's recompute is a no-op).
        let n = b.len();
        let sum: u32 = b[..n - 1].iter().map(|&x| x as u32).sum();
        b[n - 1] = (sum & 0xFF) as u8;
        b
    }

    #[test]
    fn ft60_null_port_guard() {
        unsafe {
            // Null port hits the early `cstr_to_str` guard before any serial I/O — safe with
            // no hardware. read → null handle + error; write → 0 + error.
            let mut err: *mut c_char = ptr::null_mut();
            let h = platypus_ft60_read(ptr::null(), ptr::null_mut(), None, None, &mut err);
            assert!(h.is_null());
            assert!(!err.is_null());
            platypus_string_free(err);

            let img = ft60_valid_image();
            let mut werr: *mut c_char = ptr::null_mut();
            let rc = platypus_ft60_write(
                ptr::null(),
                img.as_ptr(),
                img.len(),
                ptr::null_mut(),
                None,
                None,
                &mut werr,
            );
            assert_eq!(rc, 0);
            assert!(!werr.is_null());
            platypus_string_free(werr);
        }
    }

    #[test]
    fn ft60_apply_and_json_surface() {
        unsafe {
            let img = ft60_valid_image();
            let mut err: *mut c_char = ptr::null_mut();
            // apply with no channels: builds a handle from the base image, no edits.
            let h = platypus_ft60_apply(img.as_ptr(), img.len(), ptr::null(), 0, &mut err);
            assert!(!h.is_null());
            assert!(err.is_null());

            // No programmed channels → empty memories array (valid JSON).
            let mj = platypus_ft60_memories_json(h);
            assert_eq!(CStr::from_ptr(mj).to_str().unwrap(), "[]");
            platypus_string_free(mj);

            // Raw image bytes: image_size long and byte-equal to the input (all-zero image
            // has a zero checksum, so apply's recompute is a no-op).
            let mut len = 0usize;
            let p = platypus_ft60_image_bytes(h, &mut len);
            assert_eq!(len, img.len());
            assert!(!p.is_null());
            let out = std::slice::from_raw_parts(p, len);
            assert_eq!(out, &img[..]);

            // No PMS band edges set → empty array.
            let pj = platypus_ft60_pms_json(h);
            assert_eq!(CStr::from_ptr(pj).to_str().unwrap(), "[]");
            platypus_string_free(pj);

            platypus_ft60_free(h);
        }
    }

    #[test]
    fn ft60_apply_pms_surface() {
        unsafe {
            let img = ft60_valid_image();
            let mut err: *mut c_char = ptr::null_mut();
            // Program pair 0: record 0 = lower 144.000, record 1 = upper 148.000 (interleaved).
            let edges = [
                PlatypusFt60PmsEdge {
                    index: 0,
                    used: 1,
                    freq_hz: 144_000_000,
                    step: 0,
                },
                PlatypusFt60PmsEdge {
                    index: 1,
                    used: 1,
                    freq_hz: 148_000_000,
                    step: 0,
                },
            ];
            let h = platypus_ft60_apply_pms(
                img.as_ptr(),
                img.len(),
                edges.as_ptr(),
                edges.len(),
                &mut err,
            );
            assert!(!h.is_null());
            assert!(err.is_null());

            // Both edges now surface in the PMS JSON.
            let pj = platypus_ft60_pms_json(h);
            let s = CStr::from_ptr(pj).to_str().unwrap().to_string();
            platypus_string_free(pj);
            assert!(s.contains("\"index\":0") && s.contains("144000000"), "{s}");
            assert!(s.contains("\"index\":1") && s.contains("148000000"), "{s}");

            // Re-applying the same edges onto the result is a byte-for-byte no-op (round-trip gate).
            let mut len = 0usize;
            let p = platypus_ft60_image_bytes(h, &mut len);
            let bytes1 = std::slice::from_raw_parts(p, len).to_vec();
            let h2 = platypus_ft60_apply_pms(
                bytes1.as_ptr(),
                bytes1.len(),
                edges.as_ptr(),
                edges.len(),
                &mut err,
            );
            let mut len2 = 0usize;
            let p2 = platypus_ft60_image_bytes(h2, &mut len2);
            assert_eq!(std::slice::from_raw_parts(p2, len2), &bytes1[..]);

            platypus_ft60_free(h);
            platypus_ft60_free(h2);
        }
    }

    #[test]
    fn ft60_apply_pms_empty_is_decode_noop() {
        // The Open-Backup validation path: apply with no edges decodes the image and returns it
        // byte-identical; a too-short/foreign image is rejected.
        unsafe {
            let img = ft60_valid_image();
            let mut err: *mut c_char = ptr::null_mut();
            let h = platypus_ft60_apply_pms(img.as_ptr(), img.len(), ptr::null(), 0, &mut err);
            assert!(!h.is_null());
            assert!(err.is_null());
            let mut len = 0usize;
            let p = platypus_ft60_image_bytes(h, &mut len);
            assert_eq!(
                std::slice::from_raw_parts(p, len),
                &img[..],
                "no-edit apply is identity"
            );
            platypus_ft60_free(h);

            let bad = [0u8; 10];
            let mut e2: *mut c_char = ptr::null_mut();
            assert!(
                platypus_ft60_apply_pms(bad.as_ptr(), bad.len(), ptr::null(), 0, &mut e2).is_null()
            );
            if !e2.is_null() {
                platypus_string_free(e2);
            }
        }
    }

    #[test]
    fn ft60_apply_settings_surface() {
        unsafe {
            let img = ft60_valid_image();
            let mut err: *mut c_char = ptr::null_mut();
            // Set lamp (spec index 7) = 2 ("Toggle"); the rest stay 0.
            let mut vals = [0u8; 10];
            vals[7] = 2;
            let h = platypus_ft60_apply_settings(
                img.as_ptr(),
                img.len(),
                vals.as_ptr(),
                vals.len(),
                &mut err,
            );
            assert!(!h.is_null());
            let sj = platypus_ft60_settings_json(h);
            let s = CStr::from_ptr(sj).to_str().unwrap().to_string();
            platypus_string_free(sj);
            assert!(
                s.contains("\"key\":\"lamp\"") && s.contains("\"Toggle\""),
                "{s}"
            );
            assert!(s.contains("\"value\":2"), "{s}");
            platypus_ft60_free(h);
        }
    }

    #[test]
    fn ft60_backup_writes_exact_image() {
        unsafe {
            let img = ft60_valid_image();
            let mut err: *mut c_char = ptr::null_mut();
            let h = platypus_ft60_apply(img.as_ptr(), img.len(), ptr::null(), 0, &mut err);
            assert!(!h.is_null());
            let dir =
                std::env::temp_dir().join(format!("platypus-ft60-backup-{}", std::process::id()));
            let cdir = CString::new(dir.to_str().unwrap()).unwrap();
            let stem = CString::new("FT-60R test").unwrap();
            let pathc = platypus_ft60_backup(h, cdir.as_ptr(), stem.as_ptr(), &mut err);
            assert!(!pathc.is_null(), "backup returns the written path");
            let path = CStr::from_ptr(pathc).to_str().unwrap().to_string();
            platypus_string_free(pathc);
            assert!(path.ends_with("FT-60R test.img"), "{path}");
            assert_eq!(
                std::fs::read(&path).unwrap(),
                img,
                "backup is the exact captured image"
            );
            std::fs::remove_dir_all(&dir).ok();
            platypus_ft60_free(h);
        }
    }

    #[test]
    fn ft60_handle_null_safe() {
        unsafe {
            assert!(platypus_ft60_memories_json(ptr::null()).is_null());
            assert!(platypus_ft60_pms_json(ptr::null()).is_null());
            let mut len = 999usize;
            assert!(platypus_ft60_image_bytes(ptr::null(), &mut len).is_null());
            assert_eq!(len, 0, "null handle clears out_len");
            platypus_ft60_free(ptr::null_mut()); // no-op

            // apply with a null / empty base → null handle + error string.
            let mut err: *mut c_char = ptr::null_mut();
            assert!(platypus_ft60_apply(ptr::null(), 0, ptr::null(), 0, &mut err).is_null());
            assert!(!err.is_null());
            platypus_string_free(err);

            // A too-short base image fails to decode → null + error.
            let short = [0u8; 10];
            let mut err2: *mut c_char = ptr::null_mut();
            assert!(
                platypus_ft60_apply(short.as_ptr(), short.len(), ptr::null(), 0, &mut err2)
                    .is_null()
            );
            assert!(!err2.is_null());
            platypus_string_free(err2);
        }
    }

    // ---- favorites mutation setters + channelValueOptions ----

    #[test]
    fn favorites_setters_round_trip() {
        unsafe {
            // Build an in-memory favorites handle from the synthetic HPDB (all systems).
            let src = platypus_open_hpdb(sample("s_000090.hpd").as_ptr());
            let sel = CString::new("all").unwrap();
            let fav = platypus_favorites_build(src, sel.as_ptr(), 0.0, 0.0, 0.0, false, false);
            assert!(!fav.is_null());

            // Confirm channel s0c0 exists in the built list.
            let tj = platypus_favorites_tree_json(fav);
            let tree = CStr::from_ptr(tj).to_str().unwrap();
            assert!(tree.contains("\"id\":\"s0c0\""));
            platypus_string_free(tj);

            // set_channel_value: set the Delay field on channel s0c0, read it back via the tree.
            let tgt = CString::new("s0c0").unwrap();
            let field = CString::new("delay").unwrap();
            let val = CString::new("5").unwrap();
            let edited = platypus_favorites_set_channel_value(
                fav,
                tgt.as_ptr(),
                field.as_ptr(),
                val.as_ptr(),
            );
            assert!(!edited.is_null());
            let ej = platypus_favorites_tree_json(edited);
            assert!(CStr::from_ptr(ej)
                .to_str()
                .unwrap()
                .contains("\"delay\":\"5\""));
            platypus_string_free(ej);

            // set_avoid on the channel → avoid flips true.
            let avoided = platypus_favorites_set_avoid(fav, tgt.as_ptr(), true);
            assert!(!avoided.is_null());
            let aj = platypus_favorites_tree_json(avoided);
            let atree = CStr::from_ptr(aj).to_str().unwrap();
            // The channel object carries avoid:true (systems default to avoid:false).
            assert!(atree.contains("\"id\":\"s0c0\""));
            assert!(atree.contains("\"avoid\":true"));
            platypus_string_free(aj);

            // set_priority on the channel → priority flips true.
            let prio = platypus_favorites_set_priority(fav, tgt.as_ptr(), true);
            assert!(!prio.is_null());
            let pj = platypus_favorites_tree_json(prio);
            assert!(CStr::from_ptr(pj)
                .to_str()
                .unwrap()
                .contains("\"priority\":true"));
            platypus_string_free(pj);

            // A non-channel target rejects priority (per-channel only) → null.
            let sys_tgt = CString::new("s0").unwrap();
            assert!(platypus_favorites_set_priority(fav, sys_tgt.as_ptr(), true).is_null());
            // set_channel_value on a null handle → null.
            assert!(platypus_favorites_set_channel_value(
                ptr::null(),
                tgt.as_ptr(),
                field.as_ptr(),
                val.as_ptr()
            )
            .is_null());

            platypus_favorites_free(fav);
            platypus_favorites_free(edited);
            platypus_favorites_free(avoided);
            platypus_favorites_free(prio);
            platypus_close_hpdb(src);
        }
    }

    #[test]
    fn card_favorites_json_has_channel_value_options() {
        unsafe {
            // Throwaway card with one committed favorites list.
            let base = std::env::temp_dir().join(format!("platypus-cvo-{}", std::process::id()));
            std::fs::create_dir_all(base.join("BCDx36HP")).unwrap();
            let src = platypus_open_hpdb(sample("s_000090.hpd").as_ptr());
            let sel = CString::new("all").unwrap();
            let fav = platypus_favorites_build(src, sel.as_ptr(), 0.0, 0.0, 0.0, false, false);
            let mount = CString::new(base.to_str().unwrap()).unwrap();
            let name = CString::new("Opts").unwrap();
            assert!(platypus_favorites_commit(fav, mount.as_ptr(), 5, name.as_ptr()).is_null());

            let j = platypus_card_favorites_json(mount.as_ptr());
            assert!(!j.is_null());
            let json = CStr::from_ptr(j).to_str().unwrap();
            // The enumerated per-channel value menus (branch previously unasserted).
            assert!(json.contains("\"channelValueOptions\":{"));
            assert!(json.contains("\"alertColor\":["));
            // The alertColor enumeration leads with "Off".
            assert!(json.contains("\"alertColor\":[\"Off\""));
            platypus_string_free(j);

            platypus_favorites_free(fav);
            platypus_close_hpdb(src);
            std::fs::remove_dir_all(&base).ok();
        }
    }

    // ---- null C-string safety for the stateless / card entry points ----

    #[test]
    fn c_string_entry_points_null_safe() {
        unsafe {
            assert!(platypus_counties_json(ptr::null()).is_null());
            assert!(platypus_states_json(ptr::null()).is_null());
            assert!(platypus_card_favorites_json(ptr::null()).is_null());
            assert!(platypus_card_hpdb_dir(ptr::null()).is_null());
            // delete_slot with a null path returns an error string (not null), no crash.
            let e = platypus_card_delete_slot(ptr::null(), 1);
            assert!(!e.is_null());
            platypus_string_free(e);
        }
    }
}
