// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

// Support views + the per-mode screen builder for the Customize Display editor.

// MARK: - Screen builder (mode + model → LCDScreen)

/// Builds the laid-out `LCDScreen` for a mode from the live model. The name tiers (DispColorId=1)
/// are colored from the card and bound to real color pairs (editable); other body text uses the
/// reference tier colors (display-only — every real color pair is still editable under All elements).
enum DisplayScreenBuilder {
    struct Group { let title: String; let modeNames: [String]; let subtitle: String? }
    static let groups: [Group] = [
        Group(title: "Simple", modeNames: ["Simple Conventional", "Simple Trunk"], subtitle: nil),
        Group(title: "Detail", modeNames: ["Detail Conventional", "Detail Trunk"], subtitle: nil),
        Group(title: "Other", modeNames: ["Search / Close Call", "Weather", "Tone Out"], subtitle: "Primary areas"),
    ]

    static func isScan(_ name: String) -> Bool {
        ["Search / Close Call", "Weather", "Tone Out"].contains(name)
    }

    static func build(mode: DisplayMode, model: DisplayEditModel, selected: String?) -> LCDScreen {
        let scan = isScan(mode.name)
        let dense = !mode.name.hasPrefix("Simple")
        let blankMeta = mode.name == "Weather" || mode.name == "Tone Out"
        let layout = mode.colorLayoutId
        let dispLayout = mode.dispLayoutId

        // --- Real color-pair + item-token accessors for this mode ---
        func pairs(_ gid: Int) -> [DisplayColorPair]? {
            model.colorIndex(dispColorId: gid, colorLayoutId: layout).map { model.data.colors[$0].pairs }
        }
        func tcolor(_ gid: Int, _ idx: Int, _ fallback: Color) -> Color {
            if let ps = pairs(gid), idx < ps.count { return Color(displayHex: ps[idx].text) }
            return fallback
        }
        func bcolor(_ gid: Int, _ idx: Int) -> Color? {
            if let ps = pairs(gid), idx < ps.count { let b = ps[idx].back; return b == "000000" ? nil : Color(displayHex: b) }
            return nil
        }
        func pref(_ gid: Int, _ idx: Int) -> (dispColorId: Int, index: Int)? {
            if let ps = pairs(gid), idx < ps.count { return (gid, idx) }
            return nil
        }
        /// The token label for item area `area`, slot `i` (Empty → a dim placeholder).
        func tokenText(_ area: Int, _ i: Int) -> (text: String, dim: Bool) {
            guard let gi = model.itemIndex(dispOptId: area, dispLayoutId: dispLayout) else { return ("", true) }
            let ts = model.data.items[gi].tokens
            guard i < ts.count else { return ("", true) }
            return ts[i] == "Empty" ? ("\u{2014}\u{2014}\u{2014}", true) : (DisplayOptions.shared.tokenLabel(ts[i]), false)
        }
        /// The non-empty token labels for an item area (optionally skipping the first `n` slots).
        func areaLabels(_ area: Int, dropFirst n: Int = 0) -> [String] {
            guard let gi = model.itemIndex(dispOptId: area, dispLayoutId: dispLayout) else { return [] }
            return model.data.items[gi].tokens.dropFirst(n).filter { $0 != "Empty" }.map { DisplayOptions.shared.tokenLabel($0) }
        }
        /// Pair an item area's non-empty tokens into 2-column grid rows (for the detail/scan data grid).
        func gridRows(_ area: Int, dropFirst n: Int = 0) -> [(String, String)] {
            let toks = areaLabels(area, dropFirst: n)
            return stride(from: 0, to: toks.count, by: 2).map { i in
                (toks[i], i + 1 < toks.count ? toks[i + 1] : "")
            }
        }

        var body: [LCDField] = []
        if scan {
            // Primary Areas 1-3 + Mode detail = group 1 (0-3). (Grid/status colors are a later pass.)
            body = [
                LCDField(id: "pa1", kind: .xl, text: "Primary Area 1", color: tcolor(1, 0, LCDTier.orange), back: bcolor(1, 0), colorPair: pref(1, 0)),
                LCDField(id: "pa2", kind: .xl, text: "Primary Area 2", color: tcolor(1, 1, LCDTier.orange), back: bcolor(1, 1), colorPair: pref(1, 1)),
                LCDField(id: "pa3", kind: .xl, text: "Primary Area 3", color: tcolor(1, 2, LCDTier.yellow), back: bcolor(1, 2), colorPair: pref(1, 2)),
                LCDField(id: "srow", kind: .srow, text: "CTCSS/DCS/NAC", color: LCDTier.yellow, avoidColor: LCDTier.yellow, hold: true),
                LCDField(id: "modedet", kind: .svc, text: "Mode detail Area", color: tcolor(1, 3, LCDTier.yellow), colorPair: pref(1, 3)),
                dataGrid(gridRows(2)),  // scan data grid = the Large-area items
            ]
        } else {
            // Three name rows: Name (group1 even) + option line = real Huge item (group2) with the
            // AVOID tag (group1 odd) — data-driven, so every one recolors live.
            let names = ["System Name", "Department Name", "Channel Name"]
            let nameIds = ["system", "dept", "channel"]
            let optIds = ["sysopt", "deptopt", "chanopt"]
            let nameFallback = [LCDTier.orange, LCDTier.amber, LCDTier.yellow]
            for i in 0..<3 {
                body.append(LCDField(id: nameIds[i], kind: .xl, text: names[i],
                    color: tcolor(1, 2 * i, nameFallback[i]), back: bcolor(1, 2 * i), colorPair: pref(1, 2 * i)))
                let opt = tokenText(1, i)
                body.append(LCDField(id: optIds[i], kind: .meta, text: opt.text.isEmpty ? "\u{2014}\u{2014}\u{2014}" : opt.text,
                    color: tcolor(2, i, LCDTier.red),
                    avoid: true, avoidColor: tcolor(1, 2 * i + 1, LCDTier.red), dim: opt.dim,
                    colorPair: pref(2, i), avoidPair: pref(1, 2 * i + 1)))
            }
            // Service line = Large-area items 0/1 (group 4: Option A_1 / Option_B_1).
            let s0 = tokenText(2, 0), s1 = tokenText(2, 1)
            body.append(LCDField(id: "service", kind: .svc,
                text: s0.text.isEmpty ? "Service Type" : s0.text, color: tcolor(4, 0, LCDTier.red),
                right: s1.text.isEmpty ? "CTCSS/DCS/NAC" : s1.text, rightColor: tcolor(4, 1, LCDTier.yellow),
                colorPair: pref(4, 0), rightPair: pref(4, 1)))
            if dense {  // Detail modes add the two-column data grid = remaining Large-area items.
                let rows = gridRows(2, dropFirst: 2)
                if !rows.isEmpty { body.append(dataGrid(rows)) }
            }
        }

        // Colorable chrome (groups 3/5/6/7), each element bound to its real DispColors pair so it
        // recolors live and is clickable. Placement in the status/softkey regions is representative.
        func groupList(_ gid: Int, _ names: [String], _ id: String) -> [LCDField] {
            guard let ps = pairs(gid) else { return [] }
            return ps.indices.map { i in
                LCDField(id: "\(id)\(i)", kind: .meta,
                         text: i < names.count ? names[i] : "\(id.uppercased())\(i + 1)",
                         color: Color(displayHex: ps[i].text), colorPair: (gid, i))
            }
        }
        let smallTokens: [LCDField] = {
            guard let gi = model.itemIndex(dispOptId: 3, dispLayoutId: dispLayout) else { return [] }
            return model.data.items[gi].tokens.enumerated().compactMap { i, t in
                t == "Empty" ? nil : LCDField(id: "g3_\(i)", kind: .meta, text: DisplayOptions.shared.tokenLabel(t),
                                              color: tcolor(3, i, LCDTier.white), colorPair: pref(3, i))
            }
        }()
        let flags = groupList(6, ["F", "SIG", "BAT", "SP0", "KEY"], "g6_")
        let iconEls = groupList(5, ["IC1", "IC2", "IC3", "IC4", "IC5"], "g5_")
        // The three visible soft keys are group-7 pairs 0/2/4 (1/3 are SP separators).
        let softkeys: [LCDField] = [0, 2, 4].compactMap { i in
            guard let ps = pairs(7), i < ps.count else { return nil }
            return LCDField(id: "g7_\(i)", kind: .meta, text: "Soft \(i / 2 + 1)",
                            color: Color(displayHex: ps[i].text), colorPair: (7, i))
        }

        // Bottom indicator row = the Small-lower items (DispOptId=4); fall back to representative
        // chrome only when the card has no area-4 record for this mode.
        let lower = areaLabels(4)
        let indicatorCells: [(text: String, box: Bool, dim: Bool)] =
            lower.isEmpty ? indicator(mode.name) : lower.map { (text: $0, box: false, dim: false) }

        return LCDScreen(
            family: scan ? .scan : .named,
            detailStatus: !mode.name.hasPrefix("Simple"),
            dense: dense,
            blankMeta: blankMeta,
            numberTag: !scan,
            body: body,
            statusTokens: smallTokens,
            statusFlags: flags,
            icons: iconEls,
            softkeys: softkeys,
            indicator: indicatorCells,
            hold: scan)
    }

    private static func dataGrid(_ rows: [(String, String)]) -> LCDField {
        LCDField(id: "data", kind: .grid, text: "", color: LCDTier.orange, gridRows: rows)
    }

    private static func indicator(_ name: String) -> [(text: String, box: Bool, dim: Bool)] {
        if name == "Search / Close Call" {
            return [("SCR", false, false), ("REP", false, false), ("IFX", false, false), ("V+0", false, false),
                    ("\u{2014}\u{2014}\u{2014}", false, true), ("REC", false, false), ("GPS", false, false),
                    ("PRI", false, false), ("\u{25C8}", false, false), ("WX", false, false)]
        }
        if name == "Weather" || name == "Tone Out" {
            return [("\u{2014}\u{2014}", false, true), ("IFX", false, false), ("V+0", false, false),
                    ("\u{2014}\u{2014}\u{2014}", false, true), ("REC", false, false), ("GPS", false, false),
                    ("\u{2014}\u{2014}\u{2014}", false, true)]
        }
        return [("NFM", false, false), ("P", true, false), ("IFX", false, false), ("V+0", false, false),
                ("\u{2014}\u{2014}\u{2014}", false, true), ("REC", false, false), ("GPS", false, false),
                ("PRI", false, false), ("\u{25C8}", false, false), ("WX", false, false)]
    }
}

// MARK: - Search field

/// A compact inset search field (the app has no shared one; used by the element inspector).
struct SearchField: View {
    @Binding var text: String
    var placeholder: String
    var body: some View {
        HStack(spacing: 7) {
            Image(systemName: "magnifyingglass").font(.system(size: 11)).foregroundStyle(Theme.fg3)
            TextField(placeholder, text: $text).textFieldStyle(.plain).font(.system(size: 12.5))
            if !text.isEmpty {
                Button { text = "" } label: { Image(systemName: "xmark.circle.fill").font(.system(size: 11)) }
                    .buttonStyle(.plain).foregroundStyle(Theme.fg3)
            }
        }
        .padding(.horizontal, 9).padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: 6).fill(Theme.bg3))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(Theme.border))
    }
}

// MARK: - Color swatches

/// One live color chip that opens the palette picker; edits its bound 6-hex value.
struct SwatchChip: View {
    @Binding var hex: String
    let palette: [PaletteColor]
    var allowsOff = false
    @State private var show = false

    var body: some View {
        Button { show.toggle() } label: {
            SwatchFill(hex: hex, allowsOff: allowsOff)
                .frame(width: 30, height: 18)
                .clipShape(RoundedRectangle(cornerRadius: 4))
                .overlay(RoundedRectangle(cornerRadius: 4).stroke(.white.opacity(0.28)))
        }
        .buttonStyle(.plain)
        .popover(isPresented: $show, arrowEdge: .bottom) {
            PaletteGrid(selectedHex: hex, palette: palette, allowsOff: allowsOff) { hex = $0; show = false }
                .frame(width: 250, height: 300).padding(10).background(Theme.panel)
        }
    }
}

/// A palette-backed swatch grid (147 colors + optional Off). Used inline in the Selected editor and
/// inside `SwatchChip`'s popover.
struct PaletteGrid: View {
    let selectedHex: String
    let palette: [PaletteColor]
    var allowsOff = false
    let onPick: (String) -> Void

    private let cols = Array(repeating: GridItem(.fixed(24), spacing: 6), count: 7)

    var body: some View {
        ScrollView {
            LazyVGrid(columns: cols, spacing: 6) {
                if allowsOff { cell(hex: "000000", name: "Off", off: true) }
                ForEach(palette, id: \.hex) { c in cell(hex: c.hex, name: c.name, off: false) }
            }
        }
    }

    private func cell(hex: String, name: String, off: Bool) -> some View {
        Button { onPick(hex) } label: {
            SwatchFill(hex: hex, allowsOff: off)
                .frame(width: 24, height: 24)
                .clipShape(RoundedRectangle(cornerRadius: 5))
                .overlay(RoundedRectangle(cornerRadius: 5)
                    .stroke(selectedHex.caseInsensitiveCompare(hex) == .orderedSame ? Theme.accent : .white.opacity(0.14),
                            lineWidth: selectedHex.caseInsensitiveCompare(hex) == .orderedSame ? 2 : 1))
        }
        .buttonStyle(.plain).help(name)
    }
}

/// A swatch fill — the palette color, or a checker for the "Off" background.
struct SwatchFill: View {
    let hex: String
    var allowsOff = false
    var body: some View {
        if allowsOff && hex.caseInsensitiveCompare("000000") == .orderedSame {
            Canvas { ctx, size in
                ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Color(hex: 0x16161a)))
                let s: CGFloat = 5
                var y: CGFloat = 0
                var row = 0
                while y < size.height {
                    var x: CGFloat = (row % 2 == 0) ? 0 : s
                    while x < size.width {
                        ctx.fill(Path(CGRect(x: x, y: y, width: s, height: s)), with: .color(Color(hex: 0x2a2a2e)))
                        x += s * 2
                    }
                    y += s; row += 1
                }
            }
        } else {
            Color(displayHex: hex)
        }
    }
}

// MARK: - Globals (Settings) window

/// The small Settings window opened from the editor's gear — the SDS150 global display options
/// (Motorola TGID format, Simple mode, Color mode, Squelch/Key light…) as native pop-ups.
struct GlobalsSettingsSheet: View {
    @ObservedObject var model: DisplayEditModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Settings").font(.system(size: 15, weight: .semibold))
                Spacer()
            }
            .padding(.horizontal, 18).padding(.vertical, 14)
            Divider().overlay(Theme.border)
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    Text("GLOBALS").font(.system(size: 10.5, weight: .bold)).tracking(0.6).foregroundStyle(Theme.fg3)
                    VStack(spacing: 0) {
                        ForEach(Array(model.data.globals.enumerated()), id: \.element.id) { i, g in
                            if i > 0 { Divider().overlay(Theme.border) }
                            HStack {
                                Text(g.label).font(.system(size: 12.5)).foregroundStyle(Theme.fg)
                                Spacer()
                                Picker("", selection: globalBinding(g.key)) {
                                    ForEach(g.options, id: \.self) { Text($0).tag($0) }
                                }.pickerStyle(.menu).labelsHidden().fixedSize()
                            }
                            .padding(.horizontal, 12).padding(.vertical, 9)
                        }
                    }
                    .background(RoundedRectangle(cornerRadius: Theme.rCard).fill(Theme.panel))
                    .overlay(RoundedRectangle(cornerRadius: Theme.rCard).stroke(Theme.border))
                }
                .padding(18)
            }
            Divider().overlay(Theme.border)
            HStack {
                Spacer()
                Button("Done") { dismiss() }.buttonStyle(.borderedProminent)
            }
            .controlSize(.large).padding(.horizontal, 18).padding(.vertical, 12).background(Theme.titlebar)
        }
        .frame(width: 460, height: 440)
        .background(Theme.bg)
        .foregroundStyle(Theme.fg)
        .preferredColorScheme(.dark)
    }

    private func globalBinding(_ key: String) -> Binding<String> {
        Binding(
            get: { model.data.globals.first { $0.key == key }?.value ?? "" },
            set: { v in if let i = model.data.globals.firstIndex(where: { $0.key == key }) { model.data.globals[i].value = v } })
    }
}
