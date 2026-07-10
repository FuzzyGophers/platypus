// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! RadioReference source — the FFI surface for the app's RR browse/preview feature.
//!
//! Wraps a [`platypus_rr::RrClient`] (native OS TLS via the crate's `http` feature) behind an opaque
//! handle. `open` does the **cheap** location→system-list fetch (`getZipcodeInfo` + `getCountyInfo`);
//! a system's channels are fetched **lazily** on first drill and cached. Results cross as the *same*
//! JSON shapes the HPDB library emits (`CatalogSystem`/`CatalogChannel`/`GeoSystem`) so the app
//! renders RR and card data through one code path. Credentials never leave this layer; nothing is
//! logged. Card-writing is deliberately not here (that is the gated `Dataset`→HPDB synthesis).

use std::collections::{BTreeSet, HashMap};
use std::ffi::{c_char, c_double, c_void};
use std::ptr;

use platypus_core::model::haversine_miles;
use platypus_core::provider::{Channel, ChannelKind, SystemKind, SystemRecord};
use platypus_core::rr::{Credentials, RrSystem};
use platypus_rr::parse::{self, CountyContents, TalkgroupCat, ZipInfo};
use platypus_rr::{epoch_secs, Options, RrClient};

/// A category with a range wider than this (or no geo) is treated as **systemwide/shared** (statewide
/// interop, mutual aid) rather than a local county-level category, for location-first ranking.
const LOCAL_MAX_RANGE_MI: f64 = 60.0;

use crate::{
    cstr_to_str, ffi_guard, push_f64, push_json_string, to_c_string, PlatypusCancelFn,
    PlatypusProgressFn,
};

/// An open RadioReference browse session: the client + the resolved location + the county system
/// list, plus a lazy cache of fully-fetched systems (keyed by the ref used as each row's JSON `id`:
/// `"t<sid>"` for a trunked system, `"c<scid>"` for a conventional subcategory).
pub struct PlatypusRrSource {
    client: RrClient,
    zip: ZipInfo,
    county: CountyContents,
    fetched: HashMap<String, SystemRecord>,
    /// A trunked system's talkgroup categories (`sref` → cats), fetched on first expand.
    categories: HashMap<String, Vec<TalkgroupCat>>,
    /// A category's talkgroups as channels (`"<sref>|<tgCid>"` → channels), fetched on first expand.
    cat_channels: HashMap<String, Vec<Channel>>,
    /// Each system's real map location (`sref` → nearest-in-range site lat/lon/range) — trunked
    /// sites are fetched once when the map first needs them; conventional use their subcat geo.
    site_geo: HashMap<String, (f64, f64, f64)>,
}

fn credentials(app_key: &str, username: &str, password: &str) -> Credentials {
    Credentials {
        app_key: app_key.to_string(),
        username: username.to_string(),
        password: password.to_string(),
    }
}

fn kind_str(kind: SystemKind) -> &'static str {
    match kind {
        SystemKind::Trunk => "Trunk",
        SystemKind::Conventional => "Conventional",
        SystemKind::Other => "Other",
    }
}

/// Validate credentials against RR (`getUserData`) — the "Add source" pre-flight. Returns
/// `{"username","subExpireDate"}` JSON on success, or sets `*err_out` to the RR message (e.g. a bad
/// login fault) and returns null. Bypasses the on-disk cache (the account response is per-user, and
/// the cache key is deliberately credential-free), so this always makes a live check.
///
/// # Safety
/// The C-string params must be valid NUL-terminated or null; `err_out` valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_validate(
    app_key: *const c_char,
    username: *const c_char,
    password: *const c_char,
    err_out: *mut *mut c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let set_err = |msg: String| {
            if !err_out.is_null() {
                *err_out = to_c_string(msg);
            }
        };
        let (Some(app_key), Some(username), Some(password)) = (
            cstr_to_str(app_key),
            cstr_to_str(username),
            cstr_to_str(password),
        ) else {
            set_err("missing credentials".into());
            return ptr::null_mut();
        };
        let opts = Options {
            refresh: true,
            ..Options::default()
        };
        let client = RrClient::with_options(credentials(app_key, username, password), opts);
        match client.account() {
            Ok(u) => {
                let mut out = String::from("{\"username\":");
                push_json_string(&mut out, &u.username);
                out.push_str(",\"subExpireDate\":");
                push_json_string(&mut out, &u.sub_expire_date);
                out.push('}');
                to_c_string(out)
            }
            Err(e) => {
                set_err(e.to_string());
                ptr::null_mut()
            }
        }
    })
}

/// Open a browse session for a location. `selector` is `"zip:<zipcode>"` for now (structured so
/// `county:`/`state:`/`metro:`/`prox:` can join later). Runs the cheap two-call location→system-list
/// fetch; per-system channels load lazily via [`platypus_rr_source_channels_json`]. `progress`
/// reports `(ctx, phase, done, total)` (phase 1 = resolving location, 2 = fetching the county list);
/// `cancel` returning nonzero aborts. On failure sets `*err_out` and returns null.
///
/// # Safety
/// C-string params valid NUL-terminated or null; `err_out` valid or null; `ctx` is passed back to
/// the callbacks untouched.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_open(
    app_key: *const c_char,
    username: *const c_char,
    password: *const c_char,
    selector: *const c_char,
    ctx: *mut c_void,
    progress: PlatypusProgressFn,
    cancel: PlatypusCancelFn,
    err_out: *mut *mut c_char,
) -> *mut PlatypusRrSource {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let set_err = |msg: String| {
            if !err_out.is_null() {
                *err_out = to_c_string(msg);
            }
        };
        let report = |phase: u32, done: u32, total: u32| {
            if let Some(p) = progress {
                p(ctx, phase, done, total);
            }
        };
        let cancelled = || cancel.is_some_and(|c| c(ctx) != 0);

        let (Some(app_key), Some(username), Some(password), Some(selector)) = (
            cstr_to_str(app_key),
            cstr_to_str(username),
            cstr_to_str(password),
            cstr_to_str(selector),
        ) else {
            set_err("missing credentials or selector".into());
            return ptr::null_mut();
        };

        let Some(zip_str) = selector.strip_prefix("zip:") else {
            set_err(format!("unsupported selector: {selector}"));
            return ptr::null_mut();
        };
        let Ok(zip) = zip_str.trim().parse::<u32>() else {
            set_err("invalid ZIP".into());
            return ptr::null_mut();
        };

        let client = RrClient::new(credentials(app_key, username, password));

        report(1, 0, 2);
        let zip_xml = match client.get_zipcode_info(zip) {
            Ok(x) => x,
            Err(e) => {
                set_err(e.to_string());
                return ptr::null_mut();
            }
        };
        let Some(zip_info) = parse::parse_zipcode_info(&zip_xml) else {
            set_err(format!("ZIP {zip} not found"));
            return ptr::null_mut();
        };
        if cancelled() {
            set_err("cancelled".into());
            return ptr::null_mut();
        }

        report(2, 1, 2);
        let county_xml = match client.get_county_info(zip_info.ctid as u32) {
            Ok(x) => x,
            Err(e) => {
                set_err(e.to_string());
                return ptr::null_mut();
            }
        };
        let county = parse::parse_county(&county_xml);
        report(2, 2, 2);

        Box::into_raw(Box::new(PlatypusRrSource {
            client,
            zip: zip_info,
            county,
            fetched: HashMap::new(),
            categories: HashMap::new(),
            cat_channels: HashMap::new(),
            site_geo: HashMap::new(),
        }))
    })
}

/// The county's systems as catalog rows (same shape as `platypus_library_catalog_json`). Names come
/// from the one `getCountyInfo` call; `tech`/counts fill in once a system is drilled (cached).
/// Only the `search` (name) filter applies here — service/tech filters need channel data, so they
/// apply in the channel view. Empty array on a null handle.
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_systems_json(
    handle: *const PlatypusRrSource,
    _services_csv: *const c_char,
    _techs_csv: *const c_char,
    search: *const c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(src) = handle.as_ref() else {
            return ptr::null_mut();
        };
        let needle = cstr_to_str(search).unwrap_or("").to_lowercase();
        let matches = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);

        let mut out = String::from("[");
        let mut first = true;
        for t in &src.county.trs {
            let sref = format!("t{}", t.sid);
            let name = if t.name.is_empty() {
                format!("System {}", t.sid)
            } else {
                t.name.clone()
            };
            if !matches(&name) {
                continue;
            }
            push_row(
                &mut out,
                &mut first,
                src,
                &sref,
                &name,
                &t.city,
                SystemKind::Trunk,
            );
        }
        for sc in src.county.subcats.iter().filter(|s| !s.trunked_ref) {
            let sref = format!("c{}", sc.scid);
            if !matches(&sc.name) {
                continue;
            }
            push_row(
                &mut out,
                &mut first,
                src,
                &sref,
                &sc.name,
                "",
                SystemKind::Conventional,
            );
        }
        out.push(']');
        to_c_string(out)
    })
}

/// One catalog row, enriched from the drill cache when present. `city` is the system's home city
/// (from the county list, empty for conventional groups) — a cheap subtitle before any drill.
fn push_row(
    out: &mut String,
    first: &mut bool,
    src: &PlatypusRrSource,
    sref: &str,
    name: &str,
    city: &str,
    kind: SystemKind,
) {
    let cached = src.fetched.get(sref);
    if !*first {
        out.push(',');
    }
    *first = false;
    out.push_str("{\"id\":");
    push_json_string(out, sref);
    out.push_str(",\"name\":");
    push_json_string(out, name);
    out.push_str(",\"city\":");
    push_json_string(out, city);
    out.push_str(",\"kind\":");
    push_json_string(out, kind_str(kind));
    out.push_str(",\"tech\":");
    match cached.and_then(|r| r.tech.as_deref()) {
        Some(t) => push_json_string(out, t),
        None => out.push_str("null"),
    }
    out.push_str(",\"counties\":[");
    out.push_str(&src.zip.ctid.to_string());
    out.push_str("],\"states\":[");
    out.push_str(&src.zip.stid.to_string());
    out.push_str("],\"statewide\":false,\"siteCount\":");
    out.push_str(&cached.map(|r| r.locations.len()).unwrap_or(0).to_string());
    out.push_str(",\"channelCount\":");
    out.push_str(&cached.map(|r| r.channels.len()).unwrap_or(0).to_string());
    out.push('}');
}

/// Channels of one system (`system_ref` = a row `id`), fetched lazily on first call and cached.
/// Same shape as `platypus_library_channels_json`. Applies the service-type + `search` filters.
/// Empty array on a null handle / bad ref; sets nothing on a fetch error (returns what mapped).
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_channels_json(
    handle: *mut PlatypusRrSource,
    system_ref: *const c_char,
    services_csv: *const c_char,
    search: *const c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(src) = handle.as_mut() else {
            return ptr::null_mut();
        };
        let Some(sref) = cstr_to_str(system_ref) else {
            return to_c_string("[]".into());
        };
        if !src.fetched.contains_key(sref) {
            if let Some(rec) = fetch_system(&src.client, sref, &src.county, &src.zip) {
                src.fetched.insert(sref.to_string(), rec);
            }
        }
        let Some(rec) = src.fetched.get(sref) else {
            return to_c_string("[]".into());
        };
        let services = parse_services(services_csv);
        let needle = cstr_to_str(search).unwrap_or("").to_lowercase();
        to_c_string(channels_json(&rec.channels, sref, &services, &needle))
    })
}

/// Emit a `[CatalogChannel]` JSON array from canonical channels, filtered by service-type + name.
/// `id_prefix` seeds each channel id (`"<prefix>c<i>"`). Shared by the system + category drills.
fn channels_json(
    channels: &[Channel],
    id_prefix: &str,
    services: &BTreeSet<u16>,
    needle: &str,
) -> String {
    let mut out = String::from("[");
    let mut first = true;
    for (ci, ch) in channels.iter().enumerate() {
        if !services.is_empty() && !ch.service_type.is_some_and(|s| services.contains(&s)) {
            continue;
        }
        if !needle.is_empty() && !ch.name.to_lowercase().contains(needle) {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str("{\"id\":");
        push_json_string(&mut out, &format!("{id_prefix}c{ci}"));
        out.push_str(",\"name\":");
        push_json_string(&mut out, &ch.name);
        out.push_str(",\"kind\":");
        out.push_str(match ch.kind {
            ChannelKind::Talkgroup => "\"Talkgroup\"",
            ChannelKind::Frequency => "\"Frequency\"",
        });
        out.push_str(",\"tgid\":");
        match ch.tgid.as_deref() {
            Some(t) => push_json_string(&mut out, t),
            None => out.push_str("null"),
        }
        out.push_str(",\"freqHz\":");
        match ch.freq_hz {
            Some(hz) => out.push_str(&hz.to_string()),
            None => out.push_str("null"),
        }
        out.push_str(",\"mode\":");
        match ch.mode.as_deref() {
            Some(m) => push_json_string(&mut out, m),
            None => out.push_str("null"),
        }
        out.push_str(",\"serviceType\":");
        match ch.service_type {
            Some(c) => out.push_str(&c.to_string()),
            None => out.push_str("null"),
        }
        out.push_str(",\"tone\":");
        match ch.tone.as_deref() {
            Some(t) => push_json_string(&mut out, t),
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push(']');
    out
}

/// A trunked system's talkgroup **categories**, ranked nearest-first for a location-first drill.
/// `all == 0` returns only in-range + systemwide categories (the local view); `all != 0` includes
/// the out-of-area ones ("show all areas"). Shape:
/// `[{ "id":tgCid,"name","distanceMi":f64|null,"local":bool,"systemwide":bool }]`. Empty array for a
/// non-trunked ref or a null handle.
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_categories_json(
    handle: *mut PlatypusRrSource,
    system_ref: *const c_char,
    all: u8,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(src) = handle.as_mut() else {
            return ptr::null_mut();
        };
        let Some(sref) = cstr_to_str(system_ref) else {
            return to_c_string("[]".into());
        };
        let Some(sid) = sref.strip_prefix('t').and_then(|s| s.parse::<u32>().ok()) else {
            return to_c_string("[]".into());
        };
        if !src.categories.contains_key(sref) {
            let Ok(xml) = src.client.get_trs_talkgroup_cats(sid) else {
                return to_c_string("[]".into());
            };
            src.categories
                .insert(sref.to_string(), parse::parse_talkgroup_cats(&xml));
        }
        let (lat, lon) = (src.zip.lat, src.zip.lon);

        // Classify + rank each category: local (tight in-range) → shared (broad/systemwide in-range)
        // → far (out of range). Sort by (rank, distance); drop far unless `all`.
        let mut ranked: Vec<(&TalkgroupCat, f64, u8)> = src.categories[sref]
            .iter()
            .map(|c| {
                let systemwide_geo = c.lat == 0.0 && c.lon == 0.0;
                let dist = if systemwide_geo {
                    f64::INFINITY
                } else {
                    haversine_miles(lat, lon, c.lat, c.lon)
                };
                let covers = systemwide_geo || dist <= c.range;
                let broad = systemwide_geo || c.range > LOCAL_MAX_RANGE_MI;
                let rank = if !covers {
                    2
                } else if broad {
                    1
                } else {
                    0
                };
                (c, dist, rank)
            })
            .filter(|(_, _, rank)| all != 0 || *rank < 2)
            .collect();
        ranked.sort_by(|a, b| {
            a.2.cmp(&b.2)
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });

        let mut out = String::from("[");
        for (i, (c, dist, rank)) in ranked.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"id\":");
            out.push_str(&c.tg_cid.to_string());
            out.push_str(",\"name\":");
            push_json_string(&mut out, &c.name);
            out.push_str(",\"distanceMi\":");
            if dist.is_finite() {
                push_f64(&mut out, *dist);
            } else {
                out.push_str("null");
            }
            out.push_str(",\"local\":");
            out.push_str(if *rank == 0 { "true" } else { "false" });
            out.push_str(",\"systemwide\":");
            out.push_str(if *rank == 1 { "true" } else { "false" });
            out.push('}');
        }
        out.push(']');
        to_c_string(out)
    })
}

/// A talkgroup category's channels — `getTrsTalkgroups(sid, tgCid)` mapped to channels, fetched
/// lazily and cached. Same shape as [`platypus_rr_source_channels_json`]. Empty array for a
/// non-trunked ref / null handle.
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_category_channels_json(
    handle: *mut PlatypusRrSource,
    system_ref: *const c_char,
    tg_cid: u32,
    services_csv: *const c_char,
    search: *const c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(src) = handle.as_mut() else {
            return ptr::null_mut();
        };
        let Some(sref) = cstr_to_str(system_ref) else {
            return to_c_string("[]".into());
        };
        let Some(sid) = sref.strip_prefix('t').and_then(|s| s.parse::<u32>().ok()) else {
            return to_c_string("[]".into());
        };
        let key = format!("{sref}|{tg_cid}");
        if !src.cat_channels.contains_key(&key) {
            // Upstream freshness bound: if we know this category's `lastUpdated` (from the cats fetch),
            // its cached talkgroups re-download only when RR changed them. `Copy`, so computed before
            // the mutable insert borrow.
            let upstream = src
                .categories
                .get(sref)
                .and_then(|cats| cats.iter().find(|c| c.tg_cid as u32 == tg_cid))
                .map(|c| c.last_updated)
                .filter(|&t| t > 0);
            let Ok(xml) = src
                .client
                .get_trs_talkgroups_in_cat_fresh(sid, tg_cid, upstream)
            else {
                return to_c_string("[]".into());
            };
            let chans: Vec<Channel> = parse::parse_talkgroups(&xml)
                .iter()
                .map(Channel::from)
                .collect();
            src.cat_channels.insert(key.clone(), chans);
        }
        let services = parse_services(services_csv);
        let needle = cstr_to_str(search).unwrap_or("").to_lowercase();
        let id_prefix = format!("{sref}k{tg_cid}");
        to_c_string(channels_json(
            &src.cat_channels[&key],
            &id_prefix,
            &services,
            &needle,
        ))
    })
}

/// The county's systems as map pins (same shape as `platypus_library_geo_json`). A drilled system
/// uses its nearest site/subcat location; an un-drilled one falls back to the county centroid, so
/// the map is populated immediately and refines as systems are opened. `lat`/`lon`/`miles` are
/// accepted for signature parity with the library but not used to prune (the set is already the one
/// county). Empty array on a null handle.
///
/// # Safety
/// `handle` valid or null; the C-string params valid NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_geo_json(
    handle: *mut PlatypusRrSource,
    _lat: c_double,
    _lon: c_double,
    _miles: c_double,
    _services_csv: *const c_char,
    _techs_csv: *const c_char,
    search: *const c_char,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(src) = handle.as_mut() else {
            return ptr::null_mut();
        };
        // Instant: place conventional systems (cheap, no network) and emit every pin now — trunked
        // systems sit at the county centroid until their real site is warmed in the background (see
        // `platypus_rr_source_warm_batch`), so a big county paints immediately instead of blocking.
        ensure_conventional_geo(src);
        let needle = cstr_to_str(search).unwrap_or("").to_lowercase();
        let matches = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);

        // Emit only systems whose real site is **placed** (`site_geo`) — conventional are placed
        // cheaply by `ensure_conventional_geo` above, trunked once warmed. So pins enter the map at
        // their true locations instead of piling at the county centroid and teleporting out.
        let mut out = String::from("[");
        let mut first = true;
        for t in &src.county.trs {
            let sref = format!("t{}", t.sid);
            if !src.site_geo.contains_key(&sref) {
                continue;
            }
            let name = if t.name.is_empty() {
                format!("System {}", t.sid)
            } else {
                t.name.clone()
            };
            if matches(&name) {
                push_geo(&mut out, &mut first, src, &sref, &name, SystemKind::Trunk);
            }
        }
        for sc in src.county.subcats.iter().filter(|s| !s.trunked_ref) {
            let sref = format!("c{}", sc.scid);
            if src.site_geo.contains_key(&sref) && matches(&sc.name) {
                push_geo(
                    &mut out,
                    &mut first,
                    src,
                    &sref,
                    &sc.name,
                    SystemKind::Conventional,
                );
            }
        }
        out.push(']');
        to_c_string(out)
    })
}

/// Place conventional systems from their **subcategory geo** — cheap (no network), so the map can
/// paint immediately. A subcat with no geo falls back to the county centroid.
fn ensure_conventional_geo(src: &mut PlatypusRrSource) {
    let (zlat, zlon) = (src.zip.lat, src.zip.lon);
    let conventional: Vec<(String, f64, f64, f64)> = src
        .county
        .subcats
        .iter()
        .filter(|s| !s.trunked_ref)
        .map(|sc| (format!("c{}", sc.scid), sc.lat, sc.lon, sc.range))
        .collect();
    for (sref, lat, lon, range) in conventional {
        if src.site_geo.contains_key(&sref) {
            continue;
        }
        let geo = if lat != 0.0 || lon != 0.0 {
            (lat, lon, range)
        } else {
            (zlat, zlon, 0.0)
        };
        src.site_geo.insert(sref, geo);
    }
}

/// Fetch the **real site** for the next un-warmed trunked system (one `getTrsSites` call — nearest
/// in-range site, then the system's registered geo, then the county centroid). Returns `true` if one
/// was warmed, `false` when every trunked system already has a site. This is the *incremental* unit
/// the app drives in the background so pins spread progressively (one throttled call at a time)
/// instead of blocking on the whole county up front.
fn warm_next_trunked(src: &mut PlatypusRrSource) -> bool {
    let (zlat, zlon) = (src.zip.lat, src.zip.lon);
    let next = src
        .county
        .trs
        .iter()
        .map(|t| (format!("t{}", t.sid), t.sid as u32))
        .find(|(sref, _)| !src.site_geo.contains_key(sref));
    match next {
        Some((sref, sid)) => {
            let geo = trunked_site_geo(&src.client, sid, zlat, zlon);
            src.site_geo.insert(sref, geo);
            true
        }
        None => false,
    }
}

/// Populate `site_geo` for every system (cheap conventional placement + the full trunked site fetch).
/// The blocking, all-at-once warm — used by the manual Refresh; the map warms incrementally instead
/// via [`platypus_rr_source_warm_batch`].
fn ensure_site_geo(src: &mut PlatypusRrSource) {
    ensure_conventional_geo(src);
    while warm_next_trunked(src) {}
}

/// A trunked system's map location: the site nearest the point (with geo), else its registered
/// `getTrsDetails` geo, else the county centroid `(zlat, zlon)`.
fn trunked_site_geo(client: &RrClient, sid: u32, zlat: f64, zlon: f64) -> (f64, f64, f64) {
    if let Ok(xml) = client.get_trs_sites(sid) {
        let nearest = parse::parse_trs_sites(&xml)
            .into_iter()
            .filter(|s| s.lat != 0.0 || s.lon != 0.0)
            .min_by(|a, b| {
                haversine_miles(zlat, zlon, a.lat, a.lon)
                    .partial_cmp(&haversine_miles(zlat, zlon, b.lat, b.lon))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some(s) = nearest {
            return (s.lat, s.lon, s.range);
        }
    }
    if let Ok(xml) = client.get_trs_details(sid) {
        if let Some(t) = parse::parse_trs_details(&xml) {
            if t.lat != 0.0 || t.lon != 0.0 {
                return (t.lat, t.lon, t.range);
            }
        }
    }
    (zlat, zlon, 0.0)
}

/// One geo pin — the system's real location from `site_geo` (falling back to the county centroid).
fn push_geo(
    out: &mut String,
    first: &mut bool,
    src: &PlatypusRrSource,
    sref: &str,
    name: &str,
    kind: SystemKind,
) {
    let (lat, lon, range) =
        src.site_geo
            .get(sref)
            .copied()
            .unwrap_or((src.zip.lat, src.zip.lon, 0.0));
    let cached = src.fetched.get(sref);
    let service = cached.and_then(|r| r.service_types.iter().next().copied());

    if !*first {
        out.push(',');
    }
    *first = false;
    out.push_str("{\"id\":");
    push_json_string(out, sref);
    out.push_str(",\"name\":");
    push_json_string(out, name);
    out.push_str(",\"kind\":");
    push_json_string(out, kind_str(kind));
    out.push_str(",\"tech\":");
    match cached.and_then(|r| r.tech.as_deref()) {
        Some(t) => push_json_string(out, t),
        None => out.push_str("null"),
    }
    out.push_str(",\"serviceType\":");
    match service {
        Some(c) => out.push_str(&c.to_string()),
        None => out.push_str("null"),
    }
    out.push_str(",\"lat\":");
    push_f64(out, lat);
    out.push_str(",\"lon\":");
    push_f64(out, lon);
    out.push_str(",\"rangeMi\":");
    push_f64(out, range);
    out.push('}');
}

/// Fetch a single **rich** system by ref (`t<sid>` trunked, `c<scid>` conventional) — the full
/// `RrSystem` (sites + control/voice freqs preserved), for synthesis. Networked. `None` on a bad
/// ref or an empty/failed fetch.
fn fetch_rr_system(
    client: &RrClient,
    sref: &str,
    county: &CountyContents,
    zip: &ZipInfo,
) -> Option<RrSystem> {
    if let Some(sid) = sref.strip_prefix('t').and_then(|s| s.parse::<u32>().ok()) {
        Some(RrSystem::Trunked(client.fetch_trunked(sid).ok().flatten()?))
    } else if let Some(scid) = sref.strip_prefix('c').and_then(|s| s.parse::<u32>().ok()) {
        let name = county
            .subcats
            .iter()
            .find(|s| s.scid == scid as u64)
            .map(|s| s.name.as_str())
            .unwrap_or("Conventional");
        Some(RrSystem::Conventional(
            client
                .fetch_conventional(scid, name, vec![zip.ctid])
                .ok()
                .flatten()?,
        ))
    } else {
        None
    }
}

/// Fetch + map a single system by ref to the flat browse `SystemRecord`. `None` on a bad ref or
/// an empty/failed fetch.
fn fetch_system(
    client: &RrClient,
    sref: &str,
    county: &CountyContents,
    zip: &ZipInfo,
) -> Option<SystemRecord> {
    fetch_rr_system(client, sref, county, zip).map(|s| SystemRecord::from(&s))
}

/// Fetch the rich `RrSystem` for a browsed system ref, for favorites synthesis (used by the FFI
/// that programs RadioReference onto an SD-card scanner). `pub(crate)` so `lib.rs` can reach it.
pub(crate) fn rr_system_for(src: &PlatypusRrSource, sref: &str) -> Option<RrSystem> {
    fetch_rr_system(&src.client, sref, &src.county, &src.zip)
}

/// The resolved location + system count for the browse header (the location chip).
/// Shape: `{ "city","ctid":u64,"stid":u64,"lat":f64,"lon":f64,"systemCount":u64 }`. Null on a null
/// handle.
///
/// # Safety
/// `handle` valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_location_json(
    handle: *const PlatypusRrSource,
) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(src) = handle.as_ref() else {
            return ptr::null_mut();
        };
        to_c_string(location_json_string(src))
    })
}

/// Build the location chip's JSON (`city/ctid/stid/lat/lon/systemCount/fetchedAt`). `fetchedAt` is
/// the `getCountyInfo` cache mtime (epoch secs, 0 if unknown) — the "as of <date>" anchor.
fn location_json_string(src: &PlatypusRrSource) -> String {
    let system_count =
        src.county.trs.len() + src.county.subcats.iter().filter(|s| !s.trunked_ref).count();
    let fetched_at = src
        .client
        .county_info_fetched_at(src.zip.ctid as u32)
        .map(epoch_secs)
        .unwrap_or(0);
    let mut out = String::from("{\"city\":");
    push_json_string(&mut out, &src.zip.city);
    out.push_str(",\"ctid\":");
    out.push_str(&src.zip.ctid.to_string());
    out.push_str(",\"stid\":");
    out.push_str(&src.zip.stid.to_string());
    out.push_str(",\"lat\":");
    push_f64(&mut out, src.zip.lat);
    out.push_str(",\"lon\":");
    push_f64(&mut out, src.zip.lon);
    out.push_str(",\"systemCount\":");
    out.push_str(&system_count.to_string());
    out.push_str(",\"fetchedAt\":");
    out.push_str(&fetched_at.to_string());
    out.push('}');
    out
}

/// Warm trunked systems' real map sites, returning how many were warmed (0 when all are done). Every
/// **already-cached** site is warmed at once (disk hits — instant, so a switch back to a visited county
/// loads immediately); only **live** fetches are paced — at most `batch`, in parallel on forked clients
/// (so the per-client throttle doesn't serialize them). The app calls this in a loop, reloading the map
/// after each call, so first-visit pins refine in bursts without blocking.
///
/// # Safety
/// `handle` valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_warm_batch(
    handle: *mut PlatypusRrSource,
    batch: u32,
) -> u32 {
    ffi_guard(0, move || unsafe {
        let Some(src) = handle.as_mut() else {
            return 0;
        };
        ensure_conventional_geo(src); // cheap, idempotent — conventional pins placed without network
        let (zlat, zlon) = (src.zip.lat, src.zip.lon);
        let pending: Vec<(String, u32)> = src
            .county
            .trs
            .iter()
            .map(|t| (format!("t{}", t.sid), t.sid as u32))
            .filter(|(sref, _)| !src.site_geo.contains_key(sref))
            .collect();
        if pending.is_empty() {
            return 0;
        }
        // Split by whether the site is already cached: cached ones hit disk (instant), so warm **all**
        // of them at once (no pacing) — this is what makes a switch back to a visited county load
        // immediately. Only the **live** fetches are paced: at most `batch`, in parallel via forked
        // clients (fresh throttle, shared on-disk cache).
        let (cached, live): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|(_, sid)| src.client.trs_sites_fresh(*sid));

        let mut warmed = 0u32;
        for (sref, sid) in cached {
            let geo = trunked_site_geo(&src.client, sid, zlat, zlon); // cache hit → instant
            src.site_geo.insert(sref, geo);
            warmed += 1;
        }

        let live_batch: Vec<(String, u32)> = live.into_iter().take(batch.max(1) as usize).collect();
        if !live_batch.is_empty() {
            let client = &src.client;
            let results: Vec<(String, (f64, f64, f64))> = std::thread::scope(|scope| {
                let handles: Vec<_> = live_batch
                    .into_iter()
                    .map(|(sref, sid)| {
                        scope.spawn(move || {
                            (sref, trunked_site_geo(&client.fork(), sid, zlat, zlon))
                        })
                    })
                    .collect();
                handles.into_iter().filter_map(|h| h.join().ok()).collect()
            });
            for (sref, geo) in results {
                src.site_geo.insert(sref, geo);
                warmed += 1;
            }
        }
        warmed
    })
}

/// Manual "Refresh": force-refetch the location's base data live (bypassing the cache), so a
/// stale-cached county/site set updates on demand. Clears the in-memory memos (drilled talkgroups
/// then refresh per TTL/upstream on next open), re-fetches `getCountyInfo` + the map site geo under
/// the force flag, then clears it. Returns the fresh location JSON (with the new `fetchedAt`), or
/// null on a null handle.
///
/// # Safety
/// `handle` valid or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_refresh(handle: *mut PlatypusRrSource) -> *mut c_char {
    ffi_guard(ptr::null_mut(), move || unsafe {
        let Some(src) = handle.as_mut() else {
            return ptr::null_mut();
        };
        src.client.set_force_refresh(true);
        src.fetched.clear();
        src.categories.clear();
        src.cat_channels.clear();
        src.site_geo.clear();
        if let Ok(xml) = src.client.get_county_info(src.zip.ctid as u32) {
            src.county = parse::parse_county(&xml);
        }
        ensure_site_geo(src);
        src.client.set_force_refresh(false);
        to_c_string(location_json_string(src))
    })
}

/// Parse a `services_csv` (`"3,8"`) into a set of service-type codes.
unsafe fn parse_services(services_csv: *const c_char) -> BTreeSet<u16> {
    cstr_to_str(services_csv)
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse::<u16>().ok())
        .collect()
}

/// Free a source handle (safe with null).
///
/// # Safety
/// `handle` must be a pointer from [`platypus_rr_source_open`] not already freed, or null.
#[no_mangle]
pub unsafe extern "C" fn platypus_rr_source_free(handle: *mut PlatypusRrSource) {
    ffi_guard((), move || unsafe {
        if !handle.is_null() {
            drop(Box::from_raw(handle));
        }
    })
}
