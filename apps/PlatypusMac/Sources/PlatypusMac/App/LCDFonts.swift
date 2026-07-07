// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CoreText
import Foundation

/// The bundled OFL pixel fonts the SDS150 display preview uses to mimic the scanner's blocky
/// on-screen bitmap font. Registered once at launch from the app resource bundle; referenced by
/// PostScript name. Substitutes for the proprietary Uniden font (see `REUSE.toml`, `LICENSES/`).
enum LCDFonts {
    /// Large tier names (System / Dept / Channel, Primary Areas).
    static let big = "PixelifySans-Regular"
    /// Small chrome text (status bar, indicator bar, softkeys, AVOID/HOLD tags).
    static let small = "Silkscreen-Regular"
    static let smallBold = "Silkscreen-Bold"

    private static var registered = false

    /// Register the bundled fonts with CoreText so `Font.custom` resolves them. Idempotent.
    static func registerIfNeeded() {
        guard !registered else { return }
        registered = true
        for file in ["Silkscreen-Regular", "Silkscreen-Bold", "PixelifySans"] {
            guard let url = Bundle.module.url(forResource: file, withExtension: "ttf", subdirectory: "Fonts")
                ?? Bundle.module.url(forResource: file, withExtension: "ttf")
            else { continue }
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }
}
