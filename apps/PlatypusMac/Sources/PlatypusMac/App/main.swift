// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import Foundation

// Entry point. `--selftest <hpdb-path>` runs a headless check of the Rust FFI
// bridge (no window) and exits — handy for CI and for environments without a
// display. Otherwise launch the SwiftUI app.
let args = CommandLine.arguments
if args.count >= 3, args[1] == "--selftest" {
    runSelfTest(path: args[2], hpdbCfg: args.count >= 4 ? args[3] : nil)
} else if args.count >= 3, args[1] == "--libtest" {
    runLibTest(dir: args[2])
} else {
    // GUI. If launched (e.g. via `open --args <path>`) with an existing path,
    // preload it — a directory becomes the full-USA library, a file the old single
    // HPDB view. Robust against LaunchServices' injected args.
    var isDir: ObjCBool = false
    for arg in args.dropFirst() where FileManager.default.fileExists(atPath: arg, isDirectory: &isDir) {
        if isDir.boolValue {
            setenv("PLATYPUS_LIBRARY", arg, 1)
        } else {
            setenv("PLATYPUS_OPEN", arg, 1)
        }
        break
    }
    PlatypusApp.main()
}

/// Headless check of the full-USA library path (Swift → FFI → core): aggregate load, a
/// filtered catalog, and lazy per-system channels — no window. Prints its findings *and*
/// asserts them: structural invariants always, plus exact expected values when run against the
/// committed `samples/synthetic` fixture (as CI does), so wrong-but-nonzero output fails.
func runLibTest(dir: String) -> Never {
    var failures: [String] = []
    func check(_ ok: Bool, _ msg: String) {
        if !ok { failures.append(msg) }
    }
    // Exact-value checks only when run against the synthetic fixture (the CI/gate target).
    let synthetic = (dir as NSString).lastPathComponent == "synthetic"
    func expect(_ actual: Int, _ want: Int, _ label: String) {
        if synthetic { check(actual == want, "\(label): got \(actual), want \(want)") }
    }

    var lastPhase: UInt32 = 0
    guard let lib = ScannerLibrary(directory: dir, progress: { phase, done, total in
        if phase != lastPhase || done == total {  // print phase boundaries + completion
            lastPhase = phase
            let name = phase == 1 ? "reading" : "indexing"
            print("  progress: \(name) \(done)/\(total)")
        }
    }) else {
        FileHandle.standardError.write(Data("FAIL: could not open library at \(dir)\n".utf8))
        exit(1)
    }
    let started = Date()
    guard let stats = lib.stats() else {
        FileHandle.standardError.write(Data("FAIL: no stats\n".utf8))
        exit(1)
    }
    print("library: \(stats.files) files · \(stats.systems) systems · \(stats.channels) channels")
    check(stats.systems > 0 && stats.channels > 0, "empty library (systems=\(stats.systems), channels=\(stats.channels))")
    expect(stats.files, 1, "files")
    expect(stats.systems, 4, "systems")
    expect(stats.channels, 5, "channels")

    let all = lib.catalog(FilterState())
    print("catalog (no filter): \(all.count) system rows in \(Int(Date().timeIntervalSince(started) * 1000)) ms")
    check(all.count == stats.systems, "catalog rows (\(all.count)) ≠ system count (\(stats.systems))")

    var fire = FilterState()
    fire.services = [3]
    let fireRows = lib.catalog(fire)
    print("filter service=Fire Dispatch: \(fireRows.count) systems")
    check(fireRows.count <= all.count, "a filter returned MORE than the unfiltered set")
    expect(fireRows.count, 1, "fire filter")

    var p25 = FilterState()
    p25.techs = ["P25"]
    let p25Rows = lib.catalog(p25)
    print("filter tech=P25: \(p25Rows.count) systems")
    check(p25Rows.count <= all.count, "tech filter returned MORE than the unfiltered set")
    expect(p25Rows.count, 1, "p25 filter")

    if let first = all.first {
        let chans = lib.channels(systemID: first.id, FilterState())
        print("channels of '\(first.name)': \(chans.count) (e.g. \(chans.prefix(2).map { $0.name }))")
        check(!first.name.isEmpty, "first system has no name")
        check(chans.count > 0, "'\(first.name)' returned 0 channels")
    }

    // County-name master resolution (hpdb.cfg alongside the state files).
    let cfg = (dir as NSString).appendingPathComponent("hpdb.cfg")
    let countyName: [UInt64: String]
    let hasCfg = FileManager.default.fileExists(atPath: cfg)
    if hasCfg {
        let counties = Hpdb.counties(hpdbCfgPath: cfg)
        countyName = Dictionary(counties.map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        print("counties master: \(counties.count) entries")
        check(!counties.isEmpty, "hpdb.cfg present but county master is empty")
        expect(counties.count, 3, "counties master")
    } else {
        countyName = [:]
    }
    // The synthetic fixture ships an hpdb.cfg — its absence means the county-master path
    // silently didn't run.
    check(!synthetic || hasCfg, "synthetic fixture missing hpdb.cfg — county master untested")

    // Location-first county placement: a wide-area system places into ≥1 county and is
    // county-scopable. Generic — names come from whatever data is loaded.
    let widest = all.max(by: { $0.counties.count < $1.counties.count })
    check(!synthetic || widest != nil, "no systems to county-place")
    if let widest {
        print(
            "widest system: '\(widest.name)' covers \(widest.counties.count) counties, "
                + "states=\(widest.states), statewide=\(widest.statewide)")
        check(widest.counties.count >= 1, "widest system places into 0 counties")
        if let cid = widest.counties.first {
            let scoped = lib.countyChannels(systemID: widest.id, county: cid, FilterState())
            print(
                "  scoped to '\(countyName[cid] ?? "County \(cid)")': \(scoped.count) channels "
                    + "(e.g. \(scoped.prefix(3).map { $0.name }))")
            check(scoped.count > 0, "county-scoping the widest system returned 0 channels")
        }
    }

    // Map geo query: pick the first geo-located system as a center (generic; no hardcoded
    // location), then query a radius around it — the center must be within its own radius.
    let seed = lib.geo(lat: 39.5, lon: -98.35, miles: 1500, FilterState()).first
    check(!synthetic || seed != nil, "geo query found no geo-located system in the fixture")
    if let seed {
        let near = lib.geo(lat: seed.lat, lon: seed.lon, miles: 25, FilterState())
        print("map geo: center '\(seed.name)' (\(seed.lat),\(seed.lon)) → \(near.count) systems within 25 mi")
        check(near.contains { $0.name == seed.name }, "a system isn't within its own 25 mi radius")
    }

    // Card favorites management (only when `dir` sits inside a card layout — not the synthetic
    // fixture). Best-effort; asserted by the FFI/Swift unit tests, not here.
    let mount = ((dir as NSString).deletingLastPathComponent as NSString).deletingLastPathComponent
    if let card = CardFavorites.read(cardMount: mount) {
        print(
            "card: \(card.model) (\(card.modelId ?? "?")) · \(card.lists.count)/\(card.maxFavorites) lists"
                + " · e.g. \(card.lists.prefix(4).map { "\($0.name) [\($0.systems)sys/\($0.channels)ch]" })")
    }

    if failures.isEmpty {
        print("libtest OK")
        exit(0)
    }
    FileHandle.standardError.write(
        Data(("FAIL: libtest assertions:\n  " + failures.joined(separator: "\n  ") + "\n").utf8))
    exit(1)
}

func runSelfTest(path: String, hpdbCfg: String?) -> Never {
    guard let db = Hpdb(path: path) else {
        FileHandle.standardError.write(Data("FAIL: could not open \(path)\n".utf8))
        exit(1)
    }
    let all = db.allSystems()
    print("systems: \(all.count)")
    for s in all.prefix(3) {
        print("  [\(s.kind)] \(s.name)  counties=\(s.counties)  locations=\(s.locations.count)")
    }
    if let first = all.first(where: { !$0.counties.isEmpty })?.counties.first {
        print("filter county \(first): \(db.systems(inCounty: first).count) system(s)")
    }
    if let cfg = hpdbCfg {
        let counties = Hpdb.counties(hpdbCfgPath: cfg)
        print("counties in master: \(counties.count)  e.g. \(counties.prefix(3).map { "\($0.id)=\($0.name)" })")
    }
    exit(all.isEmpty ? 1 : 0)
}
