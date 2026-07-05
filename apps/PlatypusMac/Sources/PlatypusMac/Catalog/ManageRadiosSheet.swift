// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// Pick which supported radios you own. Toggling a row adds/removes it from the persisted
/// "My radios" set; the switcher then lists only your radios. Adding your first radio makes it
/// active. Mirrors the Add-source sheet's dark chrome.
struct ManageRadiosSheet: View {
    @ObservedObject var radios: RadioStore
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 14) {
            VStack(spacing: 7) {
                Image(systemName: "dot.radiowaves.left.and.right")
                    .font(.system(size: 18)).foregroundStyle(.white)
                    .frame(width: 46, height: 46).background(Circle().fill(Theme.accent))
                Text("Manage radios").font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.fg)
                Text("Choose the radios you own — only these appear in the switcher.")
                    .font(.system(size: 11)).foregroundStyle(Theme.fg3).multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)

            VStack(spacing: 8) {
                ForEach(RadioModel.supported) { radio in
                    radioRow(radio)
                }
            }
            .frame(maxWidth: .infinity)

            Text("Your selection is remembered across launches.")
                .font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
                .multilineTextAlignment(.center).frame(maxWidth: .infinity)

            HStack {
                Spacer()
                Button("Done") { dismiss() }.buttonStyle(.borderedProminent).controlSize(.large)
                Spacer()
            }
        }
        .padding(18).frame(width: 380)
        .background(Theme.panel)
    }

    private func radioRow(_ radio: RadioModel) -> some View {
        let owned = radios.isOwned(radio.id)
        return Button {
            if owned { radios.remove(radio.id) } else { radios.add(radio.id) }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: radio.symbol)
                    .font(.system(size: 15)).foregroundStyle(.white)
                    .frame(width: 34, height: 34).background(Circle().fill(radio.accent))
                VStack(alignment: .leading, spacing: 1) {
                    Text(radio.name).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(Theme.fg)
                    Text("\(radio.maker) · \(radio.transport)")
                        .font(.system(size: 10.5)).foregroundStyle(Theme.fg3)
                }
                Spacer()
                Image(systemName: owned ? "checkmark.circle.fill" : "circle")
                    .font(.system(size: 16))
                    .foregroundStyle(owned ? Theme.accent : Theme.fg3)
            }
            .padding(.horizontal, 10).padding(.vertical, 8)
            .background(owned ? Theme.accent.opacity(0.10) : Theme.bg2)
            .clipShape(RoundedRectangle(cornerRadius: Theme.rField))
            .overlay(RoundedRectangle(cornerRadius: Theme.rField).stroke(Theme.border))
        }
        .buttonStyle(.plain)
    }
}
