// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import Foundation

/// One system row in the catalog (system-level; channels load lazily).
struct CatalogSystem: Codable, Identifiable {
    let id: String
    let name: String
    let kind: String
    let tech: String?
    /// Counties this system *covers* (location-first placement), not just its tags.
    let counties: [UInt64]
    /// AreaState ids — the Country → State level of the hierarchy.
    let states: [UInt64]
    /// True if the system has channels with no county (state-level fallback).
    let statewide: Bool
    let siteCount: Int
    let channelCount: Int

    var isTrunk: Bool { kind == "Trunk" }
    var multiCounty: Bool { counties.count > 1 || statewide }
}

/// One channel under a system (talkgroup or conventional frequency).
struct CatalogChannel: Codable, Identifiable {
    let id: String
    let name: String
    let kind: String
    let tgid: String?
    let freqHz: UInt64?
    let mode: String?
    let serviceType: Int?
    /// Raw audio-option field (CTCSS/DCS tone, P25 NAC, DMR color code…), or nil.
    var tone: String?

    var isTalkgroup: Bool { kind == "Talkgroup" }

    /// The audio-option rendered inline, e.g. "CTCSS 156.7 Hz" / "NAC 293".
    var toneInline: String? { AudioOption.inline(tone) }

    /// The trailing right-aligned primary identifier: a talkgroup's TGID, else the
    /// tuned frequency in MHz.
    var detail: String {
        if let tgid { return "TG \(tgid)" }
        if let freqHz { return String(format: "%.4f MHz", Double(freqHz) / 1_000_000) }
        return ""
    }

    /// Frequency in MHz (e.g. "154.2650 MHz"), if this channel has one.
    var freqMHz: String? {
        freqHz.map { String(format: "%.4f MHz", Double($0) / 1_000_000) }
    }
}

/// Aggregate stats for the load summary.
struct LibraryStats: Codable {
    let files: Int
    let systems: Int
    let channels: Int
}

/// A geo-located system for the map: a pin at (lat,lon) with a coverage radius.
struct GeoSystem: Codable, Identifiable {
    let id: String
    let name: String
    let kind: String
    let tech: String?
    let serviceType: Int?
    let lat: Double
    let lon: Double
    let rangeMi: Double
}

/// The catalog filter (mirrors the 1B sidebar). Encoded into the FFI's CSV params.
struct FilterState: Equatable {
    var services: Set<Int> = []
    var techs: Set<String> = []
    var search = ""

    var servicesCSV: String { services.sorted().map(String.init).joined(separator: ",") }
    var techsCSV: String { techs.sorted().joined(separator: ",") }

    var isEmpty: Bool { services.isEmpty && techs.isEmpty && search.isEmpty }
}

/// Boxes a Swift progress closure so it can cross the C ABI via an opaque context.
private final class ProgressBox {
    let cb: (UInt32, UInt32, UInt32) -> Void
    init(_ cb: @escaping (UInt32, UInt32, UInt32) -> Void) { self.cb = cb }
}

/// Top-level (capture-free) trampoline → convertible to a C function pointer.
private func progressTrampoline(
    _ ctx: UnsafeMutableRawPointer?, _ phase: UInt32, _ done: UInt32, _ total: UInt32
) {
    guard let ctx else { return }
    Unmanaged<ProgressBox>.fromOpaque(ctx).takeUnretainedValue().cb(phase, done, total)
}

/// Safe Swift wrapper over the full-USA library FFI. Owns the Rust handle; all
/// heavy work (parse, filter) happens in the core.
final class ScannerLibrary {
    let handle: OpaquePointer?

    /// Open every `s_*.hpd` in a directory into one in-memory model. `progress`, if
    /// given, is called as `(phase, done, total)` during the load (phase 1 = reading
    /// files, 2 = indexing coverage). The load is synchronous — run it off the main
    /// thread and hop `progress` to main.
    init?(directory: String, progress: ((UInt32, UInt32, UInt32) -> Void)? = nil) {
        if let progress {
            let box = Unmanaged.passRetained(ProgressBox(progress))
            defer { box.release() }
            handle = directory.withCString {
                platypus_library_open($0, box.toOpaque(), progressTrampoline)
            }
        } else {
            handle = directory.withCString { platypus_library_open($0, nil, nil) }
        }
        if handle == nil { return nil }
    }

    deinit { platypus_library_close(handle) }

    func stats() -> LibraryStats? {
        guard let ptr = platypus_library_stats_json(handle) else { return nil }
        defer { platypus_string_free(ptr) }
        let data = Data(bytes: ptr, count: strlen(ptr))
        return try? JSONDecoder().decode(LibraryStats.self, from: data)
    }

    /// System-level rows matching the filter.
    func catalog(_ f: FilterState) -> [CatalogSystem] {
        let ptr = f.servicesCSV.withCString { svc in
            f.techsCSV.withCString { tech in
                f.search.withCString { search in
                    platypus_library_catalog_json(handle, svc, tech, search)
                }
            }
        }
        return Self.decode(ptr)
    }

    /// Append library channels to an existing favorites list, returning a new handle.
    func appendToFavorites(_ fav: Favorites, channelIDs: [String], departmentsOn: Bool) -> Favorites? {
        let csv = channelIDs.joined(separator: ",")
        let h = csv.withCString {
            platypus_favorites_append_from_library(fav.handle, handle, $0, departmentsOn)
        }
        return Favorites(handle: h)
    }

    /// Geo-located systems within `miles` of a point (for the map), filtered.
    func geo(lat: Double, lon: Double, miles: Double, _ f: FilterState) -> [GeoSystem] {
        let ptr = f.servicesCSV.withCString { svc in
            f.techsCSV.withCString { tech in
                f.search.withCString { search in
                    platypus_library_geo_json(handle, lat, lon, miles, svc, tech, search)
                }
            }
        }
        return Self.decode(ptr)
    }

    /// Build a favorites list from an explicit channel-id selection (the cart).
    func buildFavorites(channelIDs: [String], departmentsOn: Bool, bandPlan: Bool) -> Favorites? {
        let csv = channelIDs.joined(separator: ",")
        let h = csv.withCString {
            platypus_favorites_from_channels(handle, $0, departmentsOn, bandPlan)
        }
        return Favorites(handle: h)
    }

    /// The filter-passing channels of one system (lazy-load on expand). Used by the
    /// flat/search view (no county scoping).
    func channels(systemID: String, _ f: FilterState) -> [CatalogChannel] {
        let ptr = systemID.withCString { sid in
            f.servicesCSV.withCString { svc in
                f.search.withCString { search in
                    platypus_library_channels_json(handle, sid, svc, search)
                }
            }
        }
        return Self.decode(ptr)
    }

    /// The channels of one system whose group is placed in `county` (county-scoped
    /// view). `county == 0` returns the no-county/statewide-residual channels.
    func countyChannels(systemID: String, county: UInt64, _ f: FilterState) -> [CatalogChannel] {
        let ptr = systemID.withCString { sid in
            f.servicesCSV.withCString { svc in
                f.search.withCString { search in
                    platypus_library_county_channels_json(
                        handle, sid, county, svc, search)
                }
            }
        }
        return Self.decode(ptr)
    }

    /// The channels of one system whose department geo is within `miles` of
    /// (`lat`, `lon`) — the location-first "add only what's near here" for the map.
    func radiusChannels(systemID: String, lat: Double, lon: Double, miles: Double, _ f: FilterState)
        -> [CatalogChannel]
    {
        let ptr = systemID.withCString { sid in
            f.servicesCSV.withCString { svc in
                f.search.withCString { search in
                    platypus_library_radius_channels_json(
                        handle, sid, lat, lon, miles, svc, search)
                }
            }
        }
        return Self.decode(ptr)
    }

    private static func decode<T: Decodable>(_ ptr: UnsafeMutablePointer<CChar>?) -> [T] {
        guard let ptr else { return [] }
        defer { platypus_string_free(ptr) }
        let data = Data(bytes: ptr, count: strlen(ptr))
        return (try? JSONDecoder().decode([T].self, from: data)) ?? []
    }
}
