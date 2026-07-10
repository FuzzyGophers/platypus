// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import Foundation

/// Bridge over `platypus_radios_json` — the supported-radio list, straight from the core
/// profile registry (the single source of truth). The app adds only presentation on top.
enum Radios {
    /// One radio's identity, as the core reports it.
    struct Info: Codable {
        let id: String
        let name: String
        let maker: String
        let transport: String
        let deviceClass: String  // "sdCard" | "cloneImage"
        // Clone-image radios only: fixed memory capacity from the core.
        let channels: Int?
        let banks: Int?
        let nameLen: Int?
        // What the radio can be programmed with (capability-aware browse): whether it stores
        // trunked talkgroups, and the modulation labels it supports ("analog", "P25", "DMR", …).
        // Optional so the decode tolerates capability schema growth — an absent field just means
        // "unknown", and the app fails open (no filtering) rather than breaking the radio list.
        let trunking: Bool?
        let modulations: [String]?

        enum CodingKeys: String, CodingKey {
            case id, name, maker, transport, channels, banks, nameLen, trunking, modulations
            case deviceClass = "class"
        }
    }

    /// Every radio the core registry knows, in registration order.
    static func list() -> [Info] {
        FFI.decode(platypus_radios_json())
    }
}
