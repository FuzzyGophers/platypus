// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// The editable in-memory copy of a card's display customization, loaded over the FFI.
final class DisplayEditModel: ObservableObject {
    @Published var data: DisplayConfigData
    let loaded: Bool

    init(cardMount: String) {
        if let d = DisplayBridge.read(cardMount: cardMount) {
            data = d
            loaded = true
        } else {
            data = DisplayConfigData(globals: [], items: [], colors: [])
            loaded = false
        }
    }

    func itemIndex(dispOptId: Int, dispLayoutId: Int) -> Int? {
        data.items.firstIndex { $0.dispOptId == dispOptId && $0.dispLayoutId == dispLayoutId }
    }

    func colorIndex(dispColorId: Int, colorLayoutId: Int) -> Int? {
        data.colors.firstIndex { $0.dispColorId == dispColorId && $0.colorLayoutId == colorLayoutId }
    }

    /// The full state as an edit script (see `platypus_display_apply`). Change-gated in the core,
    /// so unedited fields re-encode byte-for-byte.
    func editScript() -> [String] { DisplayEditModel.editScript(for: data) }

    /// Pure encoder (unit-tested): one line per field, tab-delimited, prefixed by record kind.
    static func editScript(for data: DisplayConfigData) -> [String] {
        var out: [String] = []
        for g in data.globals { out.append("G\t\(g.key)\t\(g.value)") }
        for it in data.items {
            for (i, t) in it.tokens.enumerated() {
                out.append("I\t\(it.dispOptId)\t\(it.dispLayoutId)\t\(i)\t\(t)")
            }
        }
        for c in data.colors {
            for (i, p) in c.pairs.enumerated() {
                out.append("C\t\(c.dispColorId)\t\(c.colorLayoutId)\t\(i)\t\(p.text)\t\(p.back)")
            }
        }
        return out
    }
}

/// The SDS150 display-customization editor — a gear popup (like the FT-60 settings sheet). Pick a
/// layout mode, assign item tokens per screen area, and set text/background colors per element,
/// with a live preview mirroring the scanner screen. Writes go back to the card via `onWrite`.
struct DisplaySettingsSheet: View {
    @StateObject private var model: DisplayEditModel
    /// Whether the loaded source is a live card (vs a backup folder) — drives the write label and
    /// messaging. Writing to a backup is allowed (modify it, then Restore/write to a card later).
    let isLive: Bool
    let onWrite: ([String]) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var modeIndex = 0

    private var options: DisplayOptions { .shared }

    init(cardMount: String, isLive: Bool, onWrite: @escaping ([String]) -> Void) {
        _model = StateObject(wrappedValue: DisplayEditModel(cardMount: cardMount))
        self.isLive = isLive
        self.onWrite = onWrite
    }

    private var mode: DisplayMode {
        let m = options.modes
        return m.indices.contains(modeIndex) ? m[modeIndex] : DisplayMode(name: "—", dispLayoutId: 1, colorLayoutId: 1)
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(Theme.border)
            if !model.loaded {
                notLoaded
            } else {
                HStack(spacing: 0) {
                    preview.frame(width: 300)
                    Divider().overlay(Theme.border)
                    ScrollView { controls.padding(14) }
                }
            }
        }
        .frame(width: 780, height: 580)
        .background(Theme.panel)
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 12) {
            Image(systemName: "paintpalette")
                .font(.system(size: 15)).foregroundStyle(.white)
                .frame(width: 34, height: 34).background(Circle().fill(Theme.accent))
            VStack(alignment: .leading, spacing: 1) {
                Text("Customize display").font(.system(size: 14, weight: .semibold)).foregroundStyle(Theme.fg)
                Text(isLive
                    ? "Theme the scanner screen — written to the card"
                    : "Editing a backup — saved to the backup folder")
                    .font(.system(size: 10)).foregroundStyle(Theme.fg3)
            }
            Spacer()
            if model.loaded {
                Picker("", selection: $modeIndex) {
                    ForEach(Array(options.modes.enumerated()), id: \.offset) { i, m in
                        Text(m.name).tag(i)
                    }
                }.pickerStyle(.menu).labelsHidden().fixedSize()
            }
            Button(isLive ? "Write to Card" : "Save to Backup") { onWrite(model.editScript()); dismiss() }
                .buttonStyle(.borderedProminent).controlSize(.large).disabled(!model.loaded)
            Button("Done") { dismiss() }.controlSize(.large)
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
    }

    private var notLoaded: some View {
        VStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle").font(.system(size: 26)).foregroundStyle(Theme.warn)
            Text("No display config found").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.fg)
            Text("Open a scanner card (with a profile.cfg) first.").font(.system(size: 11)).foregroundStyle(Theme.fg3)
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Live preview (mirrors the scanner LCD structure, per mode family)

    /// The scan-family modes — no System/Dept/Channel names; they show Primary Area 1/2/3 instead
    /// (confirmed from the hardware screens).
    private var scanFamily: Bool {
        ["Search / Close Call", "Weather", "Tone Out"].contains(mode.name)
    }

    private var preview: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("PREVIEW · \(mode.name)").font(.system(size: 9, weight: .bold)).tracking(0.6)
                .foregroundStyle(Theme.fg3)
            VStack(alignment: .leading, spacing: 5) {
                statusBar
                Divider().overlay(.white.opacity(0.12))
                if scanFamily { scanBody } else { namedBody }
                Spacer(minLength: 4)
                bottomTags
            }
            .padding(10)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(RoundedRectangle(cornerRadius: 8).fill(Color.black))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.border))
        }
        .padding(14)
    }

    /// Top status row = Area 3 (small items: ATT, Date/Time, Slot, VOL&SQ, Info Areas…).
    private var statusBar: some View {
        Text(areaTokens(3).map { options.tokenLabel($0) }.joined(separator: "  "))
            .font(.system(size: 8)).foregroundStyle(regionColor(3).opacity(0.9)).lineLimit(2)
    }

    /// Bottom tag row = Area 4 (Modulation/IFX/LVL/REC/GPS/PRI/WX PRI…).
    private var bottomTags: some View {
        Text(areaTokens(4).map { options.tokenLabel($0) }.joined(separator: "  "))
            .font(.system(size: 8)).foregroundStyle(regionColor(4).opacity(0.9)).lineLimit(2)
    }

    /// Named modes — two columns like the scanner: System / Department / Channel Name (each with
    /// its Area-1 option beneath) on the left, the Area-2 field stack on the right.
    private var namedBody: some View {
        let names = ["System Name", "Department Name", "Channel Name"]
        let sizes: [CGFloat] = [14, 13, 16]
        let a1 = areaSlots(1)  // 3 slots aligned to the 3 name rows
        return HStack(alignment: .top, spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(0..<3, id: \.self) { i in
                    Text(names[i]).font(.system(size: sizes[i], weight: .semibold))
                        .foregroundStyle(nameColor(pair: i * 2)).lineLimit(1)
                    if i < a1.count, let tok = a1[i] {
                        Text(options.tokenLabel(tok)).font(.system(size: 8.5))
                            .foregroundStyle(nameColor(pair: i * 2).opacity(0.7)).lineLimit(1)
                    }
                }
            }
            Spacer(minLength: 6)
            detailColumn
        }
    }

    /// Scan modes — Primary Area 1/2/3 + Mode detail Area on the left, the Area-2 column on the right.
    private var scanBody: some View {
        let names = ["Primary Area 1", "Primary Area 2", "Primary Area 3"]
        return HStack(alignment: .top, spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(0..<3, id: \.self) { i in
                    Text(names[i]).font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(nameColor(pair: i)).lineLimit(1)
                }
                Text("Mode detail Area").font(.system(size: 9)).foregroundStyle(.white.opacity(0.5))
            }
            Spacer(minLength: 6)
            detailColumn
        }
    }

    /// Area 2 = the right-hand field column (Frequency, Service Type, IDs… / scan Batt·RSSI·Graph),
    /// one field per line — what makes the packed detail modes line up.
    private var detailColumn: some View {
        VStack(alignment: .trailing, spacing: 1) {
            ForEach(Array(areaTokens(2).enumerated()), id: \.offset) { _, tok in
                Text(options.tokenLabel(tok)).font(.system(size: 8.5))
                    .foregroundStyle(regionColor(2)).lineLimit(1)
            }
        }
    }

    /// Live tokens for an area (Empty dropped).
    private func areaTokens(_ dispOptId: Int) -> [String] {
        guard let gi = model.itemIndex(dispOptId: dispOptId, dispLayoutId: mode.dispLayoutId)
        else { return [] }
        return model.data.items[gi].tokens.filter { $0 != "Empty" }
    }

    /// Area tokens preserving slot position (Empty → nil) so they align under the name rows.
    private func areaSlots(_ dispOptId: Int) -> [String?] {
        guard let gi = model.itemIndex(dispOptId: dispOptId, dispLayoutId: mode.dispLayoutId)
        else { return [] }
        return model.data.items[gi].tokens.map { $0 == "Empty" ? nil : $0 }
    }

    /// Text color of a name element = `DispColorId=1` pair at `pair` (exact; falls back to white).
    private func nameColor(pair: Int) -> Color {
        if let ci = model.colorIndex(dispColorId: 1, colorLayoutId: mode.colorLayoutId),
            pair < model.data.colors[ci].pairs.count
        {
            return Color(displayHex: model.data.colors[ci].pairs[pair].text)
        }
        return .white
    }

    /// Representative color a screen region draws with, from its DispColors group's first pair:
    /// Area 2 (large) → group 4; Areas 3/4 (small) → group 3.
    private func regionColor(_ dispOptId: Int) -> Color {
        let groupId = dispOptId == 2 ? 4 : 3
        if let ci = model.colorIndex(dispColorId: groupId, colorLayoutId: mode.colorLayoutId),
            let p = model.data.colors[ci].pairs.first
        {
            return Color(displayHex: p.text)
        }
        return .white.opacity(0.8)
    }

    // MARK: - Controls

    private var controls: some View {
        VStack(alignment: .leading, spacing: 16) {
            section("GLOBALS") {
                ForEach(model.data.globals) { g in
                    row(g.label) {
                        Picker("", selection: globalBinding(g.key)) {
                            ForEach(g.options, id: \.self) { Text($0).tag($0) }
                        }.pickerStyle(.menu).labelsHidden().fixedSize()
                    }
                }
            }
            section("ITEMS · \(mode.name)") {
                ForEach([1, 2, 3, 4], id: \.self) { area in
                    if let gi = model.itemIndex(dispOptId: area, dispLayoutId: mode.dispLayoutId) {
                        Text(options.areaLabel(area)).font(.system(size: 10, weight: .semibold)).foregroundStyle(Theme.fg2)
                        ForEach(Array(model.data.items[gi].tokens.enumerated()), id: \.offset) { idx, tok in
                            row("Slot \(idx + 1)") {
                                Picker("", selection: tokenBinding(area: area, index: idx)) {
                                    ForEach(tokenOptions(area: area, current: tok), id: \.self) {
                                        Text(options.tokenLabel($0)).tag($0)
                                    }
                                }.pickerStyle(.menu).labelsHidden().fixedSize()
                            }
                        }
                    }
                }
            }
            section("COLORS · \(mode.name)") {
                ForEach(options.colorGroups, id: \.dispColorId) { grp in
                    if let ci = model.colorIndex(dispColorId: grp.dispColorId, colorLayoutId: mode.colorLayoutId) {
                        Text(elementSummary(grp)).font(.system(size: 10, weight: .semibold)).foregroundStyle(Theme.fg2)
                        ForEach(Array(model.data.colors[ci].pairs.enumerated()), id: \.offset) { idx, _ in
                            colorRow(group: grp, layout: mode.colorLayoutId, index: idx)
                        }
                    }
                }
            }
        }
    }

    private func colorRow(group: DisplayColorGroupInfo, layout: Int, index: Int) -> some View {
        let label = colorElementLabel(group: group, index: index)
        return HStack(spacing: 8) {
            Text(label).font(.system(size: 11)).foregroundStyle(Theme.fg3)
                .frame(width: 120, alignment: .leading)
            Spacer()
            palettePicker("Text", colorBinding(group: group.dispColorId, layout: layout, index: index, isText: true))
            palettePicker("Back", colorBinding(group: group.dispColorId, layout: layout, index: index, isText: false))
        }
    }

    private func palettePicker(_ role: String, _ binding: Binding<String>) -> some View {
        Menu {
            ForEach(options.palette, id: \.hex) { c in
                Button {
                    binding.wrappedValue = c.hex
                } label: {
                    Label(c.name, systemImage: "circle.fill")
                }
            }
        } label: {
            HStack(spacing: 4) {
                RoundedRectangle(cornerRadius: 3).fill(Color(displayHex: binding.wrappedValue))
                    .frame(width: 16, height: 16)
                    .overlay(RoundedRectangle(cornerRadius: 3).stroke(Theme.border))
                Text(role).font(.system(size: 9)).foregroundStyle(Theme.fg3)
            }
        }
        .menuStyle(.borderlessButton).fixedSize()
        .help("\(role): \(options.colorName(binding.wrappedValue))")
    }

    // MARK: - Bits

    private func section<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.system(size: 10, weight: .bold)).tracking(0.6).foregroundStyle(Theme.fg3)
            content()
        }
    }

    private func row<Content: View>(_ label: String, @ViewBuilder _ trailing: () -> Content) -> some View {
        HStack(spacing: 8) {
            Text(label).font(.system(size: 11.5)).foregroundStyle(Theme.fg2)
                .frame(width: 150, alignment: .leading)
            Spacer()
            trailing()
        }
    }

    private func elementSummary(_ grp: DisplayColorGroupInfo) -> String {
        grp.elements.prefix(3).joined(separator: ", ") + (grp.elements.count > 3 ? "…" : "")
    }

    /// The element label for a color pair. `DispColorId=1`'s elements differ by mode family
    /// (confirmed from the hardware screens): named modes paint System/Dept/Channel Name+Avoid;
    /// scan modes paint Primary Area 1/2/3 + Mode detail Area. Other groups use the core's nominal
    /// names; anything past the known list falls back to a numbered element.
    private func colorElementLabel(group: DisplayColorGroupInfo, index: Int) -> String {
        if group.dispColorId == 1 {
            let labels = scanFamily
                ? ["Primary Area 1", "Primary Area 2", "Primary Area 3", "Mode detail Area"]
                : ["System Name", "System Avoid", "Dept Name", "Dept Avoid", "Channel Name", "Channel Avoid"]
            if index < labels.count { return labels[index] }
        }
        return index < group.elements.count ? group.elements[index] : "Element \(index + 1)"
    }

    private func tokenOptions(area: Int, current: String) -> [String] {
        var opts = options.tokens(forArea: area)
        if !opts.contains(current) { opts.insert(current, at: 0) }
        return opts
    }

    // MARK: - Bindings into the model

    private func globalBinding(_ key: String) -> Binding<String> {
        Binding(
            get: { model.data.globals.first { $0.key == key }?.value ?? "" },
            set: { v in
                if let i = model.data.globals.firstIndex(where: { $0.key == key }) {
                    model.data.globals[i].value = v
                }
            })
    }

    private func tokenBinding(area: Int, index: Int) -> Binding<String> {
        Binding(
            get: {
                guard let gi = model.itemIndex(dispOptId: area, dispLayoutId: mode.dispLayoutId),
                    index < model.data.items[gi].tokens.count
                else { return "" }
                return model.data.items[gi].tokens[index]
            },
            set: { v in
                if let gi = model.itemIndex(dispOptId: area, dispLayoutId: mode.dispLayoutId),
                    index < model.data.items[gi].tokens.count
                {
                    model.data.items[gi].tokens[index] = v
                }
            })
    }

    private func colorBinding(group: Int, layout: Int, index: Int, isText: Bool) -> Binding<String> {
        Binding(
            get: {
                guard let ci = model.colorIndex(dispColorId: group, colorLayoutId: layout),
                    index < model.data.colors[ci].pairs.count
                else { return "000000" }
                let p = model.data.colors[ci].pairs[index]
                return isText ? p.text : p.back
            },
            set: { v in
                guard let ci = model.colorIndex(dispColorId: group, colorLayoutId: layout),
                    index < model.data.colors[ci].pairs.count
                else { return }
                if isText { model.data.colors[ci].pairs[index].text = v }
                else { model.data.colors[ci].pairs[index].back = v }
            })
    }
}
