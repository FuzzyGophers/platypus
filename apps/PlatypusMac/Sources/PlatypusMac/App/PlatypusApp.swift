// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

struct PlatypusApp: App {
    // Installs the quit/close guard (warn on unsaved staged changes). See QuitGuard.swift.
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        WindowGroup("Platypus") {
            CatalogView()
                .background(WindowAccessor())
        }
    }
}
