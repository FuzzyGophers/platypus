// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// Guided "write to the radio" sheet — the clone-OUT counterpart of `FT60ReadSheet`. Picks the
/// serial port (live re-scan, same as the read sheet), walks the FT-60 clone RECEIVE steps
/// (`-WAIT-`), and makes the operator confirm — this writes to the radio's memory. The transfer
/// runs behind the operation overlay (progress + cancel). Scope: writes the edited image (channel +
/// PMS edits applied onto the read image), preserving every byte we don't model.
struct FT60WriteSheet: View {
    let channelCount: Int
    let onStart: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var ports: [String] = []
    @State private var selected: String = ""
    @State private var acknowledged = false

    private let poll = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(spacing: 14) {
            VStack(spacing: 7) {
                Image(systemName: "square.and.arrow.up.on.square")
                    .font(.system(size: 18)).foregroundStyle(.white)
                    .frame(width: 46, height: 46).background(Circle().fill(Theme.warn))
                Text("Write to FT-60").font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.fg)
                Text("Clone \(channelCount) channels back onto the radio's memory")
                    .font(.system(size: 11)).foregroundStyle(Theme.fg3).multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)

            // Steps — receive side of the clone.
            VStack(alignment: .leading, spacing: 6) {
                step("1", "Radio off → hold MONI (middle left button) while turning it on.")
                step("2", "Turn the DIAL until the display reads CLONE.")
                step("3", "Press F/W (the screen blinks, returns to CLONE — now armed).")
                step("4", "Press MONI once — the display shows -WAIT- (ready to receive).")
                step("5", "Click Write below to send the image.")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(Theme.bg2).clipShape(RoundedRectangle(cornerRadius: Theme.rField))
            .overlay(RoundedRectangle(cornerRadius: Theme.rField).stroke(Theme.border))

            // Port (live scan + manual entry) — mirrors the read sheet.
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text("Serial port").font(.system(size: 10.5, weight: .medium)).foregroundStyle(Theme.fg3)
                    Text("detected: \(ports.count)").font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
                    Spacer()
                    Button { rescan() } label: {
                        Label("Refresh", systemImage: "arrow.clockwise").font(.system(size: 10))
                    }.buttonStyle(.plain).foregroundStyle(Theme.accent)
                }
                TextField("/dev/cu.usbserial-…", text: $selected)
                    .textFieldStyle(.plain).font(.system(size: 12).monospaced()).foregroundStyle(Theme.fg)
                    .padding(.horizontal, 9).padding(.vertical, 7)
                    .background(Theme.bg3).clipShape(RoundedRectangle(cornerRadius: Theme.rField))
                    .overlay(RoundedRectangle(cornerRadius: Theme.rField).stroke(Theme.border))
                if ports.isEmpty {
                    Text("None auto-detected — plug in the cable (auto-detects), or type the path. It's usually /dev/cu.usbserial-…")
                        .font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
                } else {
                    HStack(spacing: 5) {
                        ForEach(ports, id: \.self) { p in
                            Button { selected = p } label: {
                                Text(p.replacingOccurrences(of: "/dev/cu.", with: ""))
                                    .font(.system(size: 9.5, weight: .medium))
                                    .padding(.horizontal, 6).padding(.vertical, 2)
                                    .background(selected == p ? Theme.accent.opacity(0.25) : Theme.chip)
                                    .foregroundStyle(Theme.fg2)
                                    .clipShape(RoundedRectangle(cornerRadius: Theme.rChip))
                            }.buttonStyle(.plain)
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            // The write acknowledgement — this modifies the radio.
            Toggle(isOn: $acknowledged) {
                Text("I understand this writes to the radio's memory.")
                    .font(.system(size: 11)).foregroundStyle(Theme.fg2)
            }
            .toggleStyle(.checkbox)
            .frame(maxWidth: .infinity, alignment: .leading)

            Text("Writes your edited image back to this radio; every byte you didn't change is preserved from the read.")
                .font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
                .multilineTextAlignment(.center).frame(maxWidth: .infinity)

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large)
                Button("Write") { onStart(selected); dismiss() }
                    .buttonStyle(.borderedProminent).controlSize(.large).tint(Theme.warn)
                    .disabled(selected.isEmpty || !acknowledged)
                Spacer()
            }
        }
        .padding(18).frame(width: 380)
        .background(Theme.panel)
        .onAppear { rescan() }
        .onReceive(poll) { _ in rescan() }
    }

    private func rescan() {
        ports = Ft60.listPorts()
        if selected.isEmpty {
            selected = ports.first(where: { $0.contains("usbserial") || $0.contains("usbmodem") })
                ?? ports.first ?? ""
        }
    }

    private func step(_ n: String, _ text: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Text(n).font(.system(size: 10, weight: .bold)).foregroundStyle(.white)
                .frame(width: 16, height: 16).background(Circle().fill(Theme.accent))
            Text(text).font(.system(size: 11)).foregroundStyle(Theme.fg2)
            Spacer(minLength: 0)
        }
    }
}
