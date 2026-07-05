// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import Foundation

// The in-memory editor model for the Yaesu FT-60 (a clone-image handheld). Populated from a
// real serial Read (decoded by the Rust spec-derived codec via the `Ft60` bridge) or built by
// hand, and written back on Write. Multi-bank membership is native to the hardware, so a
// channel carries a Set of bank indices.

// The FT-60's editable-field option sets (modes, tone modes, steps, powers, duplexes) are no
// longer declared here — they come from the **core** profile via `Ft60Options` (loaded over
// the FFI). A channel stores each field as its on-radio `…Code`; labels are resolved through
// `Ft60Options.shared`. Only `FTTone` (a value carrier, not an attribute table) stays.

/// Structured squelch tone — the discriminated representation the string-only canonical
/// `Channel.tone` lacks (CTCSS frequency vs. DCS code vs. off).
enum FTTone: Equatable, Codable {
    case off
    case ctcss(Double)  // CTCSS tone frequency in Hz, e.g. 100.0
    case dcs(Int)  // DCS code, e.g. 23

    /// Parse the raw audio-option field our model carries (`TONE=C156.7` / `TONE=D023`).
    /// Conventional channels only ever carry CTCSS/DCS here (NAC/ColorCode are trunked).
    static func parse(_ raw: String?) -> FTTone {
        guard let raw = raw?.trimmingCharacters(in: .whitespaces), !raw.isEmpty else { return .off }
        let v = raw.hasPrefix("TONE=") ? String(raw.dropFirst("TONE=".count)) : raw
        if v.hasPrefix("C"), let f = Double(v.dropFirst()) { return .ctcss(f) }
        if v.hasPrefix("D"), let c = Int(v.dropFirst()) { return .dcs(c) }
        return .off
    }

    /// Short display, e.g. "CTCSS 100.0 Hz" / "DCS 023" / "—".
    var display: String {
        switch self {
        case .off: return "—"
        case .ctcss(let f): return String(format: "CTCSS %.1f Hz", f)
        case .dcs(let c): return String(format: "DCS %03d", c)
        }
    }

    /// Selector state ("Off" / "CTCSS" / "DCS") for the tone-mode menu.
    var modeLabel: String {
        switch self {
        case .off: return "Off"
        case .ctcss: return "CTCSS"
        case .dcs: return "DCS"
        }
    }
}

/// One FT-60 memory channel (a global slot). `banks` is the set of bank indices (0–9)
/// this slot belongs to — a channel can be in several.
struct FT60Channel: Identifiable, Codable {
    var slot: Int  // 0-based global memory index (displayed as #001 = slot 0 + 1)
    var name: String  // ≤ 6 chars on the radio
    var freqHz: UInt64
    /// Operating mode as its on-radio code (index into `Ft60Options.shared.modes`).
    var modeCode: Int
    /// Tone-mode sub-kind as its on-radio code (index into `Ft60Options.shared.toneModes`);
    /// `tone` carries the CTCSS Hz / DCS code value.
    var toneModeCode: Int = 0
    var tone: FTTone
    var banks: Set<Int>
    var skip: Bool
    /// Raw skip code (0=none, 1=Skip, 2=Preferred) — carried verbatim from the radio so a
    /// write preserves the tri-state the display-only `skip` Bool can't represent.
    var skipRaw: UInt8 = 0
    /// TX power as its on-radio code (index into `Ft60Options.shared.powers`).
    var powerCode: Int
    /// Repeater shift as the writer's duplex code (0 simplex, 2 −, 3 +, 4 split).
    var duplexCode: Int
    var offsetHz: UInt64
    /// TX frequency (Hz) for "split" duplex; 0 otherwise.
    var txHz: UInt64 = 0
    /// Tuning step as its on-radio code (index into `Ft60Options.shared.steps`).
    var stepCode: Int = 0
    /// RadioReference service-type code when this came from the catalog (drives the row
    /// icon via `ServiceType.info`); nil for hand-entered channels.
    var serviceType: Int?

    var id: Int { slot }

    /// "146.5200" (MHz, 4 dp) — matches the app's frequency formatting.
    var freqMHz: String { String(format: "%.4f", Double(freqHz) / 1_000_000) }

    /// Mode label from the core option list (e.g. "FM").
    var modeLabel: String { Ft60Options.shared.label(Ft60Options.shared.modes, modeCode) }

    /// Trailing detail line: "146.5200 · FM · CTCSS 100.0 Hz".
    var detail: String {
        var parts = ["\(freqMHz) MHz", modeLabel]
        if tone != .off { parts.append(tone.display) }
        return parts.joined(separator: "  ·  ")
    }
}

/// A clone-image radio's fixed memory capacity — sourced from the core registry (via the
/// active `RadioModel`), never re-declared here.
struct FTCapacity: Hashable {
    let channels: Int
    let banks: Int
    let nameLen: Int
}

/// The in-memory FT-60 memory image. Owns the flat channel list + captured image bytes and
/// the edit operations the UI drives. Bank indices are 0-based; the UI labels them A–J.
final class FT60Memory: ObservableObject {
    @Published var channels: [FT60Channel]
    /// The active radio's capacity (from the core). Drives slot/name/bank limits.
    let capacity: FTCapacity

    /// The raw clone-image bytes this memory was read from (empty for the mock/hand-built
    /// image). Kept verbatim so the exact same image can be written back to the radio; a
    /// non-empty `image` is what gates the Write action.
    var image: [UInt8]

    /// Programmed PMS band-edge (scan-limit) pairs, from a Read. Read-only display for now.
    @Published var pms: [FT60PmsPair]

    init(capacity: FTCapacity, channels: [FT60Channel] = [], image: [UInt8] = [], pms: [FT60PmsPair] = []) {
        self.capacity = capacity
        self.channels = channels
        self.image = image
        self.pms = pms
    }

    /// True when this memory carries a real captured image that can be written back.
    var canWrite: Bool { !image.isEmpty }

    /// Bank label for an index: 0 → "A", 1 → "B", …
    static func bankLabel(_ index: Int) -> String {
        guard index >= 0, index < 26 else { return "?" }
        return String(UnicodeScalar(UInt8(65 + index)))
    }

    /// Channels belonging to a bank (nil = all channels).
    func channels(inBank bank: Int?) -> [FT60Channel] {
        guard let bank else { return channels }
        return channels.filter { $0.banks.contains(bank) }
    }

    /// Count of channels in a bank (nil = total programmed).
    func count(inBank bank: Int?) -> Int { channels(inBank: bank).count }

    /// Channels belonging to no bank.
    var unbanked: [FT60Channel] { channels.filter { $0.banks.isEmpty } }

    /// The next free global slot index, or nil if the memory is full.
    private var nextFreeSlot: Int? {
        channels.count < capacity.channels ? channels.count : nil
    }

    /// Append a channel at the next free slot, optionally tagging it into a bank. No-op
    /// (returns false) if the memory is full.
    @discardableResult
    func append(_ make: (Int) -> FT60Channel, toBank bank: Int?) -> Bool {
        guard let slot = nextFreeSlot else { return false }
        var ch = make(slot)
        ch.slot = slot
        if let bank { ch.banks.insert(bank) }
        channels.append(ch)
        return true
    }

    /// Build an FT-60 channel from a catalog conventional channel (freq/name/mode/tone).
    func makeFromCatalog(name: String, freqHz: UInt64, mode: String?, tone: String?, serviceType: Int?)
        -> (Int) -> FT60Channel
    {
        { slot in
            let opts = Ft60Options.shared
            let t = FTTone.parse(tone)
            // A catalog tone means squelch (TSQL) for CTCSS, or DTCS for DCS; else no tone —
            // resolved to the right tone-mode code by label from the core option list.
            let toneLabel: String
            switch t {
            case .off: toneLabel = "Off"
            case .ctcss: toneLabel = "TSQL"
            case .dcs: toneLabel = "DTCS"
            }
            // Mode by label (defaults to the first option, FM, if the catalog string is unknown).
            let modeCode = mode.flatMap { opts.code(opts.modes, label: $0.uppercased()) }
                ?? opts.modes.first?.code ?? 0
            return FT60Channel(
                slot: slot,
                name: String(name.prefix(self.capacity.nameLen)),
                freqHz: freqHz,
                modeCode: modeCode,
                toneModeCode: opts.code(opts.toneModes, label: toneLabel) ?? 0,
                tone: t,
                banks: [],
                skip: false,
                powerCode: 0,
                duplexCode: Ft60Options.duplexSimplex,
                offsetHz: 0,
                serviceType: serviceType)
        }
    }

    func toggleBank(slot: Int, bank: Int) {
        guard let i = channels.firstIndex(where: { $0.slot == slot }) else { return }
        if channels[i].banks.contains(bank) {
            channels[i].banks.remove(bank)
        } else {
            channels[i].banks.insert(bank)
        }
    }

    func remove(slot: Int) {
        channels.removeAll { $0.slot == slot }
    }

    /// Replace a channel in place (used by the edit form).
    func update(_ ch: FT60Channel) {
        guard let i = channels.firstIndex(where: { $0.slot == ch.slot }) else { return }
        channels[i] = ch
    }
}
