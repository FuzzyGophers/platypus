// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import Foundation

/// A geo-bearing location inside a system (a Site, T-Group, or C-Group).
struct ScannerLocation: Codable, Identifiable {
    let name: String
    let lat: Double
    let lon: Double
    let range: Double
    var id: String { "\(name)|\(lat)|\(lon)" }
}

/// One radio system (Conventional or Trunk) and its display metadata.
struct ScannerSystem: Codable, Identifiable {
    let name: String
    let kind: String
    let counties: [UInt64]
    let locations: [ScannerLocation]
    var id: String { "\(kind)|\(name)|\(counties.map(String.init).joined(separator: ","))" }
}

/// One county from the HPDB county master.
struct County: Codable, Identifiable {
    let id: UInt64
    let name: String
    let state: UInt64?
}

/// One state from the HPDB master — the top of Country → State → County.
struct StateInfo: Codable, Identifiable {
    let id: UInt64
    let name: String
    let abbr: String
    let country: UInt64
}

/// Safe Swift wrapper over the `platypus-ffi` C ABI. Owns the Rust handle and
/// decodes the JSON results into the structs above. All calls are read-only.
final class Hpdb {
    private let handle: OpaquePointer?

    init?(path: String) {
        handle = path.withCString { platypus_open_hpdb($0) }
        if handle == nil { return nil }
    }

    deinit {
        platypus_close_hpdb(handle)
    }

    func allSystems() -> [ScannerSystem] {
        decodeSystems(platypus_systems_json(handle))
    }

    func systems(inCounty id: UInt64) -> [ScannerSystem] {
        decodeSystems(platypus_systems_in_county_json(handle, id))
    }

    func systems(near lat: Double, _ lon: Double, milesRadius: Double) -> [ScannerSystem] {
        decodeSystems(platypus_systems_in_radius_json(handle, lat, lon, milesRadius))
    }

    /// Stateless: the county master lives in its own `hpdb.cfg`.
    static func counties(hpdbCfgPath: String) -> [County] {
        let ptr = hpdbCfgPath.withCString { platypus_counties_json($0) }
        return FFI.decode(ptr)
    }

    /// Stateless: the state master (Country → State) from `hpdb.cfg`.
    static func states(hpdbCfgPath: String) -> [StateInfo] {
        let ptr = hpdbCfgPath.withCString { platypus_states_json($0) }
        return FFI.decode(ptr)
    }

    private func decodeSystems(_ ptr: UnsafeMutablePointer<CChar>?) -> [ScannerSystem] {
        FFI.decode(ptr)
    }
}
