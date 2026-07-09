// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import Combine
import CoreLocation
import Foundation

/// The browse aggregator: the set of **active** `BrowseSource`s, merged into one location-first
/// list + map. It answers the current `location` by asking every active source "what do you have
/// here?", tags each row/pin with its `source`, merges + sorts, and routes a per-item drill back to
/// the owning source by that tag. This is the one object the browse UI talks to — the UI never
/// branches on a source kind (that lives in each source's `capabilities`).
final class BrowseSources: ObservableObject {
    /// The merged system rows for the current location + filter (each tagged with its `source`).
    @Published private(set) var rows: [CatalogSystem] = []
    /// A merge is in flight (any source still fetching) — drives the browser spinner.
    @Published private(set) var loading = false
    /// The current browse location. Setting it does not fetch; call `refresh` to (re)query.
    @Published var location: BrowseLocation?

    /// Live, registered source instances keyed by kind (HPDB's `ScannerLibrary`, the RR wrapper…).
    private var registry: [DataSourceKind: BrowseSource] = [:]
    /// Which registered sources are enabled (merged into browse). Several may be on at once.
    private var enabled: Set<DataSourceKind> = []
    /// Serial background queue for the (networked) merge, so the UI never blocks.
    private let work = DispatchQueue(label: "com.platypus.browse.merge")
    /// Monotonic token so a stale merge (location/filter changed mid-flight) is discarded.
    private var generation = 0

    /// The active (enabled + registered) sources, in a stable kind order.
    var active: [BrowseSource] {
        DataSourceKind.allCases.filter { enabled.contains($0) }.compactMap { registry[$0] }
    }
    var activeKinds: [DataSourceKind] { active.map(\.sourceKind) }
    var activeCount: Int { active.count }
    /// The union of every active source's capabilities (for gating an action available on *any* source).
    var capabilities: SourceCapabilities {
        active.reduce(into: SourceCapabilities()) { $0.formUnion($1.capabilities) }
    }

    /// Register (or clear, with nil) a source's live instance. Enabling is separate (`setEnabled`).
    func register(_ source: BrowseSource?, for kind: DataSourceKind) {
        if let source { registry[kind] = source } else {
            registry.removeValue(forKey: kind)
            enabled.remove(kind)
        }
    }

    func isRegistered(_ kind: DataSourceKind) -> Bool { registry[kind] != nil }
    func isEnabled(_ kind: DataSourceKind) -> Bool { enabled.contains(kind) }
    /// A registered source's capabilities (empty if not registered) — for per-source action gating.
    func capabilities(of kind: DataSourceKind) -> SourceCapabilities { registry[kind]?.capabilities ?? [] }

    /// Whether any active source has a refreshable cache (networked sources) — shows the Refresh
    /// control + "as of" caption.
    var hasRefreshable: Bool { active.contains { $0.lastFetched != nil } }
    /// The oldest "as of" time across active cache-backed sources (the conservative freshness),
    /// or nil if none report one.
    var lastFetched: Date? { active.compactMap(\.lastFetched).min() }
    /// Force-refresh every active source's cache for the current location. Networked — call off-main.
    func refreshCaches() { active.forEach { $0.refreshCache() } }

    /// Warm the next `batch` map pins across the active sources (progressive map refinement). Returns
    /// how many were warmed (0 when all are done). Networked — call off-main.
    func warmNextPins(at location: BrowseLocation, batch: Int) -> Int {
        active.reduce(0) { $0 + $1.warmNextPins(at: location, batch: batch) }
    }

    /// Enable/disable a source in the merge (no-op if not registered).
    func setEnabled(_ kind: DataSourceKind, _ on: Bool) {
        guard registry[kind] != nil else { return }
        if on { enabled.insert(kind) } else { enabled.remove(kind) }
    }

    /// Re-query every active source at the current location and republish `rows`. Runs the
    /// (possibly networked) fetch off the main thread; a newer call supersedes an in-flight one.
    ///
    /// Only the **list** rows are fetched here — deliberately *not* the map pins. Pins come from the
    /// Map lens's own geo query when it's shown, so searching a big county (e.g. 90210 / LA) doesn't
    /// pay the per-system `getTrsSites` fetches (throttled) up front for a lens you may never open.
    func refresh(_ filter: FilterState) {
        guard let location, !active.isEmpty else {
            rows = []
            loading = false
            return
        }
        generation += 1
        let gen = generation
        let sources = active
        loading = true
        work.async { [weak self] in
            var mergedRows: [CatalogSystem] = []
            for src in sources {
                let kind = src.sourceKind
                for var s in src.systems(at: location, filter) {
                    s.source = kind
                    mergedRows.append(s)
                }
            }
            // Stable source-then-name order; per-source local relevance is already applied.
            mergedRows.sort {
                if $0.source != $1.source {
                    return kindOrder($0.source) < kindOrder($1.source)
                }
                return $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
            }
            DispatchQueue.main.async {
                guard let self, gen == self.generation else { return }
                self.rows = mergedRows
                self.loading = false
            }
        }
    }

    // MARK: - Per-item routing (by the row's `source` tag)

    /// A system's channels directly (conventional / no-category sources).
    func channels(for system: CatalogSystem, _ filter: FilterState) -> [CatalogChannel] {
        (registry[system.source]?.channels(system: system.id, filter) ?? [])
            .map { var c = $0; c.source = system.source; return c }
    }

    /// A system's categories (nil ⇒ no category level; show channels directly).
    func categories(for system: CatalogSystem, includeAll: Bool) -> [BrowseCategory]? {
        registry[system.source]?.categories(system: system.id, includeAll: includeAll)
    }

    /// One category's channels under a system.
    func categoryChannels(for system: CatalogSystem, category: Int, _ filter: FilterState)
        -> [CatalogChannel]
    {
        (registry[system.source]?.categoryChannels(system: system.id, category: category, filter) ?? [])
            .map { var c = $0; c.source = system.source; return c }
    }

    /// A map pin's channels for the info popover / add — radius-scoped by the owning source (HPDB),
    /// else its best-local set. Routed by the pin's `source` tag.
    func channelsForPin(_ id: String, source: DataSourceKind, lat: Double, lon: Double, miles: Double,
                        _ filter: FilterState) -> [CatalogChannel]
    {
        (registry[source]?.radiusChannels(system: id, lat: lat, lon: lon, miles: miles, filter) ?? [])
            .map { var c = $0; c.source = source; return c }
    }
}

/// Stable ordering of sources in the merged list (HPDB first as the offline/base source).
/// Internal (not private) so the routing tests can assert the order.
func kindOrder(_ kind: DataSourceKind) -> Int {
    switch kind {
    case .hpdb: return 0
    case .radioReference: return 1
    case .repeaterBook: return 2
    }
}
