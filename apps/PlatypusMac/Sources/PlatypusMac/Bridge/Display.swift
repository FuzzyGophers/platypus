// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import Foundation
import SwiftUI

// The SDS150 display-customization bridge — reads/writes the four `profile.cfg` display records
// (DisplayOption / Backlight / DispOptItems / DispColors) over the core FFI. The core owns every
// table (palette, item vocabularies, modes, color groups); Swift only presents them.

/// A global display setting (a `DisplayOption`/`Backlight` field) — `value` is editable.
struct DisplayGlobal: Codable, Identifiable {
    let key: String
    let label: String
    var value: String
    let options: [String]
    var id: String { key }
}

/// The ordered item tokens for one `(dispOptId, dispLayoutId)` — `tokens` are editable.
struct DisplayItemGroup: Codable {
    let dispOptId: Int
    let dispLayoutId: Int
    var tokens: [String]
}

/// One text/background color pair (6-hex values from the palette) — editable.
struct DisplayColorPair: Codable {
    var text: String
    var back: String
}

/// The ordered color pairs for one `(dispColorId, colorLayoutId)`.
struct DisplayColorGroup: Codable {
    let dispColorId: Int
    let colorLayoutId: Int
    var pairs: [DisplayColorPair]
}

/// The card's current display customization.
struct DisplayConfigData: Codable {
    var globals: [DisplayGlobal]
    var items: [DisplayItemGroup]
    var colors: [DisplayColorGroup]
}

enum DisplayBridge {
    /// Read the display config from a mounted card (its volume root). Nil if no supported card /
    /// no `profile.cfg` is there.
    static func read(cardMount: String) -> DisplayConfigData? {
        FFI.decodeOne(cardMount.withCString { platypus_display_config_json($0) })
    }

    /// Apply an edit script (see `platypus_display_apply`) and commit. Returns nil on success, or
    /// an error string. The caller must still eject.
    static func apply(cardMount: String, edits: [String]) -> String? {
        let script = edits.joined(separator: "\n")
        let err = cardMount.withCString { m in script.withCString { e in platypus_display_apply(m, e) } }
        guard let err else { return nil }
        defer { platypus_string_free(err) }
        return String(cString: err)
    }
}

/// One palette color: a name + its Uniden 6-hex value.
struct PaletteColor: Codable, Hashable {
    let name: String
    let hex: String
}

/// The allowed item tokens for one screen area.
struct ItemAreaInfo: Codable {
    let dispOptId: Int
    let label: String
    let tokens: [String]
}

/// A display layout mode and its two layout ids.
struct DisplayMode: Codable, Hashable {
    let name: String
    let dispLayoutId: Int
    let colorLayoutId: Int
}

/// A color group and its nominal element names.
struct DisplayColorGroupInfo: Codable {
    let dispColorId: Int
    let elements: [String]
}

/// The display option tables, loaded once from the core — the single source of truth for the
/// editor's pickers (palette, item vocabularies, modes, color groups).
final class DisplayOptions {
    static let shared = DisplayOptions()

    let palette: [PaletteColor]
    let areas: [ItemAreaInfo]
    let modes: [DisplayMode]
    let colorGroups: [DisplayColorGroupInfo]

    private init() {
        palette = FFI.decode(platypus_display_palette_json())
        areas = FFI.decode(platypus_display_items_json())
        modes = FFI.decode(platypus_display_modes_json())
        colorGroups = FFI.decode(platypus_display_color_groups_json())
    }

    func tokens(forArea dispOptId: Int) -> [String] {
        areas.first { $0.dispOptId == dispOptId }?.tokens ?? []
    }

    func areaLabel(_ dispOptId: Int) -> String {
        areas.first { $0.dispOptId == dispOptId }?.label ?? "Area \(dispOptId)"
    }

    func colorName(_ hex: String) -> String {
        palette.first { $0.hex.caseInsensitiveCompare(hex) == .orderedSame }?.name ?? "#\(hex)"
    }

    /// The friendly on-screen name the scanner shows for a File token (confirmed from the hardware
    /// display screens). Unmapped tokens fall back to the token with `_` turned into a space.
    func tokenLabel(_ token: String) -> String {
        DisplayOptions.tokenLabels[token] ?? token.replacingOccurrences(of: "_", with: " ")
    }

    private static let tokenLabels: [String: String] = [
        "FL_Name": "Favorites List Name",
        "CTCSS/DCS": "CTCSS/DCS/NAC",
        "ServiceType": "Service Type",
        "SiteName": "Site Name",
        "SiteId": "Site ID",
        "SystemId": "System ID",
        "SystemType": "System Type",
        "SysSubID": "Sys/Net ID",
        "UnitId": "Unit ID",
        "UnitIdName": "Unit ID Name",
        "BattVoltage": "Batt Voltage",
        "Rssi": "RSSI",
        "Rssi Bar": "RSSI Graph",
        "NumberTag": "Number Tag",
        "P25Status": "P25",
        "TdmaSlot": "Slot",
        "Volume&Squelch": "VOL & SQ",
        "D_ErrorCount": "Error Count",
        "WxPRI": "WX PRI",
        "Day": "Date",
        "P_Ch": "P-Ch",
    ]
}

extension Color {
    /// A palette 6-hex string (`"ff4600"`, no `#`) → `Color`.
    init(displayHex: String) {
        self.init(hex: UInt32(displayHex, radix: 16) ?? 0)
    }
}
