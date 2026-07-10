// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// The device *class* a radio belongs to — the single axis the UI branches on (so nothing
/// keys off a specific model's identity). New radios pick a class; behaviour follows.
enum RadioClass {
    /// Programmed by editing a relational, geo-tagged SD-card database (favorites lists).
    /// e.g. the Uniden SDS150.
    case sdCard
    /// Programmed by cloning a flat EEPROM memory image over a serial cable. e.g. the Yaesu
    /// FT-60R.
    case cloneImage
}

/// One radio Platypus supports. Identity (`id`, `name`, `maker`, `transport`, `deviceClass`)
/// comes from the **core profile registry** via the FFI — the single source of truth — so
/// adding a radio is just a core `device/<model>.rs` + `register()`, no app edit to the list.
/// The app supplies only presentation (`accent`, `symbol`). `id` is a persisted key.
struct RadioModel: Identifiable, Hashable {
    let id: String
    let name: String
    let maker: String
    /// Short transport description shown in menus/sheets (e.g. "SD card", "serial clone").
    let transport: String
    let deviceClass: RadioClass
    let accent: Color
    let symbol: String
    /// Fixed memory capacity for clone-image radios (from the core registry); nil for SD-card
    /// scanners (whose limits come from the card's profile).
    let capacity: FTCapacity?
    /// What this radio can be programmed with (trunking + modulations), from the core registry.
    /// nil ⇒ the core didn't report capability (unknown) → the browse fails open (no filtering).
    let capability: RadioCapability?

    /// "SDS150 — Uniden · SD card" — the switcher/sheet row label.
    var menuTitle: String { "\(name) — \(maker) · \(transport)" }

    /// Every radio Platypus supports — sourced from the core registry (`Radios.list()`).
    static let supported: [RadioModel] = Radios.list().map(RadioModel.init(core:))

    static func find(_ id: String?) -> RadioModel? {
        guard let id else { return nil }
        return supported.first { $0.id == id }
    }

    /// Pure-UI presentation (accent + SF Symbol) per radio id; unknown ids get a neutral default.
    private static let presentation: [String: (accent: Color, symbol: String)] = [
        "sds150": (Color(hex: 0x0a84ff), "sdcard"),
        "ft60r": (Color(hex: 0x34c759), "dot.radiowaves.left.and.right"),
    ]

    private init(core r: Radios.Info) {
        let p = RadioModel.presentation[r.id]
            ?? (Color(hex: 0x8e8e93), "dot.radiowaves.left.and.right")
        self.id = r.id
        self.name = r.name
        self.maker = r.maker
        self.transport = r.transport
        self.deviceClass = r.deviceClass == "cloneImage" ? .cloneImage : .sdCard
        self.accent = p.accent
        self.symbol = p.symbol
        if let ch = r.channels, let bk = r.banks, let nl = r.nameLen {
            self.capacity = FTCapacity(channels: ch, banks: bk, nameLen: nl)
        } else {
            self.capacity = nil
        }
        self.capability = RadioCapability(
            trunking: r.trunking ?? false, modulations: r.modulations ?? [])
    }
}

/// The user's owned radios + which one is active, persisted across launches. First run is
/// **neutral** (no radios owned, none active) — the app asks rather than assuming a default.
final class RadioStore: ObservableObject {
    private static let ownedKey = "platypus_owned_radios"
    private static let activeKey = "platypus_active_radio"

    @Published private(set) var ownedIDs: Set<String>
    @Published private(set) var activeID: String?

    init() {
        let owned = (UserDefaults.standard.array(forKey: Self.ownedKey) as? [String]) ?? []
        ownedIDs = Set(owned)
        let stored = UserDefaults.standard.string(forKey: Self.activeKey)
        // Activate the stored radio if still owned; else the sole owned radio; else neutral.
        if let stored, ownedIDs.contains(stored) {
            activeID = stored
        } else if owned.count == 1 {
            activeID = owned.first
        } else {
            activeID = nil
        }
    }

    // MARK: - Derived views

    /// The user's radios, in the supported-catalog order.
    var owned: [RadioModel] { RadioModel.supported.filter { ownedIDs.contains($0.id) } }
    var active: RadioModel? { RadioModel.find(activeID) }
    func isOwned(_ id: String) -> Bool { ownedIDs.contains(id) }

    // MARK: - Mutators (persist on every change)

    /// Add a radio to the owned set; auto-activate it if it's the first one owned.
    func add(_ id: String) {
        guard !ownedIDs.contains(id) else { return }
        ownedIDs.insert(id)
        if activeID == nil { activeID = id }
        persist()
    }

    /// Remove a radio; if it was active, fall back to another owned radio (or neutral).
    func remove(_ id: String) {
        guard ownedIDs.remove(id) != nil else { return }
        if activeID == id { activeID = owned.first?.id }
        persist()
    }

    /// Make an owned radio the active target.
    func setActive(_ id: String?) {
        if let id, !ownedIDs.contains(id) { return }
        activeID = id
        persist()
    }

    private func persist() {
        UserDefaults.standard.set(Array(ownedIDs), forKey: Self.ownedKey)
        UserDefaults.standard.set(activeID, forKey: Self.activeKey)
    }
}
