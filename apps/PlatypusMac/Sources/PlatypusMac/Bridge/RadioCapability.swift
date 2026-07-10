// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import Foundation

/// What a target radio can be **programmed with** — mirrors the core `ProgramSupport`, decoded from
/// the radio registry (`Radios.Info`). Drives the capability-aware browse: each browsed system/channel
/// is tested against the active target, and the ones it can't take are de-emphasized + badged.
///
/// Built to stay flexible as new radios add features: capability is declared per-radio in the core,
/// new axes arrive as new fields here + new checks in `reason(...)`, and an **unknown** capability
/// (no data) *fails open* — nothing is filtered rather than wrongly hidden.
struct RadioCapability: Hashable {
    /// Stores trunked-system talkgroups (a trunk-tracking scanner) vs conventional memories only.
    let trunking: Bool
    /// Supported modulation labels, lowercased ("analog", "p25", "dmr", …) — the core's labels.
    let modulations: Set<String>

    /// nil ⇒ capability unknown (the core reported no modulations); callers skip filtering (fail open).
    init?(trunking: Bool, modulations: [String]) {
        guard !modulations.isEmpty else { return nil }
        self.trunking = trunking
        self.modulations = Set(modulations.map { $0.lowercased() })
    }

    /// Why a browsed item (a possibly-trunked system with a `tech` tag, or a channel with a `mode`)
    /// can't be programmed onto this radio — or nil if it can. A sequence of checks: add new axes
    /// (band coverage, step, feature…) as more cases without touching the call sites.
    func reason(isTrunked: Bool, tech: String?, mode: String?) -> Incompatibility? {
        if isTrunked && !trunking { return .trunking }
        let mod = Modulation.classify(tech: tech, mode: mode)
        if !modulations.contains(mod.label.lowercased()) { return .modulation(mod) }
        return nil
    }

    /// The supported modulations in canonical order, for the capability legend.
    var modulationLabels: [String] {
        Modulation.allCases.map(\.label).filter { modulations.contains($0.lowercased()) }
    }

    /// One-line capability summary for the legend, e.g. "analog · conventional only" or
    /// "all modes · trunk + conventional".
    var summary: String {
        let kinds = trunking ? "trunk + conventional" : "conventional only"
        let mods = modulations.count >= Modulation.allCases.count
            ? "all modes"
            : modulationLabels.joined(separator: ", ")
        return "\(mods) · \(kinds)"
    }
}

/// Why the active target radio can't take a browsed item right now — either the radio physically
/// can't (capability) or there's no path from this source to this target yet (write path). One type
/// drives the de-emphasis + badge + disabled Add uniformly across the list and the map.
struct AddBlock: Equatable {
    /// Short chip text, e.g. "P25", "trunked", "not yet".
    let badge: String
    /// Full explanation for the tooltip / popover.
    let detail: String
}

/// A concrete reason the active target radio can't take a browsed item — drives the badge + tooltip.
enum Incompatibility: Hashable {
    /// The item is trunked; the radio stores conventional memories only.
    case trunking
    /// The item's modulation isn't supported (carries which).
    case modulation(Modulation)

    /// Terse chip text next to a greyed row/pin.
    var badge: String {
        switch self {
        case .trunking: return "trunked"
        case .modulation(let m): return m.label
        }
    }

    /// Full explanation for the disabled-Add tooltip.
    var detail: String {
        switch self {
        case .trunking: return "Trunked — this radio can't track trunked systems."
        case .modulation(let m): return "\(m.label) isn't supported by this radio."
        }
    }
}

/// Modulation family — mirrors platypus-core `Modulation`. Kept in sync with the core classifier
/// (covered by a parity test); `otherDigital` is the catch-all so unknown digital modes degrade
/// gracefully rather than being mistaken for analog.
enum Modulation: Hashable, CaseIterable {
    case analog, p25, dmr, nxdn, dStar, fusion, proVoice, otherDigital

    var label: String {
        switch self {
        case .analog: return "analog"
        case .p25: return "P25"
        case .dmr: return "DMR"
        case .nxdn: return "NXDN"
        case .dStar: return "D-STAR"
        case .fusion: return "Fusion"
        case .proVoice: return "ProVoice"
        case .otherDigital: return "digital"
        }
    }

    /// Classify a system tech tag and/or a channel mode into a modulation family — the `tech` tag is
    /// more specific, so it's consulted first; anything unrecognized is treated as analog.
    static func classify(tech: String?, mode: String?) -> Modulation {
        for hint in [tech, mode].compactMap({ $0 }) {
            if let m = fromHint(hint) { return m }
        }
        return .analog
    }

    private static func fromHint(_ s: String) -> Modulation? {
        let u = s.uppercased().trimmingCharacters(in: .whitespaces)
        switch u {
        case "FM", "NFM", "FMN", "AM", "NAM", "WFM", "ANALOG": return .analog
        default: break
        }
        if u.contains("P25") || u.contains("PROJECT 25") || u.contains("APCO") { return .p25 }
        if u.contains("DMR") || u.contains("TRBO") || u.contains("HYTERA") { return .dmr }
        if u.contains("NXDN") || u.contains("NEXEDGE") || u.contains("IDAS") { return .nxdn }
        if u.contains("D-STAR") || u.contains("DSTAR") || u.contains("D STAR") { return .dStar }
        if u.contains("FUSION") || u.contains("C4FM") || u.contains("YSF") { return .fusion }
        if u.contains("PROVOICE") || u.contains("PRO-VOICE") { return .proVoice }
        if u.contains("DIGITAL") { return .otherDigital }
        return nil
    }
}
