// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// Guided "read from the radio" sheet — pick the serial port and walk the FT-60 clone steps,
/// then start the read. The port list is **live**: it re-scans on open, auto-polls while the
/// sheet is up, and has a Refresh — so plugging in the cable after opening still shows it. The
/// actual transfer runs behind the operation overlay (progress + cancel). Read-only.
struct FT60ReadSheet: View {
    let onStart: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var ports: [String] = []
    @State private var selected: String = ""

    /// Poll for hot-plugged adapters while the sheet is open.
    private let poll = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(spacing: 14) {
            VStack(spacing: 7) {
                Image(systemName: "antenna.radiowaves.left.and.right")
                    .font(.system(size: 18)).foregroundStyle(.white)
                    .frame(width: 46, height: 46).background(Circle().fill(Theme.accent))
                Text("Read from FT-60").font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.fg)
                Text("Clone the radio's memory over the serial cable")
                    .font(.system(size: 11)).foregroundStyle(Theme.fg3).multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)

            // Steps
            VStack(alignment: .leading, spacing: 6) {
                step("1", "Radio off → hold MONI (middle left button) while turning it on.")
                step("2", "Turn the DIAL until the display reads CLONE.")
                step("3", "Press F/W (the screen blinks, returns to CLONE — now armed).")
                step("4", "Click Start below, then hold PTT for ~1 s to send.")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(Theme.bg2).clipShape(RoundedRectangle(cornerRadius: Theme.rField))
            .overlay(RoundedRectangle(cornerRadius: Theme.rField).stroke(Theme.border))

            // Port (live scan + manual entry)
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text("Serial port").font(.system(size: 10.5, weight: .medium)).foregroundStyle(Theme.fg3)
                    Text("detected: \(ports.count)").font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
                    Spacer()
                    Button { rescan() } label: {
                        Label("Refresh", systemImage: "arrow.clockwise").font(.system(size: 10))
                    }.buttonStyle(.plain).foregroundStyle(Theme.accent)
                }
                // Editable path — works even if auto-detection comes back empty.
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

            Text("Read-only — the app never writes to the radio's memory (only the clone ACK).")
                .font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
                .multilineTextAlignment(.center).frame(maxWidth: .infinity)

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large)
                Button("Start") { onStart(selected); dismiss() }
                    .buttonStyle(.borderedProminent).controlSize(.large)
                    .disabled(selected.isEmpty)
                Spacer()
            }
        }
        .padding(18).frame(width: 380)
        .background(Theme.panel)
        .onAppear { rescan() }
        .onReceive(poll) { _ in rescan() }
    }

    /// Re-enumerate ports. Only auto-fills the field when it's empty — never clobbers a path
    /// the user typed (the 1 s poll would otherwise wipe it).
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
