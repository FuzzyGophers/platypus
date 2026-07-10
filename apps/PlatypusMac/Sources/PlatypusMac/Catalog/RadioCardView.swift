// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// One entry in a `RadioCardView` overflow (⋯) menu. An empty `title` renders a divider.
struct RadioBarMenuItem: Identifiable {
    let id = UUID()
    let title: String
    var role: ButtonRole? = nil
    var disabled = false
    var action: () -> Void = {}
    static let divider = RadioBarMenuItem(title: "")
}

/// The tint of a radio card's icon tile — one look per connection state, shared by every radio so
/// the cards read as one family (SD-card blue, live serial green, disconnected gray).
enum RadioCardTint {
    case sdCard, connected, idle

    var fill: Color {
        switch self {
        case .sdCard: return Theme.accent.opacity(0.16)
        case .connected: return Theme.success.opacity(0.16)
        case .idle: return Theme.bg3
        }
    }
    var stroke: Color {
        switch self {
        case .sdCard: return Color(hex: 0x4c9bff)
        case .connected: return Theme.success
        case .idle: return Theme.fg3
        }
    }
}

/// The unified "My radios" card — one consistent skeleton every radio's editor header uses, so the
/// SD-card scanner and the serial-clone HT align instead of each crowding its own controls. Three
/// zones: **Header** (icon tile · name + connection · single ⋯ menu), **Actions** (Open · Read ·
/// Write), and a **Detail slot** (caller-supplied: the SD list chooser + capacity, the clone-info
/// line, or a connect hint). The verbs and detail are supplied per radio; the shell is shared.
struct RadioCardView<Detail: View>: View {
    let symbol: String
    let tint: RadioCardTint
    let name: String
    let subtitle: String
    /// Status-dot state next to the subtitle (green when connected, gray otherwise).
    var connected: Bool = true
    /// A warn-tinted line under the subtitle (e.g. "modified — eject before reconnecting").
    var warning: String? = nil
    /// The single overflow (⋯) menu — Settings, Eject (SD only), Forget, etc.
    var menuItems: [RadioBarMenuItem] = []

    let onOpen: () -> Void
    let onRead: () -> Void
    let onWrite: () -> Void
    var openDisabled = false
    var readDisabled = false
    var writeDisabled = false
    var writeHelp = "Save to the device"
    /// The write action's verb — "Write" (to a live device) vs "Save" (to a backup folder on disk).
    var writeLabel = "Write"
    /// Optional secondary write action (SD-card only): push an edited backup onto a connected card.
    /// Rendered as an extra button after Write when set.
    var onWriteToRadio: (() -> Void)? = nil
    var writeToRadioLabel = "Write to Card…"

    @ViewBuilder var detail: () -> Detail

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            actions.padding(.top, 14)
            detailSlot
        }
        .padding(15)
        .background(RoundedRectangle(cornerRadius: Theme.rCard).fill(Theme.panel))
        .overlay(RoundedRectangle(cornerRadius: Theme.rCard).stroke(Theme.border))
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 12) {
            RoundedRectangle(cornerRadius: 10)
                .fill(tint.fill)
                .frame(width: 40, height: 40)
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(.white.opacity(0.06)))
                .overlay(Image(systemName: symbol).font(.system(size: 18)).foregroundStyle(tint.stroke))
            VStack(alignment: .leading, spacing: 2) {
                Text(name).font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.fg).lineLimit(1)
                HStack(spacing: 6) {
                    Circle().fill(connected ? Theme.success : Theme.fg3)
                        .frame(width: 7, height: 7)
                        .overlay(connected ? Circle().stroke(Theme.success.opacity(0.25), lineWidth: 3) : nil)
                    Text(subtitle).font(.system(size: 12)).foregroundStyle(Theme.fg2).lineLimit(1)
                }
                if let warning {
                    Text(warning).font(.system(size: 10, weight: .medium)).foregroundStyle(Theme.warn).lineLimit(1)
                }
            }
            Spacer(minLength: 4)
            if !menuItems.isEmpty {
                Menu {
                    ForEach(menuItems) { item in
                        if item.title.isEmpty {
                            Divider()
                        } else {
                            Button(item.title, role: item.role, action: item.action).disabled(item.disabled)
                        }
                    }
                } label: {
                    Image(systemName: "ellipsis").font(.system(size: 15)).foregroundStyle(Theme.fg2)
                        .frame(width: 30, height: 30)
                }
                .menuStyle(.borderlessButton).fixedSize().help("More radio actions")
            }
        }
    }

    // MARK: - Actions (identical across radios)

    private var actions: some View {
        HStack(spacing: 8) {
            Button("Open", action: onOpen).controlSize(.small).disabled(openDisabled)
                .help("Open a saved backup to edit")
            Spacer(minLength: 8)
            Button("Read", action: onRead).controlSize(.small).disabled(readDisabled)
            Button(writeLabel, action: onWrite).buttonStyle(.borderedProminent).controlSize(.small)
                .disabled(writeDisabled).help(writeHelp)
            if let onWriteToRadio {
                Button(writeToRadioLabel, action: onWriteToRadio).controlSize(.small)
                    .help("Push this backup onto a connected card (Restore, or write your favorites)")
            }
        }
    }

    // MARK: - Detail slot (same position/height for every card)

    private var detailSlot: some View {
        VStack(alignment: .leading, spacing: 0) {
            Divider().overlay(Theme.border).padding(.top, 13)
            detail().padding(.top, 12)
        }
    }
}
