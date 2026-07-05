// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// The app's design tokens — the dark palette, and the single source of truth for it.
/// Light support can layer on later; this is the hi-fi dark look.
enum Theme {
    static let canvas = Color(hex: 0x2c2c2e)
    static let bg = Color(hex: 0x1c1c1e)
    static let bg2 = Color(hex: 0x242427)
    static let bg3 = Color(hex: 0x2a2a2e)
    static let panel = Color(hex: 0x303035)
    static let titlebar = Color(hex: 0x2a2a2e)
    static let fg = Color(hex: 0xf3f3f6)
    static let fg2 = Color(hex: 0xa2a2ab)
    static let fg3 = Color(hex: 0x6f6f78)
    static let border = Color.white.opacity(0.08)
    static let border2 = Color.white.opacity(0.15)
    static let accent = Color(hex: 0x0a84ff)
    static let accent2 = Color(hex: 0x409cff)
    static let selectionTint = Color(hex: 0x0a84ff).opacity(0.20)
    static let chip = Color.white.opacity(0.06)
    static let warn = Color(hex: 0xff6a3d)
    static let encrypted = Color(hex: 0xffcf33)

    // Radii (spec).
    static let rWindow: CGFloat = 13
    static let rCard: CGFloat = 9
    static let rField: CGFloat = 7
    static let rChip: CGFloat = 5
}

extension Color {
    /// 0xRRGGBB literal → Color.
    init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xff) / 255,
            green: Double((hex >> 8) & 0xff) / 255,
            blue: Double(hex & 0xff) / 255,
            opacity: 1
        )
    }
}
