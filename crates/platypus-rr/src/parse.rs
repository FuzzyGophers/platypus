// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Pure, portable parsing of RadioReference rpc/encoded SOAP responses into the core RR structs.
//! No networking here — the caller supplies the response body (from its native HTTP, or [`RrClient`]'s
//! optional native-OS-TLS transport). Every parser is offline-testable against the fixtures in `tests/`.
//!
//! [`RrClient`]: crate::RrClient
//!
//! rpc/encoded shape: `…<return xsi:type="tns:T">…</return>` with scalars as
//! `<field xsi:type="xsd:…">value</field>` (or `xsi:nil="true"`) and lists as
//! `<field xsi:type="SOAP-ENC:Array"><item xsi:type="tns:T">…</item>…</field>`.

use platypus_core::rr::{Freq, SiteFrequency, Tag, Talkgroup, Trs, TrsSite};

/// A minimal XML node — a namespace-stripped element with its text, `xsi:nil` flag, and children.
/// Enough to navigate the (regular) SOAP responses without a full DOM library.
#[derive(Debug, Default, Clone)]
pub struct Node {
    pub name: String,
    pub text: String,
    pub nil: bool,
    pub children: Vec<Node>,
}

impl Node {
    /// Parse a (well-formed, machine-generated) XML document into its root node. A minimal scanner
    /// — enough for RR's regular rpc/encoded SOAP; not a general-purpose parser (no CDATA, DTDs).
    /// Slices are taken only at ASCII `<`/`>` boundaries, so UTF-8 stays intact.
    pub fn parse(xml: &str) -> Option<Node> {
        let mut stack: Vec<Node> = Vec::new();
        let mut root: Option<Node> = None;
        let mut rest = xml;

        while let Some(lt) = rest.find('<') {
            let text = rest[..lt].trim();
            if !text.is_empty() {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&unescape(text));
                }
            }
            rest = &rest[lt..];

            if rest.starts_with("<?") {
                rest = &rest[rest.find("?>")? + 2..];
                continue;
            }
            if rest.starts_with("<!--") {
                rest = &rest[rest.find("-->")? + 3..];
                continue;
            }

            let gt = rest.find('>')?;
            let inner = &rest[1..gt];
            rest = &rest[gt + 1..];

            if inner.starts_with('/') {
                if let Some(done) = stack.pop() {
                    close(&mut stack, &mut root, done);
                }
            } else {
                let self_close = inner.ends_with('/');
                let node = node_from_tag(inner.trim_end_matches('/'));
                if self_close {
                    close(&mut stack, &mut root, node);
                } else {
                    stack.push(node);
                }
            }
        }
        root
    }

    /// Direct child with local name `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Text of a direct child `name` (None if absent or `xsi:nil`).
    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).filter(|c| !c.nil).map(|c| c.text.as_str())
    }

    /// The `<item>` children of a `SOAP-ENC:Array` element.
    pub fn items(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(|c| c.name == "item")
    }

    /// First descendant (depth-first) with local name `name`.
    pub fn find(&self, name: &str) -> Option<&Node> {
        for c in &self.children {
            if c.name == name {
                return Some(c);
            }
            if let Some(found) = c.find(name) {
                return Some(found);
            }
        }
        None
    }

    fn f64_child(&self, name: &str) -> Option<f64> {
        self.child_text(name).and_then(|t| t.trim().parse().ok())
    }
    fn u64_child(&self, name: &str) -> Option<u64> {
        self.child_text(name).and_then(|t| t.trim().parse().ok())
    }
    fn str_child(&self, name: &str) -> String {
        self.child_text(name).unwrap_or("").trim().to_string()
    }
}

/// Build a node from a start/empty tag's inner text (`name attr="v" …`) — the local name (prefix
/// stripped) plus the `xsi:nil="true"` flag.
fn node_from_tag(tag: &str) -> Node {
    let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
    let raw = &tag[..name_end];
    Node {
        name: raw.rsplit(':').next().unwrap_or(raw).to_string(),
        nil: tag.contains("nil=\"true\""),
        ..Default::default()
    }
}

/// Unescape the five XML entities + numeric char references. Unknown `&…;` are left verbatim.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn close(stack: &mut [Node], root: &mut Option<Node>, done: Node) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(done),
        None => *root = Some(done),
    }
}

/// The `<return>` payload of a response (the element right under `…Response`).
fn return_node(xml: &str) -> Option<Node> {
    Node::parse(xml)?.find("return").cloned()
}

/// Map the `<item>` children of a response's `<return>` array through `f` — the shared shape of the
/// per-endpoint list parsers. `f` returns `None` to skip an item; a missing/empty `<return>` yields
/// an empty vec.
fn parse_items<T>(xml: &str, f: impl Fn(&Node) -> Option<T>) -> Vec<T> {
    return_node(xml)
        .map(|root| root.items().filter_map(f).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Response parsers → core RR structs.
// ---------------------------------------------------------------------------

/// `getZipcodeInfo` → the county/state ids + centroid used to seed a location-first fetch.
#[derive(Debug, Clone, PartialEq)]
pub struct ZipInfo {
    pub ctid: u64,
    pub stid: u64,
    pub lat: f64,
    pub lon: f64,
    pub city: String,
}

pub fn parse_zipcode_info(xml: &str) -> Option<ZipInfo> {
    let r = return_node(xml)?;
    Some(ZipInfo {
        ctid: r.u64_child("ctid")?,
        stid: r.u64_child("stid").unwrap_or(0),
        lat: r.f64_child("lat").unwrap_or(0.0),
        lon: r.f64_child("lon").unwrap_or(0.0),
        city: r.str_child("city"),
    })
}

/// A trunked system listed by a county/state (`trsList` / `getTrsBySysid`). `getCountyInfo` and
/// `getStateInfo` include `sName`, so a system list is displayable from one call (no per-system detail).
#[derive(Debug, Clone, PartialEq)]
pub struct CountyTrs {
    pub sid: u64,
    pub name: String,
    /// The system's home city (`sCity`), when the list carries it — a cheap subtitle for the UI.
    pub city: String,
    pub s_type: i32,
    pub s_flavor: i32,
}

/// Build a `CountyTrs` from a `TrsListDef` item (`sName`/`sCity` are absent in some lists → empty).
fn county_trs_from(it: &Node) -> CountyTrs {
    CountyTrs {
        sid: it.u64_child("sid").unwrap_or(0),
        name: it.str_child("sName"),
        city: it.str_child("sCity"),
        s_type: it.u64_child("sType").unwrap_or(0) as i32,
        s_flavor: it.u64_child("sFlavor").unwrap_or(0) as i32,
    }
}

/// A conventional subcategory listed by a county (under `cats`). `trunked_ref` = it points at a
/// trunked system (has `sids`); those are covered by `trsList`, so the conventional pass skips them.
#[derive(Debug, Clone, PartialEq)]
pub struct CountySubcat {
    pub scid: u64,
    pub name: String,
    pub trunked_ref: bool,
    /// The subcategory's coverage centroid + radius — a real map location for a conventional system.
    pub lat: f64,
    pub lon: f64,
    pub range: f64,
}

/// The systems + conventional groups a county exposes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CountyContents {
    pub trs: Vec<CountyTrs>,
    pub subcats: Vec<CountySubcat>,
}

/// Parse `getCountyInfo` into its trunked systems + conventional subcategories.
pub fn parse_county(xml: &str) -> CountyContents {
    let Some(root) = Node::parse(xml) else {
        return CountyContents::default();
    };
    let trs = root
        .find("trsList")
        .map(|list| list.items().map(county_trs_from).collect())
        .unwrap_or_default();

    let mut subcats = Vec::new();
    if let Some(cats) = root.find("cats") {
        for cat in cats.items() {
            if let Some(scs) = cat.child("subcats") {
                for sc in scs.items() {
                    subcats.push(CountySubcat {
                        scid: sc.u64_child("scid").unwrap_or(0),
                        name: sc.str_child("scName"),
                        trunked_ref: sc.child("sids").is_some(),
                        lat: sc.f64_child("lat").unwrap_or(0.0),
                        lon: sc.f64_child("lon").unwrap_or(0.0),
                        range: sc.f64_child("range").unwrap_or(0.0),
                    });
                }
            }
        }
    }
    CountyContents { trs, subcats }
}

/// `getTrsDetails` → the core `Trs` header (type/flavor/voice names are resolved separately).
pub fn parse_trs_details(xml: &str) -> Option<Trs> {
    let r = return_node(xml)?;
    let county_ids = r
        .child("sCounty")
        .map(|a| a.items().filter_map(|it| it.u64_child("ctid")).collect())
        .unwrap_or_default();
    let state_ids = r
        .child("sState")
        .map(|a| a.items().filter_map(|it| it.u64_child("stid")).collect())
        .unwrap_or_default();
    Some(Trs {
        s_name: r.str_child("sName"),
        s_type: r.u64_child("sType").unwrap_or(0) as i32,
        s_flavor: r.u64_child("sFlavor").unwrap_or(0) as i32,
        s_voice: r.u64_child("sVoice").unwrap_or(0) as i32,
        type_name: None,
        flavor_name: None,
        county_ids,
        state_ids,
        lat: r.f64_child("lat").unwrap_or(0.0),
        lon: r.f64_child("lon").unwrap_or(0.0),
        range: r.f64_child("range").unwrap_or(0.0),
    })
}

/// `getTrsSites` → the sites (each with its control/voice frequencies).
pub fn parse_trs_sites(xml: &str) -> Vec<TrsSite> {
    parse_items(xml, |s| {
        let frequencies = s
            .child("siteFreqs")
            .map(|fs| {
                fs.items()
                    .filter_map(|f| {
                        Some(SiteFrequency {
                            freq_mhz: f.f64_child("freq")?,
                            use_: f.str_child("use"),
                            lcn: f.u64_child("lcn").map(|v| v as u32),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(TrsSite {
            site_id: s.u64_child("siteId").unwrap_or(0) as u32,
            site_number: s.u64_child("siteNumber").unwrap_or(0) as u32,
            site_descr: s.str_child("siteDescr"),
            lat: s.f64_child("lat").unwrap_or(0.0),
            lon: s.f64_child("lon").unwrap_or(0.0),
            range: s.f64_child("range").unwrap_or(0.0),
            nac: s.child_text("nac").map(|t| t.to_string()),
            frequencies,
        })
    })
}

/// `getTrsTalkgroups` → the talkgroups (tags reduced to their numeric service-type ids).
pub fn parse_talkgroups(xml: &str) -> Vec<Talkgroup> {
    parse_items(xml, |tg| {
        Some(Talkgroup {
            tg_dec: tg.str_child("tgDec"),
            tg_alpha: tg.str_child("tgAlpha"),
            tg_descr: tg.str_child("tgDescr"),
            tg_mode: tg.str_child("tgMode"),
            tags: parse_tags(tg),
        })
    })
}

/// `getSubcatFreqs` → the conventional frequencies of a subcategory.
pub fn parse_subcat_freqs(xml: &str) -> Vec<Freq> {
    parse_items(xml, |f| {
        Some(Freq {
            out_mhz: f.f64_child("out").unwrap_or(0.0),
            in_mhz: f.f64_child("in").unwrap_or(0.0),
            alpha: f.str_child("alpha"),
            descr: f.str_child("descr"),
            tone: f.str_child("tone"),
            mode: f.str_child("mode"),
            tags: parse_tags(f),
        })
    })
}

/// `getTrsType`/`getTrsFlavor`/`getTrsVoice` → the human name (`sTypeDescr` etc.).
pub fn parse_taxonomy_name(xml: &str, descr_field: &str) -> Option<String> {
    let root = return_node(xml)?;
    // Bind the owned result so the borrowing iterator drops before `root`.
    let name = root
        .items()
        .next()
        .map(|it| it.str_child(descr_field))
        .filter(|s| !s.is_empty());
    name
}

/// `getStateInfo` → the state's trunked systems (same `TrsListDef` shape as a county's `trsList`)
/// plus the ids of its counties and agencies (handles for further drill-down). `fetch_state` maps
/// the trunked list; the id lists let the UI expand down to a county or agency on demand.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StateContents {
    pub trs: Vec<CountyTrs>,
    pub county_ids: Vec<u64>,
    pub agency_ids: Vec<u64>,
}

pub fn parse_state(xml: &str) -> StateContents {
    let Some(root) = Node::parse(xml) else {
        return StateContents::default();
    };
    let trs = root
        .find("trsList")
        .map(|list| list.items().map(county_trs_from).collect())
        .unwrap_or_default();
    let county_ids = root
        .find("countyList")
        .map(|l| l.items().filter_map(|it| it.u64_child("ctid")).collect())
        .unwrap_or_default();
    let agency_ids = root
        .find("agencyList")
        .map(|l| l.items().filter_map(|it| it.u64_child("aid")).collect())
        .unwrap_or_default();
    StateContents {
        trs,
        county_ids,
        agency_ids,
    }
}

/// `getMode` → the display name for a modulation-mode id (RR sends `freq.mode` as a numeric id).
pub fn parse_mode_name(xml: &str) -> Option<String> {
    let root = return_node(xml)?;
    // Bind the owned result so the borrowing iterator drops before `root`.
    let name = root
        .items()
        .next()
        .map(|it| it.str_child("modeName"))
        .filter(|s| !s.is_empty());
    name
}

/// A talkgroup category (`getTrsTalkgroupCats`) — groups a system's talkgroups by `tgCid`. Most
/// categories are **geo-tagged** (county categories at a tight range, statewide services at a broad
/// one), which is what lets a location-first browse rank them by nearness. `lat`/`lon` = 0 means the
/// category is systemwide (no geo). Its talkgroups are fetched with `getTrsTalkgroups(sid, tgCid)`.
#[derive(Debug, Clone, PartialEq)]
pub struct TalkgroupCat {
    pub tg_cid: u64,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub range: f64,
    /// RR's `lastUpdated` as Unix epoch seconds (0 if absent) — the upstream freshness bound: this
    /// category's talkgroups are re-downloaded only when this advances past their cache time.
    pub last_updated: i64,
}

/// `getTrsTalkgroupCats` → a system's talkgroup categories (with geo for location-first ranking).
pub fn parse_talkgroup_cats(xml: &str) -> Vec<TalkgroupCat> {
    parse_items(xml, |c| {
        Some(TalkgroupCat {
            tg_cid: c.u64_child("tgCid")?,
            name: c.str_child("tgCname"),
            lat: c.f64_child("lat").unwrap_or(0.0),
            lon: c.f64_child("lon").unwrap_or(0.0),
            range: c.f64_child("range").unwrap_or(0.0),
            last_updated: parse_iso8601_epoch(&c.str_child("lastUpdated")).unwrap_or(0),
        })
    })
}

/// Parse RR's ISO-8601 `dateTime` (`2026-01-01T00:00:00+00:00`, or trailing `Z`) to Unix epoch
/// seconds. Hand-rolled to keep the crate dependency-light: RR emits a fixed
/// `YYYY-MM-DDThh:mm:ss` body followed by `Z` or `±hh:mm`.
pub fn parse_iso8601_epoch(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let min: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;
    let mut epoch = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + min * 60 + sec;
    // Trailing timezone offset (after the seconds): `±hh:mm` shifts to UTC; `Z`/absent = UTC.
    let tz = &s[19..];
    if tz.starts_with('+') || tz.starts_with('-') {
        let sign = if tz.starts_with('-') { -1 } else { 1 };
        let oh: i64 = tz.get(1..3)?.parse().ok()?;
        let om: i64 = tz.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(0);
        epoch -= sign * (oh * 3_600 + om * 60);
    }
    Some(epoch)
}

/// Days from the Unix epoch (1970-01-01) for a civil date — Howard Hinnant's algorithm (handles
/// leap years / Gregorian rules exactly, no tables).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ---------------------------------------------------------------------------
// Geo hierarchy — Country → State → County → Metro navigation lists.
// ---------------------------------------------------------------------------

/// A country RR covers (`getCountryList`) — the top of the geographic drill.
#[derive(Debug, Clone, PartialEq)]
pub struct Country {
    pub coid: u64,
    pub name: String,
    pub code: String,
}

pub fn parse_country_list(xml: &str) -> Vec<Country> {
    parse_items(xml, |c| {
        Some(Country {
            coid: c.u64_child("coid").unwrap_or(0),
            name: c.str_child("countryName"),
            code: c.str_child("countryCode"),
        })
    })
}

/// A state (`getStatesByList`, and `CountryInfo.stateList`).
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub stid: u64,
    pub name: String,
    pub code: String,
}

/// States from an array node (a `<return>` of `States`, or a nested `stateList`).
fn states_from(node: &Node) -> Vec<State> {
    node.items()
        .map(|s| State {
            stid: s.u64_child("stid").unwrap_or(0),
            name: s.str_child("stateName"),
            code: s.str_child("stateCode"),
        })
        .collect()
}

pub fn parse_states(xml: &str) -> Vec<State> {
    return_node(xml)
        .map(|r| states_from(&r))
        .unwrap_or_default()
}

/// A country's states + code (`getCountryInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct CountryInfo {
    pub coid: u64,
    pub name: String,
    pub code: String,
    pub states: Vec<State>,
}

pub fn parse_country_info(xml: &str) -> Option<CountryInfo> {
    let r = return_node(xml)?;
    Some(CountryInfo {
        coid: r.u64_child("coid").unwrap_or(0),
        name: r.str_child("countryName"),
        code: r.str_child("countryCode"),
        states: r.child("stateList").map(states_from).unwrap_or_default(),
    })
}

/// A county (`getCountiesByList` and `getMetroAreaInfo` — both return `Counties`).
#[derive(Debug, Clone, PartialEq)]
pub struct County {
    pub ctid: u64,
    pub name: String,
}

pub fn parse_counties(xml: &str) -> Vec<County> {
    parse_items(xml, |c| {
        Some(County {
            ctid: c.u64_child("ctid").unwrap_or(0),
            name: c.str_child("countyName"),
        })
    })
}

/// A metro area (`getMetroArea`).
#[derive(Debug, Clone, PartialEq)]
pub struct Metro {
    pub mid: u64,
    pub name: String,
}

pub fn parse_metros(xml: &str) -> Vec<Metro> {
    parse_items(xml, |m| {
        Some(Metro {
            mid: m.u64_child("mid").unwrap_or(0),
            name: m.str_child("metroName"),
        })
    })
}

// ---------------------------------------------------------------------------
// Reverse lookup & frequency search.
// ---------------------------------------------------------------------------

/// `getTrsBySysid` → the trunked systems matching a decoded on-air System ID, as `CountyTrs`
/// (`sid` is enough to drive `fetch_trunked`).
pub fn parse_trs_list(xml: &str) -> Vec<CountyTrs> {
    parse_items(xml, |n| Some(county_trs_from(n)))
}

/// A `searchCountyFreq`/`searchStateFreq`/`searchMetroFreq` hit — a conventional freq plus the refs
/// that identify what it belongs to (system `sid`, agency `aid`, subcat `scid`, county `ctid`).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub out_mhz: f64,
    pub in_mhz: f64,
    pub alpha: String,
    pub descr: String,
    pub tone: String,
    pub mode: String,
    pub callsign: String,
    pub sid: u64,
    pub aid: u64,
    pub scid: u64,
    pub ctid: u64,
    pub tags: Vec<Tag>,
}

pub fn parse_search_freqs(xml: &str) -> Vec<SearchResult> {
    parse_items(xml, |f| {
        Some(SearchResult {
            out_mhz: f.f64_child("out").unwrap_or(0.0),
            in_mhz: f.f64_child("in").unwrap_or(0.0),
            alpha: f.str_child("alpha"),
            descr: f.str_child("descr"),
            tone: f.str_child("tone"),
            mode: f.str_child("mode"),
            callsign: f.str_child("callsign"),
            sid: f.u64_child("sid").unwrap_or(0),
            aid: f.u64_child("aid").unwrap_or(0),
            scid: f.u64_child("scid").unwrap_or(0),
            ctid: f.u64_child("ctid").unwrap_or(0),
            tags: parse_tags(f),
        })
    })
}

// ---------------------------------------------------------------------------
// FCC ULS database.
// ---------------------------------------------------------------------------

/// An FCC license (`fccGetCallsign`) — its holder, status, and licensed frequencies (MHz).
#[derive(Debug, Clone, PartialEq)]
pub struct FccCallsign {
    pub callsign: String,
    pub licensee: String,
    pub status: String,
    pub grant_date: String,
    pub radio_service: String,
    pub frequencies: Vec<f64>,
}

pub fn parse_fcc_callsign(xml: &str) -> Option<FccCallsign> {
    let r = return_node(xml)?;
    let frequencies = r
        .child("frequencies")
        .map(|fs| {
            fs.items()
                .filter_map(|f| f.f64_child("frequency"))
                .collect()
        })
        .unwrap_or_default();
    Some(FccCallsign {
        callsign: r.str_child("callsign"),
        licensee: r.str_child("licensee"),
        status: r.str_child("status"),
        grant_date: r.str_child("grantDate"),
        radio_service: r.str_child("radioService"),
        frequencies,
    })
}

/// An FCC radio-service code (`fccGetRadioServiceCode`).
#[derive(Debug, Clone, PartialEq)]
pub struct FccServiceCode {
    pub code: String,
    pub description: String,
}

pub fn parse_fcc_service_codes(xml: &str) -> Vec<FccServiceCode> {
    parse_items(xml, |c| {
        Some(FccServiceCode {
            code: c.str_child("code"),
            description: c.str_child("description"),
        })
    })
}

/// An FCC license near a query point (`fccGetProxCallsigns`) — with its distance from the point.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxCallsign {
    pub callsign: String,
    pub licensee: String,
    pub lat: f64,
    pub lon: f64,
    pub distance: f64,
}

pub fn parse_prox_callsigns(xml: &str) -> Vec<ProxCallsign> {
    parse_items(xml, |c| {
        Some(ProxCallsign {
            callsign: c.str_child("callsign"),
            licensee: c.str_child("licensee"),
            lat: c.f64_child("lat").unwrap_or(0.0),
            lon: c.f64_child("lon").unwrap_or(0.0),
            distance: c.f64_child("distance").unwrap_or(0.0),
        })
    })
}

// ---------------------------------------------------------------------------
// Account diagnostics.
// ---------------------------------------------------------------------------

/// The authenticated user's account (`getUserData`). The subscription date is kept as RR's raw
/// string (zero-dep — the caller/UI interprets it).
#[derive(Debug, Clone, PartialEq)]
pub struct UserInfo {
    pub username: String,
    pub sub_expire_date: String,
}

pub fn parse_user_data(xml: &str) -> Option<UserInfo> {
    let r = return_node(xml)?;
    Some(UserInfo {
        username: r.str_child("username"),
        sub_expire_date: r.str_child("subExpireDate"),
    })
}

/// A user's Broadcastify feed broadcast (`getUserFeedBroadcasts`). The feed **password is
/// deliberately not parsed** — a secret we have no use for, so it can't leak through this struct.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedBroadcast {
    pub feed_id: u64,
    pub descr: String,
    pub hostname: String,
    pub port: String,
    pub mount: String,
}

pub fn parse_feed_broadcasts(xml: &str) -> Vec<FeedBroadcast> {
    parse_items(xml, |b| {
        Some(FeedBroadcast {
            feed_id: b.u64_child("feedId").unwrap_or(0),
            descr: b.str_child("descr"),
            hostname: b.str_child("hostname"),
            port: b.str_child("port"),
            mount: b.str_child("mount"),
        })
    })
}

/// The `<tags>` array → core `Tag`s (RR sends only the numeric `tagId`; the name is resolved from
/// the core's service-type table downstream, so `tag_descr` stays empty here).
fn parse_tags(parent: &Node) -> Vec<Tag> {
    parent
        .child("tags")
        .map(|tags| {
            tags.items()
                .filter_map(|t| {
                    Some(Tag {
                        tag_id: t.u64_child("tagId")? as u16,
                        tag_descr: String::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_to_epoch_utc_and_offsets() {
        // 2026-01-01T00:00:00Z = 1767225600 (verified against a known epoch).
        assert_eq!(
            parse_iso8601_epoch("2026-01-01T00:00:00+00:00"),
            Some(1_767_225_600)
        );
        assert_eq!(
            parse_iso8601_epoch("2026-01-01T00:00:00Z"),
            Some(1_767_225_600)
        );
        // A +05:30 offset is earlier in UTC by 5h30m.
        assert_eq!(
            parse_iso8601_epoch("2026-01-01T05:30:00+05:30"),
            Some(1_767_225_600)
        );
        // Leap day parses (2024 is a leap year).
        assert_eq!(
            parse_iso8601_epoch("2024-02-29T00:00:00Z"),
            Some(1_709_164_800)
        );
        // Garbage / too-short → None.
        assert_eq!(parse_iso8601_epoch("nope"), None);
        assert_eq!(parse_iso8601_epoch(""), None);
    }

    #[test]
    fn talkgroup_cats_carry_last_updated() {
        let xml = include_str!("../tests/fixtures/tg_cats.xml");
        let cats = parse_talkgroup_cats(xml);
        assert!(!cats.is_empty());
        // The fixture stamps every category 2026-01-01T00:00:00+00:00.
        assert_eq!(cats[0].last_updated, 1_767_225_600);
    }
}
