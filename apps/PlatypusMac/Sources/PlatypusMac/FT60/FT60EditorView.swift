// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

/// Drives the channel-form sheet: add a new channel, or edit an existing one.
enum FT60SheetMode: Identifiable {
    case add
    case edit(FT60Channel)
    var id: Int { if case .edit(let c) = self { return c.slot } else { return -1 } }
    var channel: FT60Channel? { if case .edit(let c) = self { return c } else { return nil } }
}

/// The FT-60 target editor (right column) — the clone-image analog of the favorites
/// editor. One global memory (000–999) shown as collapsible sections: **All Memories**,
/// each **Bank A–J**, and **Unbanked**. A channel can be in several banks, so it appears
/// under each of its banks (member-only bank chips on the row show membership at a glance).
/// Location-first discovery (left filters + center map/drill-down) feeds channels in; this
/// is the sink. Channels come from a serial Read of the radio (or hand-added from the catalog).
struct FT60EditorView: View {
    @ObservedObject var memory: FT60Memory
    /// The active radio's display name (from the core registry) — the editor is generic over
    /// clone-image radios, so nothing here is hardcoded to a specific model.
    let radioName: String
    /// The bank location-first adds land in (nil = Unbanked / All Memories). Also the
    /// highlighted "add target" section.
    @Binding var selectedBank: Int?
    var onRead: () -> Void
    var onWrite: () -> Void

    @State private var detailsSlot: Int?
    /// The channel-form sheet: add a new channel, or edit an existing one.
    @State private var sheet: FT60SheetMode?
    /// Collapsed section keys ("all", "b0"…"b9", "unbanked"); absent = expanded.
    @State private var collapsed: Set<String> = []

    var body: some View {
        VStack(spacing: 0) {
            headerBar
            Divider().overlay(Theme.border)
            if memory.channels.isEmpty {
                emptyState
            } else {
                ScrollView {
                    VStack(spacing: 10) {
                        section(key: "all", title: "All Memories", bank: nil,
                                rows: memory.channels.sorted { $0.slot < $1.slot })
                        ForEach(0..<memory.capacity.banks, id: \.self) { b in
                            section(key: "b\(b)", title: "Bank \(FT60Memory.bankLabel(b))", bank: b,
                                    rows: memory.channels(inBank: b).sorted { $0.slot < $1.slot })
                        }
                        section(key: "unbanked", title: "Unbanked", bank: nil,
                                rows: memory.unbanked.sorted { $0.slot < $1.slot }, isUnbanked: true)
                        if !memory.pms.isEmpty { pmsSection }
                    }
                    .padding(8)
                }
            }
            Divider().overlay(Theme.border)
            footer
        }
        .sheet(item: $sheet) { mode in
            FT60ChannelSheet(nameLimit: memory.capacity.nameLen, editing: mode.channel) { make in
                if let ed = mode.channel {
                    memory.update(make(ed.slot))
                } else {
                    memory.append(make, toBank: selectedBank)
                }
            }
        }
    }

    /// Header subtitle reflecting real state: empty, or "N channels".
    private var subtitle: String {
        let n = memory.channels.count
        return n == 0 ? "clone · empty" : "clone · \(n) channel\(n == 1 ? "" : "s")"
    }

    /// Shown when the memory has no channels — invites a Read (or hand-add) instead of
    /// rendering a dozen empty sections.
    private var emptyState: some View {
        VStack(spacing: 12) {
            Spacer()
            Image(systemName: "antenna.radiowaves.left.and.right")
                .font(.system(size: 30)).foregroundStyle(Theme.fg3)
            Text("No channels yet").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.fg2)
            Text("Read the radio to load its memory, or add channels by hand from the catalog.")
                .font(.system(size: 11)).foregroundStyle(Theme.fg3)
                .multilineTextAlignment(.center).padding(.horizontal, 24)
            Button(action: onRead) {
                Label("Read from \(radioName)", systemImage: "square.and.arrow.down")
            }.controlSize(.small).padding(.top, 2)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Header (device + clone actions)

    private var headerBar: some View {
        HStack(spacing: 8) {
            Image(systemName: "dot.radiowaves.left.and.right").foregroundStyle(Theme.accent)
            VStack(alignment: .leading, spacing: 0) {
                Text(radioName).font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.fg)
                Text(subtitle).font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
            }
            Spacer()
            Button("Read", action: onRead).controlSize(.small)
            Button("Write", action: onWrite).buttonStyle(.borderedProminent).controlSize(.small)
        }
        .padding(.horizontal, 12).padding(.vertical, 9)
    }

    // MARK: - A section (All Memories / a bank / Unbanked)

    /// `bank` non-nil = a real bank section (its rows can be added-to and removed-from);
    /// `isUnbanked` marks the Unbanked bucket. All Memories has bank == nil, isUnbanked == false.
    @ViewBuilder
    private func section(key: String, title: String, bank: Int?, rows: [FT60Channel],
                         isUnbanked: Bool = false) -> some View
    {
        let isExpanded = !collapsed.contains(key)
        let isTarget = bank == selectedBank && !isUnbanked && key != "all"
        VStack(spacing: 0) {
            // Header
            HStack(spacing: 6) {
                Button {
                    if isExpanded { collapsed.insert(key) } else { collapsed.remove(key) }
                } label: {
                    Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9)).foregroundStyle(Theme.fg3).frame(width: 10)
                    Text(title).font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.fg)
                    Text("(\(rows.count))").font(.system(size: 10)).foregroundStyle(Theme.fg3)
                }.buttonStyle(.plain)
                Spacer()
                if let bank {
                    Button { selectedBank = isTarget ? nil : bank } label: {
                        Label(isTarget ? "target" : "add here",
                              systemImage: isTarget ? "checkmark.circle.fill" : "plus.circle")
                            .font(.system(size: 10))
                            .labelStyle(.titleAndIcon)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(isTarget ? Theme.accent : Theme.fg3)
                    .help("Location-first adds land in this bank")
                }
            }
            .padding(.horizontal, 8).padding(.vertical, 5)
            .background(isTarget ? Theme.accent.opacity(0.12) : Theme.bg2)

            if isExpanded {
                if rows.isEmpty {
                    Text(isUnbanked ? "All channels are in a bank." : "Empty.")
                        .font(.system(size: 10.5)).foregroundStyle(Theme.fg3)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 10).padding(.vertical, 6)
                } else {
                    ForEach(rows) { channelRow($0, sectionBank: bank, isUnbanked: isUnbanked) }
                }
            }
        }
        .background(Theme.panel)
        .clipShape(RoundedRectangle(cornerRadius: Theme.rCard))
        .overlay(RoundedRectangle(cornerRadius: Theme.rCard).stroke(Theme.border))
    }

    // MARK: - Channel row

    /// `sectionBank` non-nil = shown inside that bank's section (⊖ removes from the bank);
    /// otherwise ⊖ deletes the channel from the memory.
    private func channelRow(_ ch: FT60Channel, sectionBank: Int?, isUnbanked: Bool) -> some View {
        let info = ServiceType.info(ch.serviceType)
        return VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(String(format: "#%03d", ch.slot + 1))
                    .font(.system(size: 10, weight: .medium).monospacedDigit()).foregroundStyle(Theme.fg3)
                Image(systemName: info.symbol).font(.system(size: 11)).foregroundStyle(info.color)
                    .frame(width: 14).opacity(ch.skip ? 0.5 : 1)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 5) {
                        Text(ch.name.isEmpty ? "(unnamed)" : ch.name)
                            .font(.system(size: 12, weight: .medium)).lineLimit(1)
                            .foregroundStyle(ch.skip ? Theme.fg3 : Theme.fg)
                        if ch.skip {
                            Image(systemName: "arrow.right.to.line.circle").font(.system(size: 8.5))
                                .foregroundStyle(Theme.fg3).help("Skipped on scan")
                        }
                    }
                    Text(ch.detail).font(.system(size: 9.5).monospacedDigit()).foregroundStyle(Theme.fg3)
                        .lineLimit(1)
                }
                Spacer(minLength: 4)
                memberChips(ch)
                Button { detailsSlot = ch.slot } label: {
                    Image(systemName: "info.circle").font(.system(size: 11)).foregroundStyle(Theme.fg3)
                }
                .buttonStyle(.plain).help("Details / banks")
                .popover(isPresented: detailsBinding(ch.slot), arrowEdge: .trailing) {
                    detailsPopover(ch)
                }
                Button { sheet = .edit(ch) } label: {
                    Image(systemName: "pencil").font(.system(size: 11)).foregroundStyle(Theme.fg3)
                }
                .buttonStyle(.plain).help("Edit channel")
                Button {
                    if let sectionBank { memory.toggleBank(slot: ch.slot, bank: sectionBank) }
                    else { memory.remove(slot: ch.slot) }
                } label: {
                    Image(systemName: sectionBank == nil ? "minus.circle" : "rectangle.badge.minus")
                        .font(.system(size: 11)).foregroundStyle(Theme.fg3)
                }
                .buttonStyle(.plain)
                .help(sectionBank == nil ? "Delete channel" : "Remove from this bank")
            }
        }
        .padding(.horizontal, 8).padding(.vertical, 5)
        .overlay(alignment: .bottom) { Divider().overlay(Theme.border).opacity(0.5) }
    }

    /// The banks a channel is in, as small filled chips (member-only). Tap removes.
    @ViewBuilder
    private func memberChips(_ ch: FT60Channel) -> some View {
        if !ch.banks.isEmpty {
            HStack(spacing: 2) {
                ForEach(ch.banks.sorted(), id: \.self) { b in
                    Button { memory.toggleBank(slot: ch.slot, bank: b) } label: {
                        Text(FT60Memory.bankLabel(b))
                            .font(.system(size: 9, weight: .bold))
                            .frame(width: 15, height: 15)
                            .background(Theme.accent.opacity(0.30))
                            .foregroundStyle(Theme.fg)
                            .clipShape(RoundedRectangle(cornerRadius: 3))
                    }
                    .buttonStyle(.plain).help("In Bank \(FT60Memory.bankLabel(b)) — tap to remove")
                }
            }
        }
    }

    // MARK: - Details popover (read-only info + editable bank grid)

    private func detailsBinding(_ slot: Int) -> Binding<Bool> {
        Binding(get: { detailsSlot == slot }, set: { if !$0 { detailsSlot = nil } })
    }

    private func detailsPopover(_ ch: FT60Channel) -> some View {
        let info = ServiceType.info(ch.serviceType)
        return VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 7) {
                Image(systemName: info.symbol).foregroundStyle(info.color)
                VStack(alignment: .leading, spacing: 1) {
                    Text(ch.name.isEmpty ? "(unnamed)" : ch.name)
                        .font(.system(size: 12.5, weight: .semibold)).lineLimit(1)
                    Text(String(format: "Memory #%03d", ch.slot + 1))
                        .font(.system(size: 10)).foregroundStyle(Theme.fg3)
                }
            }
            Divider().overlay(Theme.border)
            let opts = Ft60Options.shared
            detailRow("Frequency", "\(ch.freqMHz) MHz")
            detailRow("Mode", opts.label(opts.modes, ch.modeCode))
            detailRow("Step", opts.label(opts.steps, ch.stepCode))
            if ch.toneModeCode != 0 { detailRow("Tone mode", opts.label(opts.toneModes, ch.toneModeCode)) }
            detailRow("Tone", ch.tone.display)
            detailRow("Duplex", opts.label(opts.duplexes, ch.duplexCode))
            if opts.duplexNeedsOffset(ch.duplexCode) {
                detailRow("Offset", String(format: "%.4f MHz", Double(ch.offsetHz) / 1_000_000))
            } else if opts.duplexIsSplit(ch.duplexCode) {
                detailRow("TX freq", String(format: "%.4f MHz", Double(ch.txHz) / 1_000_000))
            }
            detailRow("Power", opts.label(opts.powers, ch.powerCode))
            detailRow("Skip", ["No", "Skip", "Preferred"][min(2, Int(ch.skipRaw))])
            if ch.serviceType != nil { detailRow("Service type", info.name) }
            Divider().overlay(Theme.border)
            Text("Banks").font(.system(size: 10.5, weight: .semibold)).foregroundStyle(Theme.fg2)
            bankGrid(ch)
        }
        .padding(14).frame(width: 236)
    }

    /// A 2×5 A–J toggle grid — the place to add a channel to any bank(s).
    private func bankGrid(_ ch: FT60Channel) -> some View {
        VStack(spacing: 4) {
            ForEach(0..<2, id: \.self) { rowIdx in
                HStack(spacing: 4) {
                    ForEach(0..<5, id: \.self) { colIdx in
                        let b = rowIdx * 5 + colIdx
                        let member = ch.banks.contains(b)
                        Button { memory.toggleBank(slot: ch.slot, bank: b) } label: {
                            Text(FT60Memory.bankLabel(b))
                                .font(.system(size: 10, weight: .bold))
                                .frame(width: 36, height: 22)
                                .background(member ? Theme.accent.opacity(0.30) : Theme.chip)
                                .foregroundStyle(member ? Theme.fg : Theme.fg3)
                                .clipShape(RoundedRectangle(cornerRadius: 4))
                        }.buttonStyle(.plain)
                    }
                }
            }
        }
    }

    private func detailRow(_ label: String, _ value: String) -> some View {
        HStack {
            Text(label).font(.system(size: 11)).foregroundStyle(Theme.fg3)
            Spacer()
            Text(value).font(.system(size: 11.5, weight: .medium).monospacedDigit()).foregroundStyle(Theme.fg2)
        }
    }

    // MARK: - PMS band-edge (scan-limit) memories — read-only display

    private var pmsSection: some View {
        let key = "pms"
        let isExpanded = !collapsed.contains(key)
        return VStack(spacing: 0) {
            Button {
                if isExpanded { collapsed.insert(key) } else { collapsed.remove(key) }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9)).foregroundStyle(Theme.fg3).frame(width: 10)
                    Text("Scan edges (PMS)").font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.fg)
                    Text("(\(memory.pms.count))").font(.system(size: 10)).foregroundStyle(Theme.fg3)
                    Spacer()
                }
                .padding(.horizontal, 8).padding(.vertical, 5).background(Theme.bg2)
            }.buttonStyle(.plain)
            if isExpanded {
                ForEach(memory.pms) { pair in
                    HStack(spacing: 6) {
                        Text(pair.label)
                            .font(.system(size: 10, weight: .medium).monospacedDigit())
                            .foregroundStyle(Theme.fg3).frame(width: 54, alignment: .leading)
                        Text(edgeLabel(pair.lowerHz)).font(.system(size: 11).monospacedDigit()).foregroundStyle(Theme.fg2)
                        Image(systemName: "arrow.right").font(.system(size: 8)).foregroundStyle(Theme.fg3)
                        Text(edgeLabel(pair.upperHz)).font(.system(size: 11).monospacedDigit()).foregroundStyle(Theme.fg2)
                        Spacer()
                    }
                    .padding(.horizontal, 10).padding(.vertical, 4)
                }
            }
        }
        .background(Theme.panel).clipShape(RoundedRectangle(cornerRadius: Theme.rCard))
        .overlay(RoundedRectangle(cornerRadius: Theme.rCard).stroke(Theme.border))
    }

    private func edgeLabel(_ hz: UInt64?) -> String {
        hz.map { String(format: "%.4f MHz", Double($0) / 1_000_000) } ?? "—"
    }

    // MARK: - Footer

    private var footer: some View {
        HStack(spacing: 10) {
            Button { sheet = .add } label: {
                Label("Add channel by hand", systemImage: "plus.circle").font(.system(size: 11))
            }.buttonStyle(.plain).foregroundStyle(Theme.accent)
            Spacer()
            Text("\(memory.count(inBank: nil)) / \(memory.capacity.channels) memories")
                .font(.system(size: 10).monospacedDigit()).foregroundStyle(Theme.fg3)
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
    }
}

/// The manual channel form — used both to **add** a channel by hand (repeaters/simplex not in
/// the DB) and to **edit** an existing one (`editing` non-nil). Covers every editable memory
/// field; banks stay on the row's bank grid. `onSave` receives a slot→channel maker so the
/// parent can append (new slot) or update (existing slot).
private struct FT60ChannelSheet: View {
    let nameLimit: Int
    let editing: FT60Channel?
    let onSave: (@escaping (Int) -> FT60Channel) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var name: String
    @State private var freqText: String
    @State private var modeCode: Int
    @State private var toneModeCode: Int
    @State private var toneText: String
    @State private var powerCode: Int
    @State private var duplexCode: Int
    @State private var offsetText: String
    @State private var txText: String
    @State private var stepCode: Int
    @State private var skipCode: Int  // 0 off, 1 Skip, 2 Preferred

    init(nameLimit: Int, editing: FT60Channel?, onSave: @escaping (@escaping (Int) -> FT60Channel) -> Void) {
        self.nameLimit = nameLimit
        self.editing = editing
        self.onSave = onSave
        let opts = Ft60Options.shared
        _name = State(initialValue: editing?.name ?? "")
        _freqText = State(initialValue: editing.map { String(format: "%.4f", Double($0.freqHz) / 1_000_000) } ?? "")
        _modeCode = State(initialValue: editing?.modeCode ?? opts.modes.first?.code ?? 0)
        _toneModeCode = State(initialValue: editing?.toneModeCode ?? 0)
        _powerCode = State(initialValue: editing?.powerCode ?? opts.powers.first?.code ?? 0)
        _duplexCode = State(initialValue: editing?.duplexCode ?? Ft60Options.duplexSimplex)
        _stepCode = State(initialValue: editing?.stepCode ?? opts.steps.first?.code ?? 0)
        _skipCode = State(initialValue: Int(editing?.skipRaw ?? 0))
        _offsetText = State(initialValue: editing.flatMap { $0.offsetHz > 0 ? String(format: "%.4f", Double($0.offsetHz) / 1_000_000) : nil } ?? "")
        _txText = State(initialValue: editing.flatMap { $0.txHz > 0 ? String(format: "%.4f", Double($0.txHz) / 1_000_000) : nil } ?? "")
        let toneVal: String
        switch editing?.tone {
        case .ctcss(let f): toneVal = String(format: "%.1f", f)
        case .dcs(let c): toneVal = String(c)
        default: toneVal = ""
        }
        _toneText = State(initialValue: toneVal)
    }

    private var isEdit: Bool { editing != nil }
    private var opts: Ft60Options { Ft60Options.shared }

    /// The value the currently-selected tone mode needs: "none" / "ctcss" / "dcs".
    private var toneValueKind: String {
        opts.option(opts.toneModes, toneModeCode)?.valueKind ?? "none"
    }

    private func mhz(_ text: String) -> UInt64? {
        guard let v = Double(text.trimmingCharacters(in: .whitespaces)), v > 0 else { return nil }
        return UInt64((v * 1_000_000).rounded())
    }
    private var freqHz: UInt64? { mhz(freqText) }

    /// The band-standard repeater shift for a frequency (US conventions), used as the default
    /// when the user picks +/− duplex without typing an offset.
    private func standardOffsetHz(for hz: UInt64) -> UInt64 {
        switch Double(hz) / 1_000_000 {
        case 144..<148: return 600_000  // 2 m
        case 222..<225: return 1_600_000  // 1.25 m
        case 420..<450: return 5_000_000  // 70 cm
        default: return 0
        }
    }
    private var offsetHz: UInt64 {
        guard opts.duplexNeedsOffset(duplexCode) else { return 0 }
        return mhz(offsetText) ?? standardOffsetHz(for: freqHz ?? 0)
    }
    private var txHz: UInt64 { opts.duplexIsSplit(duplexCode) ? (mhz(txText) ?? 0) : 0 }
    private var offsetPlaceholder: String {
        String(format: "%.4f", Double(standardOffsetHz(for: freqHz ?? 0)) / 1_000_000)
    }

    private var tone: FTTone {
        switch toneValueKind {
        case "ctcss": return Double(toneText).map(FTTone.ctcss) ?? .ctcss(100.0)
        case "dcs": return Int(toneText).map(FTTone.dcs) ?? .dcs(23)
        default: return .off
        }
    }

    var body: some View {
        VStack(spacing: 14) {
            VStack(spacing: 7) {
                Image(systemName: "antenna.radiowaves.left.and.right")
                    .font(.system(size: 18)).foregroundStyle(.white)
                    .frame(width: 46, height: 46)
                    .background(Circle().fill(Theme.accent))
                Text(isEdit ? "Edit channel" : "Add channel")
                    .font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.fg)
                Text(isEdit ? "#\(String(format: "%03d", (editing?.slot ?? 0) + 1))"
                    : "Manual entry — repeaters/simplex not in the database")
                    .font(.system(size: 11)).foregroundStyle(Theme.fg3).multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)

            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    field("Name", "≤ \(nameLimit) chars", $name)
                        .onChange(of: name) { if name.count > nameLimit { name = String(name.prefix(nameLimit)) } }
                    field("Frequency (MHz)", "146.5200", $freqText)
                    pickerRow("Mode") {
                        Picker("", selection: $modeCode) {
                            ForEach(opts.modes, id: \.code) { Text($0.label).tag($0.code) }
                        }.pickerStyle(.segmented).labelsHidden()
                    }
                    pickerRow("Tone mode") {
                        Picker("", selection: $toneModeCode) {
                            ForEach(opts.toneModes, id: \.code) { Text($0.label).tag($0.code) }
                        }.pickerStyle(.menu).labelsHidden().fixedSize()
                    }
                    if toneValueKind != "none" {
                        field(toneValueKind == "ctcss" ? "CTCSS tone (Hz)" : "DCS code",
                              toneValueKind == "ctcss" ? "100.0" : "23", $toneText)
                    }
                    pickerRow("Power") {
                        Picker("", selection: $powerCode) {
                            ForEach(opts.powers, id: \.code) { Text($0.label).tag($0.code) }
                        }.pickerStyle(.segmented).labelsHidden()
                    }
                    pickerRow("Step") {
                        Picker("", selection: $stepCode) {
                            ForEach(opts.steps, id: \.code) { Text($0.label).tag($0.code) }
                        }.pickerStyle(.menu).labelsHidden().fixedSize()
                    }
                    pickerRow("Skip") {
                        Picker("", selection: $skipCode) {
                            Text("Off").tag(0); Text("Skip").tag(1); Text("Pref").tag(2)
                        }.pickerStyle(.segmented).labelsHidden()
                    }
                    pickerRow("Duplex") {
                        Picker("", selection: $duplexCode) {
                            ForEach(opts.duplexes, id: \.code) { Text($0.label).tag($0.code) }
                        }.pickerStyle(.segmented).labelsHidden()
                    }
                    if opts.duplexNeedsOffset(duplexCode) {
                        field("Offset (MHz)", offsetPlaceholder, $offsetText)
                        Text("Blank uses the band standard (\(offsetPlaceholder) MHz).")
                            .font(.system(size: 10)).foregroundStyle(Theme.fg3)
                    } else if opts.duplexIsSplit(duplexCode) {
                        field("TX frequency (MHz)", "147.0000", $txText)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 340)

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large)
                Button(isEdit ? "Save" : "Add") {
                    guard let hz = freqHz else { return }
                    let n = name, m = modeCode, tm = toneModeCode, t = tone, p = powerCode, d = duplexCode
                    let lim = nameLimit, off = offsetHz, tx = txHz, st = stepCode, sk = skipCode
                    let ed = editing
                    onSave { slot in
                        FT60Channel(
                            slot: slot, name: String(n.prefix(lim)), freqHz: hz, modeCode: m,
                            toneModeCode: tm, tone: t, banks: ed?.banks ?? [],
                            skip: sk != 0, skipRaw: UInt8(sk), powerCode: p, duplexCode: d,
                            offsetHz: off, txHz: tx, stepCode: st, serviceType: ed?.serviceType)
                    }
                    dismiss()
                }
                .buttonStyle(.borderedProminent).controlSize(.large).disabled(freqHz == nil)
                Spacer()
            }
        }
        .padding(18).frame(width: 360)
        .background(Theme.panel)
    }

    /// A themed labeled text field (matches the app's dark chrome).
    private func field(_ label: String, _ placeholder: String, _ text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.system(size: 10.5, weight: .medium)).foregroundStyle(Theme.fg3)
            TextField(placeholder, text: text)
                .textFieldStyle(.plain).font(.system(size: 12)).foregroundStyle(Theme.fg)
                .padding(.horizontal, 9).padding(.vertical, 7)
                .background(Theme.bg3).clipShape(RoundedRectangle(cornerRadius: Theme.rField))
                .overlay(RoundedRectangle(cornerRadius: Theme.rField).stroke(Theme.border))
        }
    }

    /// A labeled row hosting a segmented picker.
    private func pickerRow<P: View>(_ label: String, @ViewBuilder _ picker: () -> P) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.system(size: 10.5, weight: .medium)).foregroundStyle(Theme.fg3)
            picker()
        }
    }
}
