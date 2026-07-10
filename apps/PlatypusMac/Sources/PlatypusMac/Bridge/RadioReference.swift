// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import Foundation

/// The RadioReference **app key** — identifies our app to RR (not a per-user secret). Build-injected
/// so it ships with the app but never lives in git: `just app::bundle` stamps `RRAppKey` into the
/// bundle's Info.plist from `$RR_APP_KEY`; `swift run` dev builds read the env var directly.
enum RadioReferenceKey {
    static var current: String? {
        if let k = Bundle.main.object(forInfoDictionaryKey: "RRAppKey") as? String, !k.isEmpty {
            return k
        }
        if let k = ProcessInfo.processInfo.environment["RR_APP_KEY"], !k.isEmpty {
            return k
        }
        return nil
    }

    /// Whether the app key is available at all (gates the RadioReference source in the UI).
    static var isConfigured: Bool { current != nil }
}

/// The account details RR returns from a credential check.
struct RadioReferenceAccount: Codable {
    let username: String
    let subExpireDate: String
}

/// The resolved location for an open browse session — drives the location chip.
struct RrLocation: Codable {
    let city: String
    let ctid: UInt64
    let stid: UInt64
    let lat: Double
    let lon: Double
    let systemCount: Int
    /// When the location's base data (`getCountyInfo`) was last fetched, Unix epoch seconds (0 =
    /// unknown) — the "as of <date>" anchor.
    var fetchedAt: Double = 0

    /// The fetch time as a `Date`, or nil when unknown.
    var fetchedDate: Date? { fetchedAt > 0 ? Date(timeIntervalSince1970: fetchedAt) : nil }
}

/// A trunked system's talkgroup category, ranked for the location-first drill. `local` = a nearby
/// county-level category; `systemwide` = broad/shared (statewide interop, mutual aid).
struct RrCategory: Codable, Identifiable {
    let id: Int
    let name: String
    let distanceMi: Double?
    let local: Bool
    let systemwide: Bool
}

/// A RadioReference failure carrying the user-facing message (the RR fault / transport text).
struct RadioReferenceError: LocalizedError {
    let message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
}

/// Boxes the progress + cancel closures so they can cross the C ABI via one opaque context.
private final class RrCallbacks {
    let progress: ((UInt32, UInt32, UInt32) -> Void)?
    let cancel: (() -> Bool)?
    init(progress: ((UInt32, UInt32, UInt32) -> Void)?, cancel: (() -> Bool)?) {
        self.progress = progress
        self.cancel = cancel
    }
}

private func rrProgressTrampoline(
    _ ctx: UnsafeMutableRawPointer?, _ phase: UInt32, _ done: UInt32, _ total: UInt32
) {
    guard let ctx else { return }
    Unmanaged<RrCallbacks>.fromOpaque(ctx).takeUnretainedValue().progress?(phase, done, total)
}

private func rrCancelTrampoline(_ ctx: UnsafeMutableRawPointer?) -> UInt8 {
    guard let ctx else { return 0 }
    let cancelled = Unmanaged<RrCallbacks>.fromOpaque(ctx).takeUnretainedValue().cancel?() ?? false
    return cancelled ? 1 : 0
}

/// Reads an FFI `err_out` string (if set) and frees it.
private func takeError(_ err: UnsafeMutablePointer<CChar>?) -> String? {
    guard let err else { return nil }
    defer { platypus_string_free(err) }
    return String(cString: err)
}

/// Safe Swift wrapper over the RadioReference source FFI. Owns the Rust handle; the throttled/cached
/// fetch runs in Rust over native OS TLS. Emits the *same* `CatalogSystem`/`CatalogChannel`/
/// `GeoSystem` models as `ScannerLibrary`, so the catalog + map render RR and card data identically.
final class RadioReferenceSource {
    let handle: OpaquePointer
    /// Serializes handle access: the FFI mutates on-disk-cache-backed maps behind the opaque
    /// pointer, and the app calls these methods from several background queues (drills, map geo) —
    /// concurrent access would race. Callers are already off-main, so `sync` never blocks the UI.
    private let queue = DispatchQueue(label: "com.platypus.rr.source")

    private init(handle: OpaquePointer) { self.handle = handle }
    deinit { platypus_rr_source_free(handle) }

    /// Live credential check (`getUserData`). `.success` carries the account; `.failure` the RR
    /// message (e.g. "Invalid Username or Password").
    static func validate(appKey: String, username: String, password: String)
        -> Result<RadioReferenceAccount, RadioReferenceError>
    {
        var err: UnsafeMutablePointer<CChar>?
        let ptr = appKey.withCString { k in
            username.withCString { u in
                password.withCString { p in
                    platypus_rr_validate(k, u, p, &err)
                }
            }
        }
        if let ptr {
            defer { platypus_string_free(ptr) }
            let data = Data(bytes: ptr, count: strlen(ptr))
            if let acct = try? JSONDecoder().decode(RadioReferenceAccount.self, from: data) {
                return .success(acct)
            }
            return .failure(RadioReferenceError("unexpected response from RadioReference"))
        }
        return .failure(RadioReferenceError(takeError(err) ?? "validation failed"))
    }

    /// Open a browse session for a `selector` (e.g. `"zip:97201"`). Synchronous + networked — run it
    /// off the main thread. `progress` is `(phase, done, total)`; returning `true` from `cancel`
    /// aborts. `.failure` carries the RR/transport message.
    static func open(
        appKey: String,
        username: String,
        password: String,
        selector: String,
        progress: ((UInt32, UInt32, UInt32) -> Void)? = nil,
        cancel: (() -> Bool)? = nil
    ) -> Result<RadioReferenceSource, RadioReferenceError> {
        var err: UnsafeMutablePointer<CChar>?
        let handle: OpaquePointer? = appKey.withCString { k in
            username.withCString { u in
                password.withCString { p in
                    selector.withCString { sel in
                        if progress == nil && cancel == nil {
                            return platypus_rr_source_open(k, u, p, sel, nil, nil, nil, &err)
                        }
                        let box = Unmanaged.passRetained(
                            RrCallbacks(progress: progress, cancel: cancel))
                        defer { box.release() }
                        return platypus_rr_source_open(
                            k, u, p, sel, box.toOpaque(),
                            rrProgressTrampoline, rrCancelTrampoline, &err)
                    }
                }
            }
        }
        if let handle { return .success(RadioReferenceSource(handle: handle)) }
        return .failure(RadioReferenceError(takeError(err) ?? "could not fetch that location"))
    }

    /// The location's systems as catalog rows (only the `search` filter applies here; service/tech
    /// filtering happens once a system's channels are drilled).
    func systems(_ f: FilterState) -> [CatalogSystem] {
        queue.sync {
        let ptr = f.servicesCSV.withCString { svc in
            f.techsCSV.withCString { tech in
                f.search.withCString { search in
                    platypus_rr_source_systems_json(handle, svc, tech, search)
                }
            }
        }
        return FFI.decode(ptr)
        }
    }

    /// Synthesize this system's SDS150 favorites records (from the fetched RR sites/talkgroups) and
    /// merge them into `fav`, returning a new handle. Networked; the caller is already off-main.
    func appendToFavorites(_ fav: Favorites, systemRef: String, departmentsOn: Bool) -> Favorites? {
        queue.sync {
            let h = systemRef.withCString {
                platypus_favorites_append_from_rr(fav.handle, handle, $0, departmentsOn)
            }
            return Favorites(handle: h)
        }
    }

    func ft60Channels(systemRef: String) -> [FT60Channel] {
        queue.sync {
            systemRef.withCString {
                FT60Channel.decode(fromJSON: platypus_ft60_channels_from_rr(handle, $0))
            }
        }
    }

    /// The location's systems as map pins (drilled systems at their real site, others at the county
    /// centroid). `lat`/`lon`/`miles` are accepted for parity with the library but not used to prune.
    func geo(lat: Double, lon: Double, miles: Double, _ f: FilterState) -> [GeoSystem] {
        queue.sync {
        let ptr = f.servicesCSV.withCString { svc in
            f.techsCSV.withCString { tech in
                f.search.withCString { search in
                    platypus_rr_source_geo_json(handle, lat, lon, miles, svc, tech, search)
                }
            }
        }
        return FFI.decode(ptr)
        }
    }

    /// The resolved location (city / ids / centroid / system count / fetchedAt) for the location chip.
    func location() -> RrLocation? {
        queue.sync { FFI.decodeOne(platypus_rr_source_location_json(handle)) }
    }

    /// Warm the next `n` un-warmed trunked systems' real map sites, fetched **concurrently** in Rust
    /// (forked clients). Returns how many were warmed (0 when done). Networked; run off the main
    /// thread — the app loops this + reloads the map so pins spread in bursts.
    func warmBatch(_ n: Int) -> Int {
        queue.sync { Int(platypus_rr_source_warm_batch(handle, UInt32(max(1, n)))) }
    }

    /// Force-refresh the location's base data live (bypassing the cache), returning the updated
    /// location (with a new `fetchedAt`). Networked; run off the main thread.
    func refresh() -> RrLocation? {
        queue.sync { FFI.decodeOne(platypus_rr_source_refresh(handle)) }
    }

    /// A trunked system's talkgroup categories, ranked local-first. `includeAll` reveals the
    /// out-of-area categories ("show all areas"). Networked on first call; run off the main thread.
    func categories(systemRef: String, includeAll: Bool) -> [RrCategory] {
        queue.sync {
        let ptr = systemRef.withCString {
            platypus_rr_source_categories_json(handle, $0, includeAll ? 1 : 0)
        }
        return FFI.decode(ptr)
        }
    }

    /// A talkgroup category's channels — fetched lazily + cached. Networked on first call.
    func categoryChannels(systemRef: String, categoryId: Int, _ f: FilterState) -> [CatalogChannel] {
        queue.sync {
        let ptr = systemRef.withCString { sid in
            f.servicesCSV.withCString { svc in
                f.search.withCString { search in
                    platypus_rr_source_category_channels_json(
                        handle, sid, UInt32(categoryId), svc, search)
                }
            }
        }
        return FFI.decode(ptr)
        }
    }

    /// One system's channels — fetched lazily on first call and cached in Rust. `systemRef` is a
    /// row `id`. Networked on the first drill; run off the main thread.
    func channels(systemRef: String, _ f: FilterState) -> [CatalogChannel] {
        queue.sync {
        let ptr = systemRef.withCString { sid in
            f.servicesCSV.withCString { svc in
                f.search.withCString { search in
                    platypus_rr_source_channels_json(handle, sid, svc, search)
                }
            }
        }
        return FFI.decode(ptr)
        }
    }

}

/// RadioReference as a universal browse source. It answers a location by opening a session for that
/// ZIP (cached), exposing the location's systems + the trunked category drill. Browse-only for now
/// (no `.addToRadio`). This adapts the per-location `RadioReferenceSource` sessions to the location-
/// agnostic `BrowseSource` protocol so RR merges with every other source.
final class RadioReferenceBrowseSource: BrowseSource {
    var sourceKind: DataSourceKind { .radioReference }
    var capabilities: SourceCapabilities { [.geoHierarchy] }

    private let appKey: String
    private let creds: RadioReferenceCredentials
    private var session: RadioReferenceSource?
    private var sessionZip: String?
    /// The current session's resolved location, cached so the "as of" caption reads without a
    /// per-render FFI call. Set on open/refresh.
    private var lastLocation: RrLocation?
    /// Serializes `session(for:)`: on a new location the list rows, the map's geo, and the pin-warm
    /// loop all reach for a session at once — without this each would open its own (a big county would
    /// fetch `getCountyInfo` several times over). The first caller opens; the rest wait and reuse it.
    private let sessionLock = NSLock()

    /// Fails to build if the app key or the user's login isn't available (source can't be active).
    init?() {
        guard let appKey = RadioReferenceKey.current, let creds = RadioReferenceKeychain.load()
        else { return nil }
        self.appKey = appKey
        self.creds = creds
    }

    /// Ensure a session for this location's ZIP (opening if the location changed). Networked; runs on
    /// the caller's (background) thread.
    private func session(for location: BrowseLocation) -> RadioReferenceSource? {
        guard let zip = location.zip, !zip.isEmpty else { return nil }
        sessionLock.lock()
        defer { sessionLock.unlock() }
        // Double-checked under the lock: a concurrent caller may have already opened this ZIP.
        if sessionZip == zip, let session { return session }
        switch RadioReferenceSource.open(
            appKey: appKey, username: creds.username, password: creds.password, selector: "zip:\(zip)")
        {
        case .success(let s):
            session = s
            sessionZip = zip
            lastLocation = s.location()
            return s
        case .failure:
            session = nil
            sessionZip = nil
            lastLocation = nil
            return nil
        }
    }

    /// The resolved location metadata for the current session (for the chip's system count).
    var resolvedLocation: RrLocation? { lastLocation }

    // Freshness (BrowseSource): RR is networked/cached, so it reports a fetch time + can refresh.
    var lastFetched: Date? { lastLocation?.fetchedDate }

    func refreshCache() {
        if let updated = session?.refresh() { lastLocation = updated }
    }

    func warmNextPins(at location: BrowseLocation, batch: Int) -> Int {
        // Warm only the session that's already open **for this location** — never open here. Otherwise
        // a superseded warm loop (from a previous ZIP) would re-open the old session and flap it with
        // the current one. `systems(at:)` / `geo(at:)` remain the sole openers.
        let s: RadioReferenceSource? = {
            sessionLock.lock()
            defer { sessionLock.unlock() }
            guard let zip = location.zip, sessionZip == zip else { return nil }
            return session
        }()
        return s?.warmBatch(batch) ?? 0
    }

    func systems(at location: BrowseLocation, _ filter: FilterState) -> [CatalogSystem] {
        session(for: location)?.systems(filter) ?? []
    }

    func channels(system: String, _ filter: FilterState) -> [CatalogChannel] {
        session?.channels(systemRef: system, filter) ?? []
    }

    /// Synthesize the given system's favorites onto an SD card via the currently-open session (the
    /// browsed location's). Networked — call off the main thread.
    func appendToFavorites(_ fav: Favorites, systemRef: String, departmentsOn: Bool) -> Favorites? {
        let s: RadioReferenceSource? = {
            sessionLock.lock()
            defer { sessionLock.unlock() }
            return session
        }()
        return s?.appendToFavorites(fav, systemRef: systemRef, departmentsOn: departmentsOn)
    }

    func categories(system: String, includeAll: Bool) -> [BrowseCategory]? {
        guard system.hasPrefix("t") else { return nil }  // only trunked systems group into categories
        return session?.categories(systemRef: system, includeAll: includeAll)
    }

    func categoryChannels(system: String, category: Int, _ filter: FilterState) -> [CatalogChannel] {
        session?.categoryChannels(systemRef: system, categoryId: category, filter) ?? []
    }

    func geo(at location: BrowseLocation, _ filter: FilterState) -> [GeoSystem] {
        session(for: location)?.geo(
            lat: location.coordinate.latitude, lon: location.coordinate.longitude,
            miles: location.radiusMi, filter) ?? []
    }

    /// A pin's most-local channels: a trunked system's nearest (ZIP-ranked) talkgroup category, or a
    /// conventional system's channels directly.
    func radiusChannels(system: String, lat: Double, lon: Double, miles: Double, _ filter: FilterState)
        -> [CatalogChannel]
    {
        guard let session else { return [] }
        if system.hasPrefix("t") {
            let cats = session.categories(systemRef: system, includeAll: false)
            guard let nearest = cats.min(by: {
                ($0.distanceMi ?? .infinity) < ($1.distanceMi ?? .infinity)
            }) else { return [] }
            return session.categoryChannels(systemRef: system, categoryId: nearest.id, filter)
        }
        return session.channels(systemRef: system, filter)
    }
}
