// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import SwiftUI

/// Service-type codes (`FuncTagId`) → display name, SF Symbol, and tint color. The
/// **names come from the core** (`platypus_service_types_json`, the single source of
/// truth); the app supplies only presentation — the SF Symbol + tint. Codes the spec
/// marks `non` (5/10/18/19/27/28/35/36) are unused; unknown codes fall back to a neutral
/// radio glyph. Colors group by family (law=blue, fire=orange, EMS=green, multi=indigo)
/// with a speech-bubble glyph for the "Talk" variants.
enum ServiceType {
    struct Info {
        let name: String
        let symbol: String
        let color: Color
    }

    /// Presentation per code — SF Symbol + tint only. Names are merged in from the core.
    private struct Look {
        let symbol: String
        let color: Color
    }

    private static let look: [Int: Look] = [
        1: Look(symbol: "person.3.fill", color: Color(hex: 0x8a8cff)),
        2: Look(symbol: "shield.lefthalf.filled", color: Color(hex: 0x5aa0ff)),
        3: Look(symbol: "flame.fill", color: Color(hex: 0xff6a3d)),
        4: Look(symbol: "cross.fill", color: Color(hex: 0x34c759)),
        6: Look(symbol: "person.3", color: Color(hex: 0xa2a4ff)),
        7: Look(symbol: "shield.fill", color: Color(hex: 0x7eb1ff)),
        8: Look(symbol: "flame", color: Color(hex: 0xffa04d)),
        9: Look(symbol: "cross.circle.fill", color: Color(hex: 0x5fd67f)),
        11: Look(symbol: "exclamationmark.triangle.fill", color: Color(hex: 0xffcf33)),
        12: Look(symbol: "cross.case.fill", color: Color(hex: 0x2dd4bf)),
        13: Look(symbol: "antenna.radiowaves.left.and.right", color: Color(hex: 0xbf7af0)),
        14: Look(symbol: "wrench.and.screwdriver.fill", color: Color(hex: 0xc79a6a)),
        15: Look(symbol: "airplane", color: Color(hex: 0x5ad1ff)),
        16: Look(symbol: "building.columns.fill", color: Color(hex: 0x9aa3b2)),
        17: Look(symbol: "briefcase.fill", color: Color(hex: 0xa89bc4)),
        20: Look(symbol: "tram.fill", color: Color(hex: 0xb5835a)),
        21: Look(symbol: "ellipsis.circle.fill", color: Color(hex: 0x98a0ad)),
        22: Look(symbol: "bubble.left.and.bubble.right.fill", color: Color(hex: 0xb4b6ff)),
        23: Look(symbol: "bubble.left.fill", color: Color(hex: 0x9cc3ff)),
        24: Look(symbol: "bubble.left.fill", color: Color(hex: 0xffb877)),
        25: Look(symbol: "bubble.left.fill", color: Color(hex: 0x86e0a0)),
        26: Look(symbol: "bus.fill", color: Color(hex: 0x2db39a)),
        29: Look(symbol: "exclamationmark.octagon.fill", color: Color(hex: 0xff7a5c)),
        30: Look(symbol: "star.circle.fill", color: Color(hex: 0x7d8c4f)),
        31: Look(symbol: "megaphone.fill", color: Color(hex: 0xe0679a)),
        32: Look(symbol: "graduationcap.fill", color: Color(hex: 0xd0a94f)),
        33: Look(symbol: "lock.shield.fill", color: Color(hex: 0x8fa0b8)),
        34: Look(symbol: "bolt.fill", color: Color(hex: 0xf2c744)),
        37: Look(symbol: "lock.fill", color: Color(hex: 0x8f8a80)),
        208: Look(symbol: "star.circle.fill", color: Color(hex: 0x9aa0a8)),
        209: Look(symbol: "star.circle.fill", color: Color(hex: 0x9aa0a8)),
        210: Look(symbol: "star.circle.fill", color: Color(hex: 0x9aa0a8)),
        211: Look(symbol: "star.circle.fill", color: Color(hex: 0x9aa0a8)),
        212: Look(symbol: "star.circle.fill", color: Color(hex: 0x9aa0a8)),
        213: Look(symbol: "star.circle.fill", color: Color(hex: 0x9aa0a8)),
        214: Look(symbol: "star.circle.fill", color: Color(hex: 0x9aa0a8)),
        215: Look(symbol: "star.circle.fill", color: Color(hex: 0x9aa0a8)),
        216: Look(symbol: "flag.checkered", color: Color(hex: 0xc94f6d)),
        217: Look(symbol: "flag.checkered", color: Color(hex: 0xd06a84)),
    ]

    /// Code → name, loaded once from the core (`platypus_service_types_json`).
    private static let names: [Int: String] = {
        guard let c = platypus_service_types_json() else { return [:] }
        defer { platypus_string_free(c) }
        let data = Data(String(cString: c).utf8)
        struct Entry: Codable {
            let code: Int
            let name: String
        }
        let entries = (try? JSONDecoder().decode([Entry].self, from: data)) ?? []
        return Dictionary(uniqueKeysWithValues: entries.map { ($0.code, $0.name) })
    }()

    /// Every named service type offered in the filter rail (Custom slots excluded —
    /// they're user-specific and rare). Rendered alphabetically via
    /// `filterOrderAlphabetical`.
    static let filterOrder = [
        1, 2, 3, 4, 6, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 20, 21, 22, 23, 24, 25, 26, 29, 30, 31,
        32, 33, 34, 37,
    ]

    /// The filter rail sorted alphabetically by display name (the sidebar order).
    static var filterOrderAlphabetical: [Int] {
        filterOrder.sorted {
            info($0).name.localizedCaseInsensitiveCompare(info($1).name) == .orderedAscending
        }
    }

    static func info(_ code: Int?) -> Info {
        guard let code else {
            return Info(name: "Other", symbol: "dot.radiowaves.left.and.right", color: Theme.fg3)
        }
        let name = names[code] ?? "Service type \(code)"
        guard let look = look[code] else {
            return Info(name: name, symbol: "dot.radiowaves.left.and.right", color: Theme.fg3)
        }
        return Info(name: name, symbol: look.symbol, color: look.color)
    }
}

/// Technology pills the 1B sidebar offers (system-level, fuzzy-matched in core).
enum TechFilter {
    static let all = ["P25", "DMR", "NXDN", "LTR", "Motorola", "EDACS", "Analog"]

    /// The pills in the sidebar order — alphabetical.
    static var allAlphabetical: [String] {
        all.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    }
}

/// Parses a channel's raw audio-option field (SDS150 C-Freq column 7) into a friendly
/// label/value. The field is a union: `TONE=C156.7` (CTCSS) / `TONE=D023` (DCS) /
/// `NAC=293` (P25) / `ColorCode=1` (DMR) / `RAN=1` (NXDN) / `Area=…`.
enum AudioOption {
    /// (row label, human value), or nil when the field is empty.
    static func parse(_ raw: String?) -> (label: String, value: String)? {
        guard let raw = raw?.trimmingCharacters(in: .whitespaces), !raw.isEmpty else { return nil }
        func after(_ p: String) -> String { String(raw.dropFirst(p.count)) }
        if raw.hasPrefix("TONE=") {
            let v = after("TONE=")
            if v.hasPrefix("C") { return ("Tone", "CTCSS \(v.dropFirst()) Hz") }
            if v.hasPrefix("D") { return ("Tone", "DCS \(v.dropFirst())") }
            return ("Tone", v)
        }
        if raw.hasPrefix("NAC=") { return ("NAC", after("NAC=")) }
        if raw.hasPrefix("ColorCode=") { return ("Color code", after("ColorCode=")) }
        if raw.hasPrefix("RAN=") { return ("RAN", after("RAN=")) }
        if raw.hasPrefix("Area=") { return ("Area", after("Area=")) }
        return ("Tone", raw)
    }

    /// A compact one-piece label for inline subtitles, e.g. "CTCSS 156.7 Hz" or "NAC 293".
    static func inline(_ raw: String?) -> String? {
        parse(raw).map { $0.label == "Tone" ? $0.value : "\($0.label) \($0.value)" }
    }
}

/// Country id → name. The HPDB master has no `CountryInfo` record, so this is the
/// known RadioReference country mapping (the SDS database covers the US + Canada).
enum Country {
    static let names: [UInt64: String] = [
        0: "Nationwide & Interstate", 1: "United States", 2: "Canada",
    ]
    static func name(_ id: UInt64) -> String { names[id] ?? "Country \(id)" }
}
