// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! RadioReference (RR) SOAP client — the **network glue** behind `platypus_core::rr`.
//!
//! The core owns the RR schema + the pure, tested mapping to the canonical
//! [`Dataset`](platypus_core::provider::Dataset); this crate does only the I/O: build the SOAP
//! `authInfo` envelope, POST it, and hand the raw response back (the typed parse lands in a later
//! step). The service is an **rpc-style SOAP** endpoint (WSDL:
//! `https://api.radioreference.com/soap2/?wsdl`), namespace `http://api.radioreference.com/soap2`.
//!
//! **API-friendly by construction:** every response is cached on disk (the file keyed by the *query*,
//! under a per-account directory), so repeated runs hit RR at most once per unique request; live
//! requests are throttled and carry a descriptive User-Agent. The cache uses the OS's conventional
//! cache location. See [`Options`].
//!
//! **Transport is the caller's.** The portable heart of this crate is [`request`] (build a SOAP
//! request) + [`parse`] (parse the response) — no networking. Shipped GUI front-ends POST the request
//! with their **native HTTP** (`URLSession` on macOS, `HttpClient` on .NET/Windows). For a CLI or
//! headless use, [`RrClient`] can do the round-trip itself over **native OS TLS** — but only with the
//! optional `http` feature (`ureq` + `native-tls`: Secure Transport / SChannel / system OpenSSL);
//! without it the crate is sha2-only and live requests error, leaving just `request`/`parse` + the
//! on-disk cache.

pub mod parse;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use platypus_core::rr::{Credentials, RrConventionalSystem, RrSystem, RrTrunkedSystem, Subcat};
use sha2::{Digest, Sha256};

/// The SOAP endpoint (HTTPS; the WSDL advertises `http`, but RR serves TLS).
pub const ENDPOINT: &str = "https://api.radioreference.com/soap2/index.php";
/// The service namespace (WSDL `targetNamespace`).
pub const NAMESPACE: &str = "http://api.radioreference.com/soap2";
/// A descriptive User-Agent (the settled string shared with RR).
pub const USER_AGENT: &str = "Platypus/0.2.0 (+https://github.com/FuzzyGophers/platypus)";
/// `authInfo.version` — the service version to target.
pub const VERSION: &str = "latest";
/// `authInfo.style` — the return style for the rpc binding.
pub const STYLE: &str = "rpc";

/// Errors from the RR glue layer. (Distinct from `platypus_core::Error`, which is I/O-free.)
#[derive(Debug)]
pub enum RrError {
    /// Transport / HTTP-status failure.
    Http(String),
    /// A SOAP `<faultstring>` returned by the service (bad auth, non-premium account, …).
    Fault(String),
    /// Local cache read/write failure.
    Io(String),
}

impl std::fmt::Display for RrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RrError::Http(s) => write!(f, "RR HTTP error: {s}"),
            RrError::Fault(s) => write!(f, "RR SOAP fault: {s}"),
            RrError::Io(s) => write!(f, "RR cache I/O error: {s}"),
        }
    }
}
impl std::error::Error for RrError {}

pub type Result<T> = std::result::Result<T, RrError>;

/// Client behaviour — the cache directory, the minimum spacing between *live* requests, and whether
/// to bypass the cache. Defaults are deliberately gentle on the API.
#[derive(Debug, Clone)]
pub struct Options {
    /// The cache **base** directory (default: the OS's conventional cache location — macOS
    /// `~/Library/Caches/platypus/rr`, Windows `%LOCALAPPDATA%\platypus\rr`, else
    /// `$XDG_CACHE_HOME`/`~/.cache/platypus/rr`). The client scopes it per account underneath.
    pub cache_dir: PathBuf,
    /// Minimum interval between live requests on one client (default 0.5 s). Only gates *live*
    /// requests — cache hits skip it — so it's a first-visit cost; the map's site warm parallelizes
    /// across forked clients ([`RrClient::fork`]) to fill faster still.
    pub min_interval: Duration,
    /// Re-fetch even on a cache hit (default false).
    pub refresh: bool,
    /// Cache **time-to-live**: a cached entry older than this is re-fetched on use (freshness). `None`
    /// disables expiry (cache forever). Default 30 days — gentle on the API, yet not indefinitely
    /// stale. A re-fetch that fails falls back to the stale cache, so freshness never breaks offline.
    pub max_age: Option<Duration>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            cache_dir: default_cache_dir(),
            min_interval: Duration::from_millis(500),
            refresh: false,
            max_age: Some(Duration::from_secs(30 * 24 * 60 * 60)),
        }
    }
}

/// Whether a cache entry fetched at `fetched` is stale now (`now`) given `max_age`. Pure — the
/// freshness decision, split out so it's unit-testable without the clock or filesystem. `None`
/// max_age (cache forever) is never stale; a fetch time in the future (clock skew) is treated fresh.
pub fn is_stale(fetched: SystemTime, max_age: Option<Duration>, now: SystemTime) -> bool {
    match max_age {
        None => false,
        Some(ttl) => now
            .duration_since(fetched)
            .map(|age| age > ttl)
            .unwrap_or(false),
    }
}

/// A `SystemTime` as Unix epoch seconds (0 if before the epoch) — for comparing a cache file's mtime
/// against RR's `lastUpdated`.
pub fn epoch_secs(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Read a cache file, returning `Some(body)` only when it reads **and is non-empty** — a 0-byte or
/// truncated file (e.g. a legacy/interrupted write) is treated as a miss, never served as an empty
/// "hit".
fn read_cache(path: &std::path::Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(body) if !body.trim().is_empty() => Some(body),
        _ => None,
    }
}

/// Write a cache entry **atomically**: write to a unique temp file in the same directory, then rename
/// it into place (atomic on one filesystem). A reader never sees a half-written file, and the
/// concurrent site-warm forks (distinct final paths) can't clobber one another.
fn write_cache(path: &std::path::Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| RrError::Io(e.to_string()))?;
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp{}-{n}", std::process::id()));
    fs::write(&tmp, body).map_err(|e| RrError::Io(e.to_string()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        RrError::Io(e.to_string())
    })
}

/// The default cache root, using each OS's conventional **cache** location (responses are
/// re-fetchable, so a cache dir the OS may purge is correct). Falls back to the temp dir if the
/// environment is bare. Per-account scoping is applied on top of this by [`RrClient::with_options`].
fn default_cache_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Library").join("Caches"))
        .unwrap_or_else(std::env::temp_dir);

    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local"))
        })
        .unwrap_or_else(std::env::temp_dir);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);

    base.join("platypus").join("rr")
}

/// A per-account cache subdirectory — the first 16 hex of `sha256(username)`. Upstream data sources
/// are authenticated per user, so each account's cached responses live under their own segment: the
/// cache is bound to whoever fetched it and different accounts don't co-mingle on disk. This is data
/// hygiene / provenance, **not** an access control — filesystem permissions (and anyone with admin
/// access) still govern who can read the files. A hash, not the raw login, keeps a semi-identifying
/// username out of the path.
fn account_segment(username: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(username.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A RadioReference SOAP client. Cheap to construct; holds the per-user credentials + a throttle.
pub struct RrClient {
    // Consumed only when building a live request envelope — dead when built without `http`.
    #[cfg_attr(not(feature = "http"), allow(dead_code))]
    creds: Credentials,
    opts: Options,
    // Only consulted by `throttle`, which spaces live requests — dead when built without `http`.
    #[cfg_attr(not(feature = "http"), allow(dead_code))]
    last_request: Mutex<Option<Instant>>,
    /// One-shot cache bypass for a manual "Refresh" — forces the next fetches live (overwriting the
    /// cache), independent of the static `opts.refresh`. Cleared by the caller when the refresh
    /// completes. Consulted only in `call`.
    force_refresh: AtomicBool,
    /// The effective cache root: `opts.cache_dir` scoped to this account (`<base>/<account_segment>`),
    /// so cached responses are bound to the login that fetched them and different accounts don't
    /// co-mingle on disk. All cache paths hang off this.
    cache_root: PathBuf,
}

impl RrClient {
    /// A client with the default (gentle) [`Options`].
    pub fn new(creds: Credentials) -> Self {
        Self::with_options(creds, Options::default())
    }

    pub fn with_options(creds: Credentials, opts: Options) -> Self {
        let cache_root = opts.cache_dir.join(account_segment(&creds.username));
        RrClient {
            creds,
            opts,
            last_request: Mutex::new(None),
            force_refresh: AtomicBool::new(false),
            cache_root,
        }
    }

    /// Arm/disarm the one-shot cache bypass for a manual "Refresh": while set, `call` fetches live and
    /// overwrites the cache. The caller sets it, re-fetches the data it wants fresh, then clears it.
    pub fn set_force_refresh(&self, on: bool) {
        self.force_refresh.store(on, Ordering::Relaxed);
    }

    /// A fresh client with this one's credentials + options — hence the **same on-disk cache** — but an
    /// independent throttle. Used to fetch several queries concurrently (the map's site warm) without
    /// the shared per-client throttle serializing them: each fork's first (and only) request fires
    /// immediately, so N forks give N-way parallelism.
    pub fn fork(&self) -> RrClient {
        Self::with_options(self.creds.clone(), self.opts.clone())
    }

    /// When a query was last fetched (its cache file's mtime), or `None` if never cached — powers the
    /// UI "as of <date>" freshness caption.
    pub fn cached_at(&self, method: &str, params_xml: &str) -> Option<SystemTime> {
        fs::metadata(self.cache_path(method, params_xml))
            .and_then(|m| m.modified())
            .ok()
    }

    /// When this county's `getCountyInfo` (the location's base data) was last fetched — the anchor
    /// for the "as of <date>" caption. `None` if never cached.
    pub fn county_info_fetched_at(&self, ctid: u32) -> Option<SystemTime> {
        self.cached_at("getCountyInfo", &int_param("ctid", ctid))
    }

    /// Whether this system's `getTrsSites` is already cached and fresh — i.e. warming its map site
    /// will **hit disk, not the network**. Lets the warm fill every cached site at once (instant) and
    /// pace only the live fetches.
    pub fn trs_sites_fresh(&self, sid: u32) -> bool {
        if self.opts.refresh || self.force_refresh.load(Ordering::Relaxed) {
            return false;
        }
        self.cached_at("getTrsSites", &int_param("sid", sid))
            .is_some_and(|t| !is_stale(t, self.opts.max_age, SystemTime::now()))
    }

    /// `getZipcodeInfo(zipcode)` — the location-first entry point (zip → county/state/lat/lon).
    /// Returns the raw SOAP response body (typed parse comes next).
    pub fn get_zipcode_info(&self, zip: u32) -> Result<String> {
        self.call("getZipcodeInfo", &int_param("zipcode", zip))
    }

    /// `getCountyInfo(ctid)` — a county's trunked systems (`trsList`), conventional categories
    /// (`cats`), and agencies (`agencyList`).
    pub fn get_county_info(&self, ctid: u32) -> Result<String> {
        self.call("getCountyInfo", &int_param("ctid", ctid))
    }

    /// `getTrsDetails(sid)` — a trunked system's header (`sName`, type/flavor/voice ids, geo).
    pub fn get_trs_details(&self, sid: u32) -> Result<String> {
        self.call("getTrsDetails", &int_param("sid", sid))
    }

    /// `getTrsSites(sid)` — a trunked system's sites (geo + control/voice frequencies).
    pub fn get_trs_sites(&self, sid: u32) -> Result<String> {
        self.call("getTrsSites", &int_param("sid", sid))
    }

    /// `getTrsTalkgroups(sid, …)` — all talkgroups for a system (the category/tag/dec filters are 0 = all).
    pub fn get_trs_talkgroups(&self, sid: u32) -> Result<String> {
        self.get_trs_talkgroups_in_cat(sid, 0)
    }

    /// `getTrsTalkgroups(sid, tgCid)` — the talkgroups in one category (`tg_cid`; 0 = all). The
    /// location-first drill fetches a category's talkgroups only when it's opened.
    pub fn get_trs_talkgroups_in_cat(&self, sid: u32, tg_cid: u32) -> Result<String> {
        self.get_trs_talkgroups_in_cat_fresh(sid, tg_cid, None)
    }

    /// As [`get_trs_talkgroups_in_cat`], but honoring an upstream freshness bound: pass the parent
    /// category's `lastUpdated` (epoch secs) so the cached talkgroups re-download only when RR changed
    /// that category. `None` ⇒ TTL-only freshness.
    pub fn get_trs_talkgroups_in_cat_fresh(
        &self,
        sid: u32,
        tg_cid: u32,
        upstream_updated: Option<i64>,
    ) -> Result<String> {
        let params = format!(
            "{}{}{}{}",
            int_param("sid", sid),
            int_param("tgCid", tg_cid),
            int_param("tgTag", 0),
            int_param("tgDec", 0),
        );
        self.call_if_stale("getTrsTalkgroups", &params, upstream_updated)
    }

    /// `getSubcatFreqs(scid)` — the conventional frequencies in a subcategory.
    pub fn get_subcat_freqs(&self, scid: u32) -> Result<String> {
        self.call("getSubcatFreqs", &int_param("scid", scid))
    }

    /// `getTrsType(id)` — resolve a system-type id to its name.
    pub fn get_trs_type(&self, id: u32) -> Result<String> {
        self.call("getTrsType", &int_param("id", id))
    }

    /// `getTrsFlavor(id)` — resolve a system-flavor id to its name.
    pub fn get_trs_flavor(&self, id: u32) -> Result<String> {
        self.call("getTrsFlavor", &int_param("id", id))
    }

    /// `getTrsVoice(id)` — resolve a system-voice id to its name.
    pub fn get_trs_voice(&self, id: u32) -> Result<String> {
        self.call("getTrsVoice", &int_param("id", id))
    }

    /// `getStateInfo(stid)` — a state's trunked systems (`trsList`), agencies, and counties.
    pub fn get_state_info(&self, stid: u32) -> Result<String> {
        self.call("getStateInfo", &int_param("stid", stid))
    }

    /// `getMode(id)` — resolve a modulation-mode id to its name (the numeric `freq.mode`).
    pub fn get_mode(&self, id: u32) -> Result<String> {
        self.call("getMode", &int_param("mode", id))
    }

    /// `getTag(id)` — resolve a service-type tag id to its RR name.
    pub fn get_tag(&self, id: u32) -> Result<String> {
        self.call("getTag", &int_param("id", id))
    }

    /// `getTrsTalkgroupCats(sid)` — a system's talkgroup categories (group TGs by `tgCid`).
    pub fn get_trs_talkgroup_cats(&self, sid: u32) -> Result<String> {
        self.call("getTrsTalkgroupCats", &int_param("sid", sid))
    }

    /// `getAgencyInfo(aid)` — an agency's conventional categories/subcategories.
    pub fn get_agency_info(&self, aid: u32) -> Result<String> {
        self.call("getAgencyInfo", &int_param("aid", aid))
    }

    /// `getCountyFreqsByTag(ctid, tag)` — a county's conventional freqs filtered by service-type tag.
    pub fn get_county_freqs_by_tag(&self, ctid: u32, tag: u32) -> Result<String> {
        let params = format!("{}{}", int_param("ctid", ctid), int_param("tag", tag));
        self.call("getCountyFreqsByTag", &params)
    }

    /// `getAgencyFreqsByTag(aid, tag)` — an agency's conventional freqs filtered by service-type tag.
    pub fn get_agency_freqs_by_tag(&self, aid: u32, tag: u32) -> Result<String> {
        let params = format!("{}{}", int_param("aid", aid), int_param("tag", tag));
        self.call("getAgencyFreqsByTag", &params)
    }

    // --- Geo hierarchy (top-down Country → State → County → Metro navigation) ---

    /// `getCountryList()` — every country RR covers (the top of the geographic drill).
    pub fn get_country_list(&self) -> Result<String> {
        self.call("getCountryList", "")
    }

    /// `getCountryInfo(coid)` — a country's states + national agencies.
    pub fn get_country_info(&self, coid: u32) -> Result<String> {
        self.call("getCountryInfo", &int_param("coid", coid))
    }

    /// `getStatesByList(stids)` — resolve a batch of state ids to names (completeness — the `*Info`
    /// calls already return named lists).
    pub fn get_states_by_list(&self, stids: &[u32]) -> Result<String> {
        self.call("getStatesByList", &id_list_param("request", "stid", stids))
    }

    /// `getCountiesByList(ctids)` — resolve a batch of county ids to names.
    pub fn get_counties_by_list(&self, ctids: &[u32]) -> Result<String> {
        self.call(
            "getCountiesByList",
            &id_list_param("request", "ctid", ctids),
        )
    }

    /// `getMetroArea(mid)` — a metro area's name (urban multi-county grouping).
    pub fn get_metro_area(&self, mid: u32) -> Result<String> {
        self.call("getMetroArea", &int_param("mid", mid))
    }

    /// `getMetroAreaInfo(mid)` — the counties that make up a metro area.
    pub fn get_metro_area_info(&self, mid: u32) -> Result<String> {
        self.call("getMetroAreaInfo", &int_param("mid", mid))
    }

    // --- Reverse lookup & frequency search ("what am I hearing?") ---

    /// `getTrsBySysid(sysid)` — the trunked system(s) matching a decoded on-air System ID.
    pub fn get_trs_by_sysid(&self, sysid: &str) -> Result<String> {
        self.call("getTrsBySysid", &str_param("sysid", sysid))
    }

    /// `searchCountyFreq(ctid, freq, tone)` — identify a conventional freq (+ optional `tone`, ""=any)
    /// within a county. Returns richer `searchFreqResult`s carrying the matching system/agency refs.
    pub fn search_county_freq(&self, ctid: u32, freq: f64, tone: &str) -> Result<String> {
        let params = format!(
            "{}{}{}",
            int_param("ctid", ctid),
            dec_param("freq", freq),
            str_param("tone", tone)
        );
        self.call("searchCountyFreq", &params)
    }

    /// `searchStateFreq(stid, freq, tone)` — as [`Self::search_county_freq`], statewide.
    pub fn search_state_freq(&self, stid: u32, freq: f64, tone: &str) -> Result<String> {
        let params = format!(
            "{}{}{}",
            int_param("stid", stid),
            dec_param("freq", freq),
            str_param("tone", tone)
        );
        self.call("searchStateFreq", &params)
    }

    /// `searchMetroFreq(mid, freq, tone)` — as [`Self::search_county_freq`], across a metro area.
    pub fn search_metro_freq(&self, mid: u32, freq: f64, tone: &str) -> Result<String> {
        let params = format!(
            "{}{}{}",
            int_param("mid", mid),
            dec_param("freq", freq),
            str_param("tone", tone)
        );
        self.call("searchMetroFreq", &params)
    }

    // --- FCC ULS database (a distinct, non-hierarchical data axis) ---

    /// `fccGetCallsign(callsign)` — an FCC license's details (licensee, status, its frequencies).
    pub fn fcc_get_callsign(&self, callsign: &str) -> Result<String> {
        self.call("fccGetCallsign", &str_param("callsign", callsign))
    }

    /// `fccGetRadioServiceCode(code)` — resolve an FCC radio-service code to its description.
    pub fn fcc_get_radio_service_code(&self, code: &str) -> Result<String> {
        self.call("fccGetRadioServiceCode", &str_param("code", code))
    }

    /// `fccGetProxCallsigns(lat, lon, range, unit)` — FCC licenses near a point (`unit` = `m`/`km`).
    /// The one true coordinate-radius query — location-first without the geographic hierarchy.
    pub fn fcc_get_prox_callsigns(
        &self,
        lat: f64,
        lon: f64,
        range: f64,
        unit: &str,
    ) -> Result<String> {
        let params = format!(
            "{}{}{}{}",
            dec_param("lat", lat),
            dec_param("lon", lon),
            dec_param("range", range),
            str_param("unit", unit)
        );
        self.call("fccGetProxCallsigns", &params)
    }

    // --- Account diagnostics ---

    /// `getUserData()` — the authenticated user's account (username + subscription expiry).
    pub fn get_user_data(&self) -> Result<String> {
        self.call("getUserData", "")
    }

    /// `getUserFeedBroadcasts()` — the user's Broadcastify feed broadcasts (secrets are not parsed).
    pub fn get_user_feed_broadcasts(&self) -> Result<String> {
        self.call("getUserFeedBroadcasts", "")
    }

    // --- Assembled fetches (chain + parse → canonical-ready RR structs) ---

    /// Resolve a system-type id to its name (cached).
    pub fn resolve_type(&self, id: u32) -> Option<String> {
        parse::parse_taxonomy_name(&self.get_trs_type(id).ok()?, "sTypeDescr")
    }

    /// Resolve a system-flavor id to its name (cached).
    pub fn resolve_flavor(&self, id: u32) -> Option<String> {
        parse::parse_taxonomy_name(&self.get_trs_flavor(id).ok()?, "sFlavorDescr")
    }

    /// Resolve a numeric modulation-mode id (RR's `freq.mode`) to its display name (cached).
    pub fn resolve_mode(&self, id: u32) -> Option<String> {
        parse::parse_mode_name(&self.get_mode(id).ok()?)
    }

    /// Fetch one trunked system fully (details + sites + talkgroups), type/flavor names resolved.
    pub fn fetch_trunked(&self, sid: u32) -> Result<Option<RrTrunkedSystem>> {
        let details = self.get_trs_details(sid)?;
        let sites = self.get_trs_sites(sid)?;
        let talkgroups = self.get_trs_talkgroups(sid)?;
        let Some(mut sys) = parse_trunked_system(&details, &sites, &talkgroups) else {
            return Ok(None);
        };
        if sys.trs.s_type > 0 {
            sys.trs.type_name = self.resolve_type(sys.trs.s_type as u32);
        }
        if sys.trs.s_flavor > 0 {
            sys.trs.flavor_name = self.resolve_flavor(sys.trs.s_flavor as u32);
        }
        Ok(Some(sys))
    }

    /// Fetch a conventional subcategory's frequencies as one system (empty ⇒ nothing to add).
    pub fn fetch_conventional(
        &self,
        scid: u32,
        name: &str,
        county_ids: Vec<u64>,
    ) -> Result<Option<RrConventionalSystem>> {
        let freqs = parse::parse_subcat_freqs(&self.get_subcat_freqs(scid)?);
        Ok(conventional_from_freqs(name, freqs, county_ids))
    }

    /// Fetch a county's systems — trunked (fully) + conventional. `max_trunked`/`max_conventional`
    /// cap the pass so a dev run stays gentle on the API (each unique call is cached anyway).
    pub fn fetch_county(
        &self,
        ctid: u32,
        max_trunked: usize,
        max_conventional: usize,
    ) -> Result<Vec<RrSystem>> {
        let contents = parse::parse_county(&self.get_county_info(ctid)?);
        let mut systems = Vec::new();
        for t in contents.trs.iter().take(max_trunked) {
            if let Some(sys) = self.fetch_trunked(t.sid as u32)? {
                systems.push(RrSystem::Trunked(sys));
            }
        }
        for sc in contents
            .subcats
            .iter()
            .filter(|s| !s.trunked_ref)
            .take(max_conventional)
        {
            if let Some(sys) =
                self.fetch_conventional(sc.scid as u32, &sc.name, vec![ctid as u64])?
            {
                systems.push(RrSystem::Conventional(sys));
            }
        }
        Ok(systems)
    }

    /// Fetch a state's trunked systems fully — the statewide, multi-county P25 systems a single
    /// county's `trsList` misses. `max_trunked` caps the pass (each unique call is cached anyway).
    pub fn fetch_state(&self, stid: u32, max_trunked: usize) -> Result<Vec<RrSystem>> {
        let state = parse::parse_state(&self.get_state_info(stid)?);
        let mut systems = Vec::new();
        for t in state.trs.iter().take(max_trunked) {
            if let Some(sys) = self.fetch_trunked(t.sid as u32)? {
                systems.push(RrSystem::Trunked(sys));
            }
        }
        Ok(systems)
    }

    /// Resolve a decoded on-air System ID to its full trunked system(s) — `getTrsBySysid` then
    /// `fetch_trunked` for each hit. The reverse of the location-first drill ("what am I hearing?").
    pub fn fetch_by_sysid(&self, sysid: &str) -> Result<Vec<RrSystem>> {
        let hits = parse::parse_trs_list(&self.get_trs_by_sysid(sysid)?);
        let mut systems = Vec::new();
        for t in &hits {
            if let Some(sys) = self.fetch_trunked(t.sid as u32)? {
                systems.push(RrSystem::Trunked(sys));
            }
        }
        Ok(systems)
    }

    /// The authenticated user's account info (`getUserData`) — for a friendly pre-flight check
    /// (surface an expired/absent subscription before a fetch storm).
    pub fn account(&self) -> Result<parse::UserInfo> {
        parse::parse_user_data(&self.get_user_data()?)
            .ok_or_else(|| RrError::Http("getUserData returned no UserInfo".into()))
    }

    /// Fetch a county's conventional freqs for one service-type `tag` in a single targeted call —
    /// the gentlest conventional query ("give me all Fire in this county"). Empty ⇒ nothing to add.
    pub fn fetch_county_by_tag(
        &self,
        ctid: u32,
        tag: u32,
        name: &str,
    ) -> Result<Option<RrConventionalSystem>> {
        let freqs = parse::parse_subcat_freqs(&self.get_county_freqs_by_tag(ctid, tag)?);
        Ok(conventional_from_freqs(name, freqs, vec![ctid as u64]))
    }

    /// Fetch an agency's conventional freqs for one service-type `tag` in a single targeted call.
    pub fn fetch_agency_by_tag(
        &self,
        aid: u32,
        tag: u32,
        name: &str,
    ) -> Result<Option<RrConventionalSystem>> {
        let freqs = parse::parse_subcat_freqs(&self.get_agency_freqs_by_tag(aid, tag)?);
        Ok(conventional_from_freqs(name, freqs, Vec::new()))
    }

    /// POST a SOAP method (cache-first). The cache file name is keyed by `method + params`; the
    /// account scoping lives in the cache root (see [`cache_path`](Self::cache_path)), so a query is
    /// reused within an account without another network hit.
    fn call(&self, method: &str, params_xml: &str) -> Result<String> {
        self.call_if_stale(method, params_xml, None)
    }

    /// `call`, plus an optional **upstream freshness bound**: a cached entry is honored only if it is
    /// newer than `upstream_updated` (epoch secs). This is the two-tier validation — a cheap parent
    /// query (which carries per-item `lastUpdated`) invalidates a heavy child precisely, so we
    /// re-download the child only when RR actually changed it. `None` ⇒ TTL-only freshness.
    fn call_if_stale(
        &self,
        method: &str,
        params_xml: &str,
        upstream_updated: Option<i64>,
    ) -> Result<String> {
        let path = self.cache_path(method, params_xml);
        let bypass = self.opts.refresh || self.force_refresh.load(Ordering::Relaxed);
        if !bypass {
            if let Ok(md) = fs::metadata(&path) {
                let fetched = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let ttl_stale = is_stale(fetched, self.opts.max_age, SystemTime::now());
                let upstream_stale = upstream_updated.is_some_and(|u| epoch_secs(fetched) < u);
                if !ttl_stale && !upstream_stale {
                    if let Some(body) = read_cache(&path) {
                        return Ok(body);
                    }
                }
            }
        }

        // Re-fetch live. If that fails but we have *any* (non-empty) cached copy, fall back to it —
        // freshness must never cost availability (offline / transient RR fault).
        let body = match self.fetch_live(method, params_xml) {
            Ok(b) => b,
            Err(e) => {
                if let Some(stale) = read_cache(&path) {
                    return Ok(stale);
                }
                return Err(e);
            }
        };
        if let Some(fault) = soap_fault(&body) {
            if let Some(stale) = read_cache(&path) {
                return Ok(stale);
            }
            return Err(RrError::Fault(fault));
        }

        write_cache(&path, &body)?;
        Ok(body)
    }

    /// Perform one live request (throttled) — the only networked step. Available only with the
    /// `http` feature; otherwise it errors so a misbuilt consumer fails loudly instead of silently.
    #[cfg(feature = "http")]
    fn fetch_live(&self, method: &str, params_xml: &str) -> Result<String> {
        self.throttle();
        let envelope = build_envelope(method, &self.creds, params_xml);
        http_post(&envelope, method)
    }

    #[cfg(not(feature = "http"))]
    fn fetch_live(&self, _method: &str, _params_xml: &str) -> Result<String> {
        Err(RrError::Http(
            "platypus-rr built without the `http` feature — enable it, or POST request()/parse yourself"
                .into(),
        ))
    }

    /// Sleep just enough to keep at least `min_interval` between live requests.
    #[cfg(feature = "http")]
    fn throttle(&self) {
        let mut last = self.last_request.lock().unwrap();
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.opts.min_interval {
                std::thread::sleep(self.opts.min_interval - elapsed);
            }
        }
        *last = Some(Instant::now());
    }

    /// `<cache_root>/<method>-<sha256(method+params)>.xml` — the file name is stable per query; the
    /// account scoping lives in `cache_root` (`<base>/<account_segment>`), so entries are bound to the
    /// authenticated user who fetched them.
    fn cache_path(&self, method: &str, params_xml: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(method.as_bytes());
        hasher.update(b"\n");
        hasher.update(params_xml.as_bytes());
        let hash = hasher.finalize();
        let short = hash
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        self.cache_root.join(format!("{method}-{short}.xml"))
    }
}

/// Build an rpc-style SOAP 1.1 envelope with the `authInfo` block. Credentials are XML-escaped.
fn build_envelope(method: &str, creds: &Credentials, params_xml: &str) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"",
            " xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\"",
            " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
            " xmlns:ns=\"{ns}\">",
            "<soap:Body><ns:{method}>{params}",
            "<authInfo xsi:type=\"ns:authInfo\">",
            "<username xsi:type=\"xsd:string\">{user}</username>",
            "<password xsi:type=\"xsd:string\">{pass}</password>",
            "<appKey xsi:type=\"xsd:string\">{key}</appKey>",
            "<version xsi:type=\"xsd:string\">{version}</version>",
            "<style xsi:type=\"xsd:string\">{style}</style>",
            "</authInfo></ns:{method}></soap:Body></soap:Envelope>",
        ),
        ns = NAMESPACE,
        method = method,
        params = params_xml,
        user = xml_escape(&creds.username),
        pass = xml_escape(&creds.password),
        key = xml_escape(&creds.app_key),
        version = VERSION,
        style = STYLE,
    )
}

/// A ready-to-send SOAP request — the **portable transport seam**. A front-end POSTs `body` to `url`
/// with `SOAPAction: soap_action` + `Content-Type: content_type` using its own native HTTP stack
/// (`URLSession`, `HttpClient`, …). [`RrClient`] (with the `http` feature) is the native-OS-TLS
/// implementation of this seam.
pub struct SoapRequest {
    pub url: String,
    pub soap_action: String,
    pub content_type: String,
    pub body: String,
}

/// Build the SOAP request for `method` with its params — no I/O. Pair with [`parse`] on the response.
pub fn request(method: &str, params_xml: &str, creds: &Credentials) -> SoapRequest {
    SoapRequest {
        url: ENDPOINT.to_string(),
        soap_action: format!("{NAMESPACE}#{method}"),
        content_type: "text/xml; charset=utf-8".to_string(),
        body: build_envelope(method, creds, params_xml),
    }
}

/// Assemble a trunked system from its three response bodies (details + sites + talkgroups) into the
/// core [`RrTrunkedSystem`], ready for `RadioReferenceProvider::from_systems`.
pub fn parse_trunked_system(
    details_xml: &str,
    sites_xml: &str,
    talkgroups_xml: &str,
) -> Option<RrTrunkedSystem> {
    Some(RrTrunkedSystem {
        trs: parse::parse_trs_details(details_xml)?,
        sites: parse::parse_trs_sites(sites_xml),
        talkgroups: parse::parse_talkgroups(talkgroups_xml),
    })
}

/// Wrap a flat list of conventional frequencies as one `RrConventionalSystem` (empty ⇒ `None`).
/// Shared by the subcategory and tag-filtered conventional fetches — the freqs already carry their
/// own service-type tags, so the wrapper subcat needs only a display name.
fn conventional_from_freqs(
    name: &str,
    freqs: Vec<platypus_core::rr::Freq>,
    county_ids: Vec<u64>,
) -> Option<RrConventionalSystem> {
    if freqs.is_empty() {
        return None;
    }
    let subcat = Subcat {
        sc_name: name.to_string(),
        lat: 0.0,
        lon: 0.0,
        range: 0.0,
    };
    Some(RrConventionalSystem {
        name: name.to_string(),
        county_ids,
        state_ids: Vec::new(),
        subcats: vec![(subcat, freqs)],
    })
}

/// A single typed `xsd:int` parameter element for an rpc SOAP method body.
fn int_param(name: &str, value: u32) -> String {
    format!("<{name} xsi:type=\"xsd:int\">{value}</{name}>")
}

/// A single typed `xsd:string` parameter element (value XML-escaped).
fn str_param(name: &str, value: &str) -> String {
    format!(
        "<{name} xsi:type=\"xsd:string\">{}</{name}>",
        xml_escape(value)
    )
}

/// A single typed `xsd:decimal` parameter element (Rust's shortest round-trip float repr).
fn dec_param(name: &str, value: f64) -> String {
    format!("<{name} xsi:type=\"xsd:decimal\">{value}</{name}>")
}

/// A `SOAP-ENC:Array` request parameter — `<part …arrayType="tns:item[N]"><item …><item>V</item>…`.
/// Used by `getStatesByList`/`getCountiesByList`, whose `request` is an array of `{stid}`/`{ctid}`.
fn id_list_param(part: &str, item: &str, ids: &[u32]) -> String {
    let mut out = format!(
        "<{part} xsi:type=\"SOAP-ENC:Array\" SOAP-ENC:arrayType=\"tns:{item}[{}]\">",
        ids.len()
    );
    for id in ids {
        out.push_str(&format!(
            "<item xsi:type=\"tns:{item}\">{}</item>",
            int_param(item, *id)
        ));
    }
    out.push_str(&format!("</{part}>"));
    out
}

/// Extract a SOAP `<faultstring>` (namespace-agnostic) if the body is a fault.
fn soap_fault(body: &str) -> Option<String> {
    let start = body.find("faultstring")?;
    let after = &body[start..];
    let open = after.find('>')? + start + 1;
    let close = body[open..].find('<')? + open;
    Some(body[open..close].trim().to_string())
}

/// POST a SOAP envelope to [`ENDPOINT`] over **native OS TLS** (`ureq` + `native-tls`: Secure
/// Transport on macOS, SChannel on Windows, system OpenSSL on Linux), returning the raw response
/// body. HTTPS is enforced: a non-`https` endpoint is refused, and redirects are disabled so the
/// request can never be silently downgraded to `http`. RR returns SOAP faults as HTTP 500 *with*
/// the fault body, so a status error keeps its body for [`soap_fault`] to parse.
#[cfg(feature = "http")]
fn http_post(envelope: &str, method: &str) -> Result<String> {
    if !ENDPOINT.starts_with("https://") {
        return Err(RrError::Http(format!(
            "refusing to POST credentials to a non-HTTPS endpoint: {ENDPOINT}"
        )));
    }
    // Wire the OS TLS backend explicitly: ureq auto-configures rustls, but the native-tls backend
    // must be handed in as a connector (which is why `native-tls` is a direct dependency).
    let connector = native_tls::TlsConnector::new().map_err(|e| RrError::Http(e.to_string()))?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .tls_connector(std::sync::Arc::new(connector))
        .build();
    let resp = agent
        .post(ENDPOINT)
        .set("Content-Type", "text/xml; charset=utf-8")
        .set("SOAPAction", &format!("\"{NAMESPACE}#{method}\""))
        .set("User-Agent", USER_AGENT)
        .send_string(envelope);
    let resp = match resp {
        Ok(r) => r,
        // Keep the body on a status error — RR delivers `<faultstring>` in a 500 response.
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(t)) => return Err(RrError::Http(t.to_string())),
    };
    resp.into_string().map_err(|e| RrError::Http(e.to_string()))
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials {
            app_key: "KEY".into(),
            username: "user".into(),
            password: "p&ss<w>".into(),
        }
    }

    #[test]
    fn envelope_has_method_params_and_escaped_auth() {
        let env = build_envelope("getZipcodeInfo", &creds(), "<zipcode>97201</zipcode>");
        assert!(env.contains("<ns:getZipcodeInfo>"));
        assert!(env.contains("<zipcode>97201</zipcode>"));
        assert!(env.contains("<appKey xsi:type=\"xsd:string\">KEY</appKey>"));
        // password is XML-escaped
        assert!(env.contains("p&amp;ss&lt;w&gt;"));
        assert!(!env.contains("p&ss<w>"));
    }

    #[test]
    fn detects_soap_fault() {
        let body = "<soap:Fault><faultcode>x</faultcode><faultstring>Invalid credentials</faultstring></soap:Fault>";
        assert_eq!(soap_fault(body).as_deref(), Some("Invalid credentials"));
        assert_eq!(soap_fault("<ok/>"), None);
    }

    #[test]
    fn ttl_freshness_boundaries() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let ttl = Some(Duration::from_secs(100));
        // Within TTL → fresh; past TTL → stale; exactly at TTL → fresh (age > ttl is strict).
        assert!(!is_stale(now - Duration::from_secs(50), ttl, now));
        assert!(is_stale(now - Duration::from_secs(150), ttl, now));
        assert!(!is_stale(now - Duration::from_secs(100), ttl, now));
        // No TTL → never stale; future fetch time (clock skew) → treated fresh.
        assert!(!is_stale(now - Duration::from_secs(9_999), None, now));
        assert!(!is_stale(now + Duration::from_secs(10), ttl, now));
    }

    #[test]
    fn epoch_secs_of_known_time() {
        assert_eq!(
            epoch_secs(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            1_700_000_000
        );
        assert_eq!(epoch_secs(SystemTime::UNIX_EPOCH), 0);
    }

    #[test]
    fn cache_file_name_is_credential_free_and_stable() {
        // The cache *file* (method+params) is credential-free: the same account with a different
        // password maps the same query to the same file (scoping is by username, not password).
        let a = RrClient::new(creds());
        let mut other = creds();
        other.password = "different".into();
        let b = RrClient::new(other);
        assert_eq!(
            a.cache_path("getZipcodeInfo", "<zipcode>1</zipcode>"),
            b.cache_path("getZipcodeInfo", "<zipcode>1</zipcode>"),
        );
    }

    #[test]
    fn account_segment_is_deterministic_hashed_and_username_free() {
        let a = account_segment("alice");
        assert_eq!(a, account_segment("alice")); // deterministic
        assert_ne!(a, account_segment("bob")); // per-account
        assert_eq!(a.len(), 16); // 8 bytes hex
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!account_segment("SecretLogin").contains("SecretLogin")); // no raw login on disk
    }

    #[test]
    fn cache_is_scoped_per_account() {
        // Different usernames never share a cache path (bound to whoever fetched it); an explicit
        // cache_dir override is honored, with the account segment underneath it.
        let opts = || Options {
            cache_dir: PathBuf::from("/tmp/base"),
            ..Options::default()
        };
        let alice = RrClient::with_options(
            Credentials {
                app_key: "K".into(),
                username: "alice".into(),
                password: "x".into(),
            },
            opts(),
        );
        let bob = RrClient::with_options(
            Credentials {
                app_key: "K".into(),
                username: "bob".into(),
                password: "x".into(),
            },
            opts(),
        );
        let q = ("getCountyInfo", "<ctid>9001</ctid>");
        assert_ne!(alice.cache_path(q.0, q.1), bob.cache_path(q.0, q.1));
        assert!(alice
            .cache_path(q.0, q.1)
            .starts_with(PathBuf::from("/tmp/base").join(account_segment("alice"))));
    }

    // --- Store/retrieve round-trip. These run without the `http` feature, where `fetch_live` errors,
    // so a served **cache** response (hit or fallback) is `Ok` while a genuine miss is `Err`. The live
    // fetch→write→hit path (proving the store side) is `tests/live_cache.rs`. ---

    /// A client over a fresh unique temp cache dir (cleaned up by the caller).
    fn temp_client() -> (RrClient, PathBuf) {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "platypus-rr-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let opts = Options {
            cache_dir: dir.clone(),
            ..Options::default()
        };
        (RrClient::with_options(creds(), opts), dir)
    }

    #[test]
    fn cache_hit_returns_stored_bytes() {
        let (client, dir) = temp_client();
        let (m, p) = ("getThing", "<a>1</a>");
        write_cache(&client.cache_path(m, p), "<cached-body/>").unwrap();
        // A present, fresh entry is served verbatim — no network (which would error without `http`).
        assert_eq!(client.call(m, p).unwrap(), "<cached-body/>");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_miss_has_nothing_to_serve() {
        let (client, dir) = temp_client();
        // No cached file and no live transport → a miss surfaces the error (nothing retrieved before).
        assert!(client.call("getThing", "<a>2</a>").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_cache_file_is_treated_as_a_miss() {
        let (client, dir) = temp_client();
        let (m, p) = ("getThing", "<a>3</a>");
        let path = client.cache_path(m, p);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "").unwrap(); // a 0-byte partial write
                                       // The guard rejects the empty file, so it's a miss (not served as an empty "hit").
        assert!(client.call(m, p).is_err());
        assert!(read_cache(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
