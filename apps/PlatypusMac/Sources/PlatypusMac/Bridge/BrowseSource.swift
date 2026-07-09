// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CoreLocation
import Foundation

/// What a browse source can do — drives capability gating in the UI, never a source-type `if`.
struct SourceCapabilities: OptionSet {
    let rawValue: Int
    /// Can provide a Country → State → County hierarchy to pick a location.
    static let geoHierarchy = SourceCapabilities(rawValue: 1 << 0)
    /// Supports "add every system near here" (radius-scoped add) on the map.
    static let radiusAdd = SourceCapabilities(rawValue: 1 << 1)
    /// Its channels can be programmed onto a radio (added to a favorites list / clone image).
    static let addToRadio = SourceCapabilities(rawValue: 1 << 2)
}

/// The universal query: a place every source answers ("what do you have here?"). A ZIP is carried for
/// sources that query by ZIP (RadioReference); the coordinate + radius serve geo-scoped sources (HPDB).
struct BrowseLocation: Equatable {
    let label: String
    let coordinate: CLLocationCoordinate2D
    var radiusMi: Double = 25
    let zip: String?

    static func == (a: BrowseLocation, b: BrowseLocation) -> Bool {
        a.label == b.label && a.zip == b.zip && a.radiusMi == b.radiusMi
            && a.coordinate.latitude == b.coordinate.latitude
            && a.coordinate.longitude == b.coordinate.longitude
    }
}

/// A talkgroup/service category a system groups its channels into (RadioReference trunked systems);
/// sources without a category level return nil from `categories`. `local`/`systemwide` rank it for a
/// location-first drill. (Generalizes the former `RrCategory`.)
typealias BrowseCategory = RrCategory

/// A node in a source's geographic picker (country/state/county), used to set the location.
struct GeoNode: Identifiable, Hashable {
    let id: UInt64
    let name: String
    let coordinate: CLLocationCoordinate2D?

    static func == (a: GeoNode, b: GeoNode) -> Bool { a.id == b.id && a.name == b.name }
    func hash(into h: inout Hasher) { h.combine(id); h.combine(name) }
}

/// A universal browse data source. The app treats every source through this protocol — no source-type
/// branching. Each source answers a location, exposes what it has, and degrades consistently. Adding a
/// source (e.g. RepeaterBook) means a new conformer + a `DataSourceKind` case, with no UI change.
///
/// All item ids are **source-local** here; the aggregator namespaces them `"<source>:<id>"` and tags
/// each `CatalogSystem`/`CatalogChannel`/`GeoSystem` with its `source`, so a per-item action routes
/// back to the owning source.
protocol BrowseSource: AnyObject {
    var sourceKind: DataSourceKind { get }
    var capabilities: SourceCapabilities { get }

    /// Systems this source has at a location (filter-applied). Networked sources fetch; static sources
    /// scope their loaded data. Run off the main thread.
    func systems(at location: BrowseLocation, _ filter: FilterState) -> [CatalogSystem]

    /// A system's channels directly (no category level), for conventional systems / sources.
    func channels(system: String, _ filter: FilterState) -> [CatalogChannel]

    /// A system's channel categories, ranked local-first — or nil when the system/source has no
    /// category level (then the UI shows `channels` directly).
    func categories(system: String, includeAll: Bool) -> [BrowseCategory]?

    /// One category's channels.
    func categoryChannels(system: String, category: Int, _ filter: FilterState) -> [CatalogChannel]

    /// This source's systems at a location as map pins.
    func geo(at location: BrowseLocation, _ filter: FilterState) -> [GeoSystem]

    /// A map pin's most-local channels for the info popover / "add near here" — radius-scoped for
    /// sources that support it (HPDB), else the system's best-local set. Default: `channels`.
    func radiusChannels(system: String, lat: Double, lon: Double, miles: Double, _ filter: FilterState)
        -> [CatalogChannel]

    /// When this source's data for the current location was last fetched — for networked/cached
    /// sources (RadioReference). `nil` for sources with no cache concept (a loaded local file is
    /// always "current"). Drives the "as of <date>" caption + whether a Refresh control shows.
    var lastFetched: Date? { get }

    /// Force-refresh this source's cached data for the current location (networked sources re-fetch
    /// live). Default: no-op (local sources are already current). Call off the main thread.
    func refreshCache()

    /// Warm the next `batch` map pins' precise positions for the current location (networked sources
    /// fetch that many sites, concurrently). Returns how many were warmed (0 when done / nothing to
    /// warm). The map loops this in the background so pins refine in bursts without blocking. Default:
    /// 0 — local sources already place every pin. Call off the main thread.
    func warmNextPins(at location: BrowseLocation, batch: Int) -> Int
}

extension BrowseSource {
    func radiusChannels(system: String, lat: Double, lon: Double, miles: Double, _ filter: FilterState)
        -> [CatalogChannel]
    {
        channels(system: system, filter)
    }

    var lastFetched: Date? { nil }
    func refreshCache() {}
    func warmNextPins(at location: BrowseLocation, batch: Int) -> Int { 0 }
}
