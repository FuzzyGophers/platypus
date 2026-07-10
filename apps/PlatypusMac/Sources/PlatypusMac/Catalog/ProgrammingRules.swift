// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import Foundation

/// Pure decision rules for programming browsed data onto a radio — extracted from `CatalogView` so
/// they're unit-testable (the two capability/backup bugs this session lived in this logic). No UI or
/// device state: every input is passed in.

/// Whether the active target is loaded + ready to receive channels (the whole-target gate).
enum AddReadiness: Equatable {
    case ready
    case notReady(String)
}

/// Whether a browse **source** can be programmed onto a **target radio class** — the write-path gate:
/// - clone-image: any source (conventional channels synthesized via `makeFromCatalog`),
/// - sd-card: `.hpdb` (selected from the loaded library) or `.radioReference` (favorites synthesized),
/// - neutral (no radio): nothing.
func writePath(target: RadioClass?, source: DataSourceKind) -> Bool {
    switch target {
    case .cloneImage: return true
    case .sdCard: return source == .hpdb || source == .radioReference
    case nil: return false
    }
}

/// Whether the target is loaded + ready to add into — generic by class, with a name-derived hint.
/// A clone image must have been read (`hasImage`, so the write-back preserves its settings); an
/// SD-card list must be open (`hasList`), which needs a card open first (`hasCard`). Neutral is never
/// ready.
func addReadiness(
    deviceClass: RadioClass?, radioName: String, hasImage: Bool, hasList: Bool, hasCard: Bool
) -> AddReadiness {
    guard let deviceClass else { return .notReady("Choose a radio to add channels.") }
    switch deviceClass {
    case .cloneImage:
        return hasImage ? .ready : .notReady("Read your \(radioName) first to add channels.")
    case .sdCard:
        if hasList { return .ready }
        // A card must be open before you can select/create a list — guide to the right next step,
        // not "open a card" you already opened.
        return hasCard
            ? .notReady("Select or add a favorites list to add channels.")
            : .notReady("Open your \(radioName)’s card first to add channels.")
    }
}

/// The backup-before-save safety gate applies **only to a live SD card** (protect it with a restore
/// point). A backup/library folder on disk isn't a live card — editing + saving it is safe.
func needsBackup(isLive: Bool, backedUp: Bool) -> Bool { isLive && !backedUp }

/// A connected scanner card's identity for the "Write to Card" model gate: its product-name `model`
/// (e.g. `"SDS150"`), or nil if the card's header couldn't be read. We match on the product name
/// because that's the one identifier shared across both sides — `CardInfo.model` and the app's radio
/// `name` both come from the profile's `product_name()`. (Do NOT use `CardInfo.modelId`; that's the
/// serial-protocol id, e.g. `"SDS150GBT"`, a different id space from `radios.activeID`.)
struct CardIdentity: Equatable {
    let model: String?
}

/// The outcome of matching connected cards against the model we can write to.
enum CardMatch: Equatable {
    /// Index of the first connected card whose model we can write to.
    case match(Int)
    /// No scanner card detected at all.
    case noCard
    /// A card is present but it's a different model (its display name) — we won't write to it.
    case wrongModel(String)
}

/// Pick the connected card we can write favorites to — a **model** match (any SDS150), not a
/// specific device. Returns the first card whose product-name `model` equals `wantModel` (the active
/// radio's `name`; an unreadable card never matches); else explains why: nothing detected, or a card
/// of another model. Pure so the write-gate decision is unit-tested without a real card.
func matchWritableCard(_ cards: [CardIdentity], wantModel: String) -> CardMatch {
    if let i = cards.firstIndex(where: { $0.model == wantModel }) {
        return .match(i)
    }
    if cards.isEmpty { return .noCard }
    return .wrongModel(cards.first.flatMap(\.model) ?? "different scanner")
}
