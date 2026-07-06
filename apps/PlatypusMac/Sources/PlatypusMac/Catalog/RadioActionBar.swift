// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// One entry in a `RadioActionBar` overflow (⋯) menu. An empty `title` renders a divider.
struct RadioBarMenuItem: Identifiable {
    let id = UUID()
    let title: String
    var disabled = false
    var action: () -> Void = {}
    static let divider = RadioBarMenuItem(title: "")
}

/// The shared editor action bar used by every radio's editor pane — one consistent header with a
/// device glyph + name/subtitle and the **Open / Read / Write** actions (the verbs are supplied
/// per radio; "Open" loads a saved backup to edit). Optional row-1 trailing controls — a settings
/// gear (clone-image radios), an Eject button + an overflow ⋯ menu (SD-card radios) — sit top-right.
/// Laid out in two rows so nothing crowds or truncates in the narrow editor column.
struct RadioActionBar: View {
    let symbol: String
    let accent: Color
    let name: String
    let subtitle: String
    /// A warn-tinted line under the subtitle (e.g. "modified — eject before reconnecting").
    var warning: String? = nil
    /// Optional settings action — nil hides the gear (only clone-image radios have one today).
    var onSettings: (() -> Void)? = nil
    var settingsDisabled = false
    let onOpen: () -> Void
    let onRead: () -> Void
    let onWrite: () -> Void
    var writeDisabled = false
    var writeHelp = "Save to the device"
    /// Optional Eject action — nil hides the button (SD-card radios only).
    var onEject: (() -> Void)? = nil
    var ejectDisabled = false
    /// Tint Eject as a warning (card modified — must eject before reconnecting).
    var ejectModified = false
    /// Optional overflow (⋯) menu — hidden when empty.
    var menuItems: [RadioBarMenuItem] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 9) {
                Image(systemName: symbol).font(.system(size: 16)).foregroundStyle(accent)
                    .frame(width: 22)
                VStack(alignment: .leading, spacing: 1) {
                    Text(name).font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Theme.fg).lineLimit(1)
                    Text(subtitle).font(.system(size: 9.5)).foregroundStyle(Theme.fg3).lineLimit(1)
                    if let warning {
                        Text(warning).font(.system(size: 9.5, weight: .medium))
                            .foregroundStyle(Theme.warn).lineLimit(1)
                    }
                }
                Spacer(minLength: 4)
                if let onSettings {
                    Button(action: onSettings) {
                        Image(systemName: "gearshape").font(.system(size: 12.5))
                    }
                    .buttonStyle(.plain).foregroundStyle(Theme.fg2)
                    .disabled(settingsDisabled).help("Radio settings")
                }
                if let onEject {
                    Button(action: onEject) { Label("Eject", systemImage: "eject") }
                        .controlSize(.small).tint(ejectModified ? Theme.warn : nil)
                        .disabled(ejectDisabled)
                        .help("Flush + eject the card. Always do this before reconnecting it.")
                }
                if !menuItems.isEmpty {
                    Menu {
                        ForEach(menuItems) { item in
                            if item.title.isEmpty {
                                Divider()
                            } else {
                                Button(item.title, action: item.action).disabled(item.disabled)
                            }
                        }
                    } label: {
                        Image(systemName: "ellipsis.circle").font(.system(size: 13))
                    }
                    .menuStyle(.borderlessButton).fixedSize()
                    .help("More card actions")
                }
            }
            HStack(spacing: 6) {
                Button("Open", action: onOpen).controlSize(.small)
                    .help("Open a saved backup to edit")
                Spacer(minLength: 8)
                Button("Read", action: onRead).controlSize(.small)
                Button("Write", action: onWrite).buttonStyle(.borderedProminent).controlSize(.small)
                    .disabled(writeDisabled).help(writeHelp)
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
    }
}
