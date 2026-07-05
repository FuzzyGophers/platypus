// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import AppKit
import CPlatypusFFI
import Foundation
import SwiftUI

/// Preview summary of a built favorites list (from the core).
struct FavoritesSummary: Codable {
    let systems: Int
    let sites: Int
    let dqks: Int
    let bandPlans: Int
    let bytes: Int
}

/// A built favorites list, ready to preview and commit. Owns the Rust handle.
final class Favorites: Identifiable {
    let id = UUID()
    let handle: OpaquePointer?

    init?(handle: OpaquePointer?) {
        self.handle = handle
        if handle == nil { return nil }
    }

    deinit { platypus_favorites_free(handle) }

    func summary() -> FavoritesSummary? {
        guard let ptr = platypus_favorites_summary_json(handle) else { return nil }
        defer { platypus_string_free(ptr) }
        let data = Data(bytes: ptr, count: strlen(ptr))
        return try? JSONDecoder().decode(FavoritesSummary.self, from: data)
    }

    /// Write to a mounted card. Returns nil on success, else an error message.
    func commit(cardMount: String, slot: UInt32, label: String) -> String? {
        let err = cardMount.withCString { m in
            label.withCString { l in platypus_favorites_commit(handle, m, slot, l) }
        }
        guard let err else { return nil }
        defer { platypus_string_free(err) }
        return String(cString: err)
    }

    /// Write just this list's slot file (content only — no f_list/app_data). Part of
    /// the batched save; pair with `CardFavorites.applyLayout`. Nil on success.
    func writeSlot(cardMount: String, slot: UInt32) -> String? {
        guard let err = cardMount.withCString({ platypus_favorites_write_slot(handle, $0, slot) })
        else { return nil }
        defer { platypus_string_free(err) }
        return String(cString: err)
    }

    // MARK: - Editing (Update)

    /// A new, empty favorites list (for the "+ New list" flow).
    static func new() -> Favorites? {
        Favorites(handle: platypus_favorites_new())
    }

    /// Open an existing favorites list from a card slot as an editable handle.
    static func open(cardMount: String, slot: UInt32) -> Favorites? {
        let h = cardMount.withCString { platypus_favorites_open($0, slot) }
        return Favorites(handle: h)
    }

    /// The systems + channels in this list (for the edit UI).
    func systems() -> [FavSystem] {
        guard let ptr = platypus_favorites_channels_json(handle) else { return [] }
        defer { platypus_string_free(ptr) }
        let data = Data(bytes: ptr, count: strlen(ptr))
        return (try? JSONDecoder().decode([FavSystem].self, from: data)) ?? []
    }

    /// A new handle with the given channels (`"s<si>c<ci>"`) removed.
    func removing(channelIDs: [String]) -> Favorites? {
        let csv = channelIDs.joined(separator: ",")
        return Favorites(handle: csv.withCString { platypus_favorites_remove(handle, $0) })
    }

    /// A new handle with systems sorted alphabetically.
    func sortedBySystem() -> Favorites? {
        Favorites(handle: platypus_favorites_sort(handle))
    }

    /// The scan/avoid tree: systems → departments → channels, each with its avoid flag.
    func tree() -> [FavSystemTree] {
        guard let ptr = platypus_favorites_tree_json(handle) else { return [] }
        defer { platypus_string_free(ptr) }
        let data = Data(bytes: ptr, count: strlen(ptr))
        return (try? JSONDecoder().decode([FavSystemTree].self, from: data)) ?? []
    }

    /// A new handle with the avoid flag of one record set. `target` is `"s<si>"`
    /// (system), `"s<si>g<gi>"` (department), or `"s<si>c<ci>"` (channel).
    func settingAvoid(target: String, avoid: Bool) -> Favorites? {
        Favorites(handle: target.withCString { platypus_favorites_set_avoid(handle, $0, avoid) })
    }

    /// A new handle with the Priority Channel flag of one **channel** set. `target`
    /// is a channel id `"s<si>c<ci>"`.
    func settingPriority(target: String, on: Bool) -> Favorites? {
        Favorites(handle: target.withCString { platypus_favorites_set_priority(handle, $0, on) })
    }

    /// A new handle with an editable per-channel value field (e.g. `field` = "delay")
    /// of one channel set to `value`. `target` is a channel id `"s<si>c<ci>"`.
    func settingChannelValue(target: String, field: String, value: String) -> Favorites? {
        let h = target.withCString { t in
            field.withCString { f in
                value.withCString { v in
                    platypus_favorites_set_channel_value(handle, t, f, v)
                }
            }
        }
        return Favorites(handle: h)
    }
}

/// One system inside a favorites list (the edit view; channel ids are `"s<si>c<ci>"`).
struct FavSystem: Codable, Identifiable {
    let id: String
    let name: String
    let kind: String
    let channels: [CatalogChannel]
    var isTrunk: Bool { kind == "Trunk" }
}

// MARK: - Scan/avoid tree

/// A system in the avoid tree: its departments, each holding channels.
struct FavSystemTree: Codable, Identifiable {
    let id: String
    let name: String
    let kind: String
    let tech: String?
    let avoid: Bool
    let groups: [FavGroup]
    var isTrunk: Bool { kind == "Trunk" }
}

/// A department (`T-Group`/`C-Group`) within a system. An empty `name` is the
/// synthetic "ungrouped" bucket (channels with no department).
struct FavGroup: Codable, Identifiable {
    let id: String
    let name: String
    let avoid: Bool
    let channels: [FavTreeChannel]
}

/// A channel in the avoid tree (talkgroup or conventional frequency) + its avoid /
/// priority flags.
struct FavTreeChannel: Codable, Identifiable {
    let id: String
    let name: String
    let tgid: String?
    let freqHz: UInt64?
    let mode: String?
    let serviceType: Int?
    /// Raw audio-option field (CTCSS/DCS tone, P25 NAC, DMR color code…), or nil.
    var tone: String?
    let avoid: Bool
    let priority: Bool
    /// Editable per-channel value settings the model exposes for this record type,
    /// keyed by field name (e.g. `"delay"`, `"attenuator"`, `"modulation"`). Only the
    /// fields applicable to this channel's record type are present.
    let settings: [String: String]

    var isTalkgroup: Bool { tgid != nil }
    /// "TG 12345" or "154.2650 MHz".
    var detail: String {
        if let tgid { return "TG \(tgid)" }
        if let freqHz { return String(format: "%.4f MHz", Double(freqHz) / 1_000_000) }
        return ""
    }
    /// Secondary parameter line (service type · mode · the other identifier).
    var subtitle: String {
        var parts: [String] = [ServiceType.info(serviceType).name]
        if let mode { parts.append(mode) }
        if tgid != nil, let freqHz { parts.append(String(format: "%.4f MHz", Double(freqHz) / 1_000_000)) }
        return parts.joined(separator: "  ·  ")
    }
}
