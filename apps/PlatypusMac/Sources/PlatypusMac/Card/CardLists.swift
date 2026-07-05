// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import AppKit
import CPlatypusFFI
import Foundation
import SwiftUI

/// One favorites list currently on the card.
struct FavoriteList: Codable, Identifiable {
    let slot: UInt32
    let name: String
    let filename: String
    let systems: Int
    let channels: Int
    let bytes: Int
    /// The list's **Monitor** flag: scanned whenever the favorites list is active,
    /// regardless of quick keys (the radio's "Select Lists to Monitor").
    let monitor: Bool
    /// The list's **quick key** (0–99), or nil for "Off"/unassigned.
    let quickKey: Int?
    /// The list's **number tag** (0–99), or nil for "Off"/unassigned.
    let numberTag: Int?
    var id: UInt32 { slot }
}

/// The card's scanner model, its limits, and the favorites lists on it.
struct CardInfo: Codable {
    let model: String
    let modelId: String?
    let maxFavorites: Int
    let maxListBytes: Int
    let quickKeys: Int
    /// Enumerated per-channel value options (e.g. `alertColor`, `alertPattern`, `alertTone`,
    /// `alertVolume`) from the card's core profile — the single source for the editor's alert
    /// menus. The value is the stored label; the app adds only presentation (the color swatch).
    let channelValueOptions: [String: [String]]
    let lists: [FavoriteList]

    /// The selectable values for an editable per-channel value field (empty if unknown).
    func valueOptions(_ field: String) -> [String] { channelValueOptions[field] ?? [] }
}

/// Stateless wrappers over the card-management FFI (read lists / delete a slot).
enum CardFavorites {
    static func read(cardMount: String) -> CardInfo? {
        guard let ptr = cardMount.withCString({ platypus_card_favorites_json($0) }) else { return nil }
        defer { platypus_string_free(ptr) }
        let data = Data(bytes: ptr, count: strlen(ptr))
        return try? JSONDecoder().decode(CardInfo.self, from: data)
    }

    /// Returns nil on success, else an error message.
    static func deleteSlot(cardMount: String, slot: UInt32) -> String? {
        guard let err = cardMount.withCString({ platypus_card_delete_slot($0, slot) }) else { return nil }
        defer { platypus_string_free(err) }
        return String(cString: err)
    }

    /// Alphabetize the favorites lists on the card. Nil on success, else an error.
    static func sortLists(cardMount: String) -> String? {
        guard let err = cardMount.withCString({ platypus_card_sort_lists($0) }) else { return nil }
        defer { platypus_string_free(err) }
        return String(cString: err)
    }

    /// Reorder lists into an explicit slot order. Nil on success, else an error.
    static func reorder(cardMount: String, slots: [UInt32]) -> String? {
        let csv = slots.map(String.init).joined(separator: ",")
        guard let err = cardMount.withCString({ m in csv.withCString { platypus_card_reorder_lists(m, $0) } })
        else { return nil }
        defer { platypus_string_free(err) }
        return String(cString: err)
    }

    /// Apply the full favorites layout in one structural pass: `entries` is the
    /// complete ordered set of (slot, name, monitor, quickKey, numberTag);
    /// previously-registered slots not listed are deleted; `app_data.cfg` is deleted.
    /// Each field override sets that F-List field (nil = leave as-is; every other
    /// field is preserved verbatim). `quickKey`/`numberTag` are 0–99 or nil ("Off").
    /// Nil on success, else an error. Pair with `Favorites.writeSlot` for changed
    /// content. The batched-save primitive.
    static func applyLayout(
        cardMount: String,
        entries: [(slot: UInt32, name: String, monitor: Bool?, quickKey: Int?, numberTag: Int?)]
    ) -> String? {
        // "slot\tname\tmonitor\tquickkey\tnumbertag" per line — TAB/newline can't occur
        // in a card list name. Trailing fields carry the F-List settings on every save,
        // so all three are always emitted (quick key / number tag nil → "Off").
        let encoded = entries.map { e -> String in
            let mon = e.monitor.map { $0 ? "On" : "Off" } ?? ""
            let qk = e.quickKey.map(String.init) ?? "Off"
            let nt = e.numberTag.map(String.init) ?? "Off"
            return "\(e.slot)\t\(e.name)\t\(mon)\t\(qk)\t\(nt)"
        }.joined(separator: "\n")
        guard let err = cardMount.withCString({ m in encoded.withCString { platypus_card_apply_layout(m, $0) } })
        else { return nil }
        defer { platypus_string_free(err) }
        return String(cString: err)
    }
}
