// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import Foundation

/// One selectable value for an FT-60 channel-form field: the human `label` shown in a picker
/// and the on-radio `code` the writer stores. `valueKind` is present only on tone modes
/// ("none"/"ctcss"/"dcs"), telling the form which value field (if any) that mode needs.
struct Ft60Option: Codable, Hashable {
    let label: String
    let code: Int
    let valueKind: String?
}

/// The FT-60 channel-form option sets, loaded once from the **core** profile
/// (`platypus_ft60_options_json`) — the single source of truth for the editor's pickers, so
/// the app declares no mode/step/power/duplex/tone-mode tables of its own. Only presentation
/// (icons, colors) stays in Swift.
final class Ft60Options {
    static let shared = Ft60Options()

    let modes: [Ft60Option]
    let toneModes: [Ft60Option]
    let steps: [Ft60Option]
    let powers: [Ft60Option]
    let duplexes: [Ft60Option]

    private struct Payload: Codable {
        let modes: [Ft60Option]
        let toneModes: [Ft60Option]
        let steps: [Ft60Option]
        let powers: [Ft60Option]
        let duplexes: [Ft60Option]
    }

    private init() {
        let p = Ft60Options.load()
        modes = p?.modes ?? []
        toneModes = p?.toneModes ?? []
        steps = p?.steps ?? []
        powers = p?.powers ?? []
        duplexes = p?.duplexes ?? []
    }

    private static func load() -> Payload? {
        FFI.decodeOne(platypus_ft60_options_json())
    }

    // MARK: - Lookups (used by the row/detail displays and the form)

    /// The label for a `code` in a given option list (empty if unknown).
    func label(_ list: [Ft60Option], _ code: Int) -> String {
        list.first { $0.code == code }?.label ?? ""
    }

    /// The option carrying a `code` in a given list (nil if unknown).
    func option(_ list: [Ft60Option], _ code: Int) -> Ft60Option? {
        list.first { $0.code == code }
    }

    /// The code for the first option whose label matches (nil if none) — used to map a
    /// catalog tone → the right tone-mode code without hard-coding an index.
    func code(_ list: [Ft60Option], label: String) -> Int? {
        list.first { $0.label == label }?.code
    }

    // MARK: - Duplex semantics (the documented transport codes, per platypus.h:
    //         0 simplex, 2 −, 3 +, 4 split). Behavior the form owns, keyed on the ABI code.

    static let duplexSimplex = 0
    /// A +/− shift carries a magnitude offset.
    func duplexNeedsOffset(_ code: Int) -> Bool { code == 2 || code == 3 }
    /// "Split" carries an explicit TX frequency.
    func duplexIsSplit(_ code: Int) -> Bool { code == 4 }
}
