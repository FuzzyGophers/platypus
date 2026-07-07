// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// The editable in-memory copy of a card's display customization, loaded over the FFI. Keeps the
/// originally-loaded data so a per-mode "Reset to defaults" can restore it.
final class DisplayEditModel: ObservableObject {
    @Published var data: DisplayConfigData
    let original: DisplayConfigData
    let loaded: Bool

    init(cardMount: String) {
        if let d = DisplayBridge.read(cardMount: cardMount) {
            data = d
            original = d
            loaded = true
        } else {
            let empty = DisplayConfigData(globals: [], items: [], colors: [])
            data = empty
            original = empty
            loaded = false
        }
    }

    func itemIndex(dispOptId: Int, dispLayoutId: Int) -> Int? {
        data.items.firstIndex { $0.dispOptId == dispOptId && $0.dispLayoutId == dispLayoutId }
    }

    func colorIndex(dispColorId: Int, colorLayoutId: Int) -> Int? {
        data.colors.firstIndex { $0.dispColorId == dispColorId && $0.colorLayoutId == colorLayoutId }
    }

    /// Restore the color + item groups that belong to `mode`'s layout ids from the loaded original.
    func resetMode(_ mode: DisplayMode) {
        for oi in original.items where oi.dispLayoutId == mode.dispLayoutId {
            if let i = data.items.firstIndex(where: { $0.dispOptId == oi.dispOptId && $0.dispLayoutId == oi.dispLayoutId }) {
                data.items[i].tokens = oi.tokens
            }
        }
        for oc in original.colors where oc.colorLayoutId == mode.colorLayoutId {
            if let i = data.colors.firstIndex(where: { $0.dispColorId == oc.dispColorId && $0.colorLayoutId == oc.colorLayoutId }) {
                data.colors[i].pairs = oc.pairs
            }
        }
        objectWillChange.send()
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

/// The SDS150 Customize Display editor — a 3-pane WYSIWYG (mode rail · pixel-accurate live screen ·
/// element inspector) mirroring the scanner's on-screen customization + the T-COLOR / B-COLOR
/// softkeys. Writes go back to the card (or a backup) via `onWrite`.
struct DisplaySettingsSheet: View {
    @StateObject private var model: DisplayEditModel
    /// A live card (vs a backup folder) — drives the write label + messaging. Writing to a backup
    /// is allowed (edit it, Restore/write to a card later).
    let isLive: Bool
    let onWrite: ([String]) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var modeIndex = 0
    @State private var selected: String?          // selected on-screen field key
    @State private var inspectorAll = true        // All elements ↔ Selected
    @State private var query = ""
    @State private var openGroups: Set<String> = []
    @State private var showGlobals = false

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
                    modeRail.frame(width: 210)
                    Divider().overlay(Theme.border)
                    stage.frame(maxWidth: .infinity)
                    Divider().overlay(Theme.border)
                    inspector.frame(width: 288)
                }
                Divider().overlay(Theme.border)
                footer
            }
        }
        .frame(width: 1180, height: 820)
        .background(Theme.bg)
        .foregroundStyle(Theme.fg)
        .preferredColorScheme(.dark)
        .sheet(isPresented: $showGlobals) {
            GlobalsSettingsSheet(model: model)
        }
    }

    // MARK: - Header

    private var header: some View {
        HStack(alignment: .top, spacing: 14) {
            VStack(alignment: .leading, spacing: 3) {
                Text("Customize Display").font(.system(size: 17, weight: .semibold))
                Text("Pick a display mode, click a field on the screen, then set its content and colors. Changes preview live and write to the scanner.")
                    .font(.system(size: 12)).foregroundStyle(Theme.fg3).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            Button { showGlobals = true } label: {
                Label("Settings", systemImage: "gearshape").font(.system(size: 12.5))
            }.controlSize(.regular)
        }
        .padding(.horizontal, 20).padding(.vertical, 14)
    }

    private var notLoaded: some View {
        VStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle").font(.system(size: 26)).foregroundStyle(Theme.warn)
            Text("No display config found").font(.system(size: 13, weight: .semibold))
            Text("Open a scanner card (with a profile.cfg) first.").font(.system(size: 11)).foregroundStyle(Theme.fg3)
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Left rail (7 modes grouped)

    private var modeRail: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 1) {
                ForEach(Array(DisplayScreenBuilder.groups.enumerated()), id: \.offset) { _, grp in
                    Text(grp.title.uppercased()).font(.system(size: 10.5, weight: .bold)).tracking(0.6)
                        .foregroundStyle(Theme.fg3).padding(.horizontal, 8).padding(.top, 12).padding(.bottom, 4)
                    ForEach(grp.modeNames, id: \.self) { name in
                        if let i = options.modes.firstIndex(where: { $0.name == name }) {
                            modeRow(index: i, name: name, sub: grp.subtitle)
                        }
                    }
                }
            }
            .padding(.horizontal, 8).padding(.bottom, 16)
        }
        .background(Theme.bg2)
    }

    private func modeRow(index: Int, name: String, sub: String?) -> some View {
        let on = index == modeIndex
        return Button {
            modeIndex = index; selected = nil
        } label: {
            HStack(spacing: 9) {
                Text("\(index + 1)").font(.system(size: 11, weight: .bold)).monospacedDigit()
                    .foregroundStyle(on ? .white.opacity(0.85) : Theme.fg3).frame(width: 16)
                VStack(alignment: .leading, spacing: 1.5) {
                    ForEach([LCDTier.orange, LCDTier.amber, LCDTier.yellow], id: \.self) { c in
                        RoundedRectangle(cornerRadius: 1).fill(c).frame(width: 20, height: 2)
                    }
                }
                .padding(4).background(RoundedRectangle(cornerRadius: 3).fill(Color(hex: 0x0b0c12)))
                VStack(alignment: .leading, spacing: 0) {
                    Text(name).font(.system(size: 12.5)).foregroundStyle(on ? .white : Theme.fg).lineLimit(1)
                    if let sub { Text(sub).font(.system(size: 10)).foregroundStyle(on ? .white.opacity(0.8) : Theme.fg3) }
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10).padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: 6).fill(on ? Theme.accent : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    // MARK: - Center stage (pixel LCD)

    private var stage: some View {
        VStack(spacing: 14) {
            Spacer(minLength: 0)
            LCDPixelView(screen: screen, scale: 1.5, selected: selected) { key in
                selected = key; inspectorAll = false
            }
            .padding(24)
            .background(RoundedRectangle(cornerRadius: 18).fill(LinearGradient(
                colors: [Color(hex: 0x2a2b2f), Color(hex: 0x161619)], startPoint: .top, endPoint: .bottom)))
            .overlay(alignment: .topLeading) {
                Text("uniden").font(.system(size: 10, weight: .heavy)).tracking(1).foregroundStyle(Color(hex: 0x6f7076))
                    .padding(.leading, 26).padding(.top, 10)
            }
            HStack(spacing: 7) {
                Image(systemName: "hand.tap").font(.system(size: 12)).foregroundStyle(Theme.fg3)
                Text("Click a colored field to edit it. Dashed outline = current selection.")
                    .font(.system(size: 11.5)).foregroundStyle(Theme.fg3)
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(StripeBackground())
    }

    private var screen: LCDScreen {
        DisplayScreenBuilder.build(mode: mode, model: model, selected: selected)
    }

    // MARK: - Right inspector

    private var inspector: some View {
        VStack(spacing: 0) {
            Picker("", selection: $inspectorAll) {
                Text("All elements").tag(true)
                Text("Selected").tag(false)
            }.pickerStyle(.segmented).labelsHidden().padding(14)
            Divider().overlay(Theme.border)
            ScrollView {
                if inspectorAll { allElements } else { selectedEditor }
            }
        }
        .background(Theme.panel)
    }

    // All elements: every real DispColors group's pairs (Text/Back swatches) + the item-area slots.
    private var allElements: some View {
        VStack(alignment: .leading, spacing: 0) {
            SearchField(text: $query, placeholder: "Search elements…").padding(.horizontal, 14).padding(.vertical, 10)
            ForEach(options.colorGroups, id: \.dispColorId) { grp in
                if let ci = model.colorIndex(dispColorId: grp.dispColorId, colorLayoutId: mode.colorLayoutId) {
                    let rows = colorRows(grp, colorIndex: ci)
                    if !rows.isEmpty {
                        collapsibleGroup(title: colorGroupTitle(grp.dispColorId), count: rows.count) {
                            ForEach(rows, id: \.self) { idx in colorElementRow(grp: grp, colorIndex: ci, index: idx) }
                        }
                    }
                }
            }
            ForEach([1, 2, 3, 4], id: \.self) { area in
                if let gi = model.itemIndex(dispOptId: area, dispLayoutId: mode.dispLayoutId) {
                    let slots = itemSlots(gi)
                    if !slots.isEmpty {
                        collapsibleGroup(title: "Items · \(options.areaLabel(area))", count: slots.count) {
                            ForEach(slots, id: \.self) { idx in itemSlotRow(area: area, groupIndex: gi, index: idx) }
                        }
                    }
                }
            }
        }
        .padding(.bottom, 16)
    }

    private func colorRows(_ grp: DisplayColorGroupInfo, colorIndex ci: Int) -> [Int] {
        Array(model.data.colors[ci].pairs.indices).filter { idx in
            !isSpacer(grp, idx)
                && (query.isEmpty || colorElementLabel(group: grp, index: idx).localizedCaseInsensitiveContains(query))
        }
    }

    /// A spec spacer/separator slot (`SP0`/`SP1`/`SP2`) — carries a color pair but paints no visible
    /// element, so it's hidden from the inspector (still preserved verbatim in the file on write).
    private func isSpacer(_ group: DisplayColorGroupInfo, _ index: Int) -> Bool {
        guard index < group.elements.count else { return false }
        let e = group.elements[index]
        return e.hasPrefix("SP") && !e.dropFirst(2).isEmpty && e.dropFirst(2).allSatisfy(\.isNumber)
    }

    private func itemSlots(_ gi: Int) -> [Int] {
        Array(model.data.items[gi].tokens.indices).filter { idx in
            query.isEmpty || options.tokenLabel(model.data.items[gi].tokens[idx]).localizedCaseInsensitiveContains(query)
        }
    }

    private func colorElementRow(grp: DisplayColorGroupInfo, colorIndex ci: Int, index: Int) -> some View {
        HStack(spacing: 10) {
            Text(colorElementLabel(group: grp, index: index)).font(.system(size: 12.5)).foregroundStyle(Theme.fg)
                .lineLimit(1)
            Spacer(minLength: 6)
            SwatchChip(hex: colorBinding(group: grp.dispColorId, index: index, isText: true), palette: options.palette)
            SwatchChip(hex: colorBinding(group: grp.dispColorId, index: index, isText: false), palette: options.palette, allowsOff: true)
        }
        .padding(.horizontal, 14).padding(.vertical, 5)
    }

    private func itemSlotRow(area: Int, groupIndex gi: Int, index: Int) -> some View {
        HStack(spacing: 8) {
            Text("Slot \(index + 1)").font(.system(size: 12)).foregroundStyle(Theme.fg2).frame(width: 54, alignment: .leading)
            Spacer(minLength: 4)
            Picker("", selection: tokenBinding(area: area, index: index)) {
                ForEach(tokenOptions(area: area, current: model.data.items[gi].tokens[index]), id: \.self) {
                    Text(options.tokenLabel($0)).tag($0)
                }
            }.pickerStyle(.menu).labelsHidden().fixedSize()
        }
        .padding(.horizontal, 14).padding(.vertical, 3)
    }

    private func collapsibleGroup<C: View>(title: String, count: Int, @ViewBuilder _ content: () -> C) -> some View {
        let open = query.isEmpty ? !openGroups.contains(title) : true
        return VStack(alignment: .leading, spacing: 0) {
            Button {
                if openGroups.contains(title) { openGroups.remove(title) } else { openGroups.insert(title) }
            } label: {
                HStack(spacing: 7) {
                    Image(systemName: "chevron.right").font(.system(size: 10, weight: .bold))
                        .rotationEffect(.degrees(open ? 90 : 0)).foregroundStyle(Theme.fg3)
                    Text(title.uppercased()).font(.system(size: 11, weight: .bold)).tracking(0.4).foregroundStyle(Theme.fg3)
                    Spacer()
                    Text("\(count)").font(.system(size: 11)).foregroundStyle(Theme.fg3)
                }
                .padding(.horizontal, 14).padding(.vertical, 9).contentShape(Rectangle())
            }.buttonStyle(.plain)
            if open { content() }
            Divider().overlay(Theme.border)
        }
    }

    // Selected: the single-field color editor for a tier field bound to a real color pair.
    private var selectedEditor: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let key = selected, let field = screen.allFields.first(where: { $0.id == key }), let pair = field.colorPair {
                Text("EDITING FIELD").font(.system(size: 10.5, weight: .bold)).tracking(0.6).foregroundStyle(Theme.fg3)
                Text(colorPairLabel(field)).font(.system(size: 16, weight: .semibold))
                swatchGrid(title: "Text color", sk: "T-COLOR",
                           binding: colorBinding(group: pair.dispColorId, index: pair.index, isText: true))
                swatchGrid(title: "Background", sk: "B-COLOR", allowsOff: true,
                           binding: colorBinding(group: pair.dispColorId, index: pair.index, isText: false))
                tierLegend
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Click a field on the screen — or pick one under **All elements** — to edit its content and colors.")
                        .font(.system(size: 12)).foregroundStyle(Theme.fg3).fixedSize(horizontal: false, vertical: true)
                    Text("The **T-COLOR** and **B-COLOR** softkeys map to the Text and Background pickers.")
                        .font(.system(size: 12)).foregroundStyle(Theme.fg3).fixedSize(horizontal: false, vertical: true)
                }
                tierLegend
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
    }

    private func swatchGrid(title: String, sk: String, allowsOff: Bool = false, binding: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(title).font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.fg2)
                Spacer()
                Text(sk).font(.custom(LCDFonts.small, size: 9)).foregroundStyle(Theme.fg3)
                    .padding(.horizontal, 5).padding(.vertical, 1)
                    .overlay(RoundedRectangle(cornerRadius: 3).stroke(Theme.border2))
            }
            PaletteGrid(selectedHex: binding.wrappedValue, palette: options.palette, allowsOff: allowsOff) {
                binding.wrappedValue = $0
            }
        }
    }

    private var tierLegend: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("TIER COLOR HIERARCHY").font(.system(size: 10.5, weight: .bold)).tracking(0.6).foregroundStyle(Theme.fg3)
            legendRow(LCDTier.orange, "System / Primary Area 1–2")
            legendRow(LCDTier.amber, "Department / Site")
            legendRow(LCDTier.yellow, "Channel / Frequency / TGID")
            legendRow(LCDTier.red, "Labels & service type")
        }
        .padding(.top, 6)
    }

    private func legendRow(_ c: Color, _ label: String) -> some View {
        HStack(spacing: 9) {
            RoundedRectangle(cornerRadius: 3).fill(c).frame(width: 12, height: 12)
            Text(label).font(.system(size: 11.5)).foregroundStyle(Theme.fg2)
        }
    }

    // MARK: - Footer

    private var footer: some View {
        HStack(spacing: 10) {
            Button { model.resetMode(mode) } label: { Label("Reset mode to defaults", systemImage: "arrow.counterclockwise") }
                .buttonStyle(.plain).foregroundStyle(Theme.fg2)
            Spacer()
            Button("Cancel") { dismiss() }
            Button("Apply") { onWrite(model.editScript()) }
            Button(isLive ? "Write to radio" : "Save to backup") { onWrite(model.editScript()); dismiss() }
                .buttonStyle(.borderedProminent)
        }
        .controlSize(.large)
        .padding(.horizontal, 18).padding(.vertical, 11)
        .background(Theme.titlebar)
    }

    // MARK: - Labels + bindings

    private func colorGroupTitle(_ id: Int) -> String {
        switch id {
        case 1: return "Names & Avoid"
        case 2: return "Name options"
        case 3: return "Small-area items"
        case 4: return "Large-area items"
        case 5: return "Icons"
        case 6: return "Status flags"
        case 7: return "Soft keys"
        default: return "Group \(id)"
        }
    }

    /// The item token assigned to `(area, index)` for the current mode, or nil if empty/absent.
    private func itemToken(area: Int, index: Int) -> String? {
        guard let gi = model.itemIndex(dispOptId: area, dispLayoutId: mode.dispLayoutId),
              index < model.data.items[gi].tokens.count else { return nil }
        let t = model.data.items[gi].tokens[index]
        return t == "Empty" ? nil : t
    }

    private func colorPairLabel(_ field: LCDField) -> String {
        guard let pair = field.colorPair,
              let grp = options.colorGroups.first(where: { $0.dispColorId == pair.dispColorId })
        else { return field.text }
        return colorElementLabel(group: grp, index: pair.index)
    }

    /// The element label for a color pair. `DispColorId=1`'s elements differ by mode family (named
    /// modes paint System/Dept/Channel Name+Avoid; scan modes paint Primary Area 1/2/3 + Mode detail).
    private func colorElementLabel(group: DisplayColorGroupInfo, index: Int) -> String {
        func nth(_ a: [String]) -> String? { index >= 0 && index < a.count ? a[index] : nil }
        let gid = group.dispColorId
        if gid == 1 {
            let labels = DisplayScreenBuilder.isScan(mode.name)
                ? ["Primary Area 1", "Primary Area 2", "Primary Area 3", "Mode detail Area"]
                : ["System Name", "System Avoid", "Dept Name", "Dept Avoid", "Channel Name", "Channel Avoid"]
            if let l = nth(labels) { return l }
        }
        // Groups 2/3/4 paint item-area slots — name them by the real token where one is assigned
        // (Huge=area 1, Small=area 3, Large=area 2), so you see "Frequency"/"ATT" not "Option_1".
        if let area = [2: 1, 3: 3, 4: 2][gid], let tok = itemToken(area: area, index: index) {
            return options.tokenLabel(tok)
        }
        switch gid {
        case 2: return nth(["System option", "Dept option", "Channel option"]) ?? "Name option \(index + 1)"
        case 3: return "Small item \(index + 1)"
        case 4: return "Large item \(index + 1)"
        case 5: return "Icon \(index + 1)"
        case 6: return nth(["Function (F)", "Signal (SIG)", "Battery (BAT)", "Spacer (SP0)", "Key lock (KEY)"]) ?? "Status \(index + 1)"
        case 7: return nth(["Soft key 1", "Separator", "Soft key 2", "Separator", "Soft key 3"]) ?? "Soft key \(index + 1)"
        default: return nth(group.elements) ?? "Element \(index + 1)"
        }
    }

    private func tokenOptions(area: Int, current: String) -> [String] {
        var opts = options.tokens(forArea: area)
        if !opts.contains(current) { opts.insert(current, at: 0) }
        return opts
    }

    private func tokenBinding(area: Int, index: Int) -> Binding<String> {
        Binding(
            get: {
                guard let gi = model.itemIndex(dispOptId: area, dispLayoutId: mode.dispLayoutId),
                      index < model.data.items[gi].tokens.count else { return "" }
                return model.data.items[gi].tokens[index]
            },
            set: { v in
                if let gi = model.itemIndex(dispOptId: area, dispLayoutId: mode.dispLayoutId),
                   index < model.data.items[gi].tokens.count { model.data.items[gi].tokens[index] = v }
            })
    }

    private func colorBinding(group: Int, index: Int, isText: Bool) -> Binding<String> {
        Binding(
            get: {
                guard let ci = model.colorIndex(dispColorId: group, colorLayoutId: mode.colorLayoutId),
                      index < model.data.colors[ci].pairs.count else { return "000000" }
                let p = model.data.colors[ci].pairs[index]
                return isText ? p.text : p.back
            },
            set: { v in
                guard let ci = model.colorIndex(dispColorId: group, colorLayoutId: mode.colorLayoutId),
                      index < model.data.colors[ci].pairs.count else { return }
                if isText { model.data.colors[ci].pairs[index].text = v } else { model.data.colors[ci].pairs[index].back = v }
            })
    }
}

/// A diagonal-stripe backdrop for the preview stage (matches the prototype).
private struct StripeBackground: View {
    var body: some View {
        Canvas { ctx, size in
            ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Color(hex: 0x1b1b20)))
            var x: CGFloat = -size.height
            while x < size.width {
                var p = Path()
                p.move(to: CGPoint(x: x, y: size.height)); p.addLine(to: CGPoint(x: x + size.height, y: 0))
                ctx.stroke(p, with: .color(Color(hex: 0x1d1d23)), lineWidth: 12)
                x += 24
            }
        }
    }
}
