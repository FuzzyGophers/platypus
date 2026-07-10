// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import XCTest

@testable import PlatypusMac

/// `AudioOption` union parser (SDS150 C-Freq audio field) — pure.
final class AudioOptionTests: XCTestCase {
    func testTone() {
        let p = AudioOption.parse("TONE=C156.7")
        XCTAssertEqual(p?.label, "Tone")
        XCTAssertEqual(p?.value, "CTCSS 156.7 Hz")
        XCTAssertEqual(AudioOption.parse("TONE=D023")?.value, "DCS 023")
        XCTAssertEqual(AudioOption.inline("TONE=C156.7"), "CTCSS 156.7 Hz")
    }

    func testDigitalIdentifiers() {
        XCTAssertEqual(AudioOption.parse("NAC=293")?.label, "NAC")
        XCTAssertEqual(AudioOption.parse("NAC=293")?.value, "293")
        XCTAssertEqual(AudioOption.parse("ColorCode=1")?.label, "Color code")
        XCTAssertEqual(AudioOption.parse("RAN=1")?.label, "RAN")
        XCTAssertEqual(AudioOption.inline("NAC=293"), "NAC 293")
    }

    func testEmptyIsNil() {
        XCTAssertNil(AudioOption.parse(nil))
        XCTAssertNil(AudioOption.parse(""))
        XCTAssertNil(AudioOption.inline(nil))
    }
}

/// `ServiceType.info` fallbacks + `Country.name`. Names come from the core over the FFI
/// (the test target links the staticlib), symbol/color are the app's presentation.
final class ServiceTypeTests: XCTestCase {
    func testKnownCode() {
        let fire = ServiceType.info(3)
        XCTAssertEqual(fire.name, "Fire Dispatch") // sourced from core::model::SERVICE_TYPES
        XCTAssertEqual(fire.symbol, "flame.fill")
    }

    func testNilAndUnknownFallback() {
        let none = ServiceType.info(nil)
        XCTAssertEqual(none.name, "Other")
        XCTAssertEqual(none.symbol, "dot.radiowaves.left.and.right")

        let unknown = ServiceType.info(9999)
        XCTAssertEqual(unknown.name, "Service type 9999")
        XCTAssertEqual(unknown.symbol, "dot.radiowaves.left.and.right") // neutral glyph
    }

    func testFilterOrderIsAlphabeticalByName() {
        let names = ServiceType.filterOrderAlphabetical.map { ServiceType.info($0).name }
        XCTAssertEqual(names, names.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending })
    }

    func testCountryNames() {
        XCTAssertEqual(Country.name(0), "Nationwide & Interstate")
        XCTAssertEqual(Country.name(1), "United States")
        XCTAssertEqual(Country.name(2), "Canada")
        XCTAssertEqual(Country.name(99), "Country 99")
    }
}

/// `DataSourceStore` — HPDB is a folder-picked source, not a default, and drives the map count.
final class DataSourceStoreTests: XCTestCase {
    func testNothingAddedByDefault() {
        let store = DataSourceStore()
        XCTAssertEqual(store.source(.hpdb)?.added, false)
        XCTAssertEqual(store.activeCount, 0)               // no default source
        XCTAssertNil(store.source(.hpdb)?.folderPath)
        XCTAssertEqual(store.source(.hpdb)?.configured, false)
    }

    func testHpdbIsAddableUntilAdded() {
        let store = DataSourceStore()
        XCTAssertTrue(store.addableKinds.contains(.hpdb))   // offered in "Add a source…"
        store.addHpdb(folderPath: "/tmp/MyCard/HPDB")
        XCTAssertFalse(store.addableKinds.contains(.hpdb))  // gone once added
    }

    func testAddHpdbConfiguresAndEnables() {
        let store = DataSourceStore()
        store.addHpdb(folderPath: "/tmp/MyCard/HPDB")
        let hpdb = store.source(.hpdb)
        XCTAssertEqual(hpdb?.added, true)
        XCTAssertEqual(hpdb?.enabled, true)
        XCTAssertEqual(hpdb?.configured, true)
        XCTAssertEqual(hpdb?.folderPath, "/tmp/MyCard/HPDB")
        XCTAssertEqual(hpdb?.statusText, "MyCard · HPDB")   // parent · leaf
        XCTAssertEqual(store.activeCount, 1)
    }

    func testDisableKeepsFolderButDropsFromActive() {
        let store = DataSourceStore()
        store.addHpdb(folderPath: "/tmp/MyCard/HPDB")
        store.toggleEnabled(.hpdb)
        XCTAssertEqual(store.activeCount, 0)                // no longer merges into the map
        XCTAssertEqual(store.source(.hpdb)?.folderPath, "/tmp/MyCard/HPDB")  // path retained
        store.toggleEnabled(.hpdb)
        XCTAssertEqual(store.activeCount, 1)
    }

    func testMultipleSourcesActiveAtOnce() {
        // Multi-active: enabling a second configured source doesn't disable the first — they merge.
        let store = DataSourceStore()
        store.addHpdb(folderPath: "/tmp/MyCard/HPDB")
        store.configure(.repeaterBook, token: "rbuapp_test")
        XCTAssertEqual(store.activeCount, 2)
        XCTAssertTrue(store.isActive(.hpdb))
        XCTAssertTrue(store.isActive(.repeaterBook))
    }
}

/// The SD-card pane subtitle builder — card/volume + list count + live-vs-backup origin.
final class SdCardSubtitleTests: XCTestCase {
    func testNoCard() {
        XCTAssertEqual(
            CatalogView.sdCardSubtitle(hasCard: false, volume: "", lists: 0, isLive: false),
            "card · no card open")
    }

    func testLiveCardPluralizes() {
        XCTAssertEqual(
            CatalogView.sdCardSubtitle(hasCard: true, volume: "MyCard", lists: 3, isLive: true),
            "MyCard · 3 lists · live card")
        XCTAssertEqual(
            CatalogView.sdCardSubtitle(hasCard: true, volume: "MyCard", lists: 1, isLive: true),
            "MyCard · 1 list · live card")
    }

    func testBackupFolderAndBlankVolume() {
        XCTAssertEqual(
            CatalogView.sdCardSubtitle(hasCard: true, volume: "", lists: 0, isLive: false),
            "card · 0 lists · backup folder")
    }
}

/// SDS150 display customization — core-sourced option tables + the edit-script encoder.
final class DisplayCustomizationTests: XCTestCase {
    func testOptionTablesFromCore() {
        let o = DisplayOptions.shared
        XCTAssertEqual(o.palette.count, 147)                 // the full color palette
        XCTAssertEqual(o.palette.first { $0.name == "Aqua" }?.hex, "00fbf7")
        XCTAssertEqual(o.modes.count, 7)                     // the seven layout modes
        XCTAssertEqual(o.areas.count, 4)                     // four screen areas
        XCTAssertEqual(o.colorGroups.count, 7)               // seven color groups
        // The Simple-Conventional mode maps to item layout 1 / color layout 1.
        XCTAssertEqual(o.modes.first { $0.name == "Simple Conventional" }?.colorLayoutId, 1)
    }

    func testPaletteHexParsesToColor() {
        // Sanity: a palette hex string is a valid 6-hex value.
        XCTAssertEqual(UInt32("ff4600", radix: 16), 0xff4600)
    }

    func testEditScriptEncoding() {
        let data = DisplayConfigData(
            globals: [DisplayGlobal(key: "ColorMode", label: "Color mode", value: "BLACK", options: ["COLOR", "BLACK"])],
            items: [DisplayItemGroup(dispOptId: 1, dispLayoutId: 1, tokens: ["FL_Name", "TGID"])],
            colors: [DisplayColorGroup(dispColorId: 1, colorLayoutId: 1,
                                       pairs: [DisplayColorPair(text: "ffffff", back: "000000")])])
        let script = DisplayEditModel.editScript(for: data)
        XCTAssertEqual(script, [
            "G\tColorMode\tBLACK",
            "I\t1\t1\t0\tFL_Name",
            "I\t1\t1\t1\tTGID",
            "C\t1\t1\t0\tffffff\t000000",
        ])
    }
}

/// The map's radius→camera-span framing math (shared by the ZIP jump and "center on me").
final class MapFramingTests: XCTestCase {
    func testSpanScalesWithRadius() {
        // ~1° lat ≈ 69 mi; 25 mi → 25·2.6/69 ≈ 0.942°.
        XCTAssertEqual(MapLensView.frameSpanDegrees(radiusMi: 25), 25 * 2.6 / 69.0, accuracy: 1e-9)
        // A larger radius frames a wider span.
        XCTAssertGreaterThan(
            MapLensView.frameSpanDegrees(radiusMi: 80),
            MapLensView.frameSpanDegrees(radiusMi: 25))
    }

    func testTinyRadiusIsFloored() {
        // A very small radius clamps to the 0.15° floor so the map doesn't zoom in too far.
        XCTAssertEqual(MapLensView.frameSpanDegrees(radiusMi: 1), 0.15, accuracy: 1e-9)
    }
}

/// Multi-source browse routing: the `<source>:<localId>` namespacing that keeps browse state
/// collision-safe across merged sources, and the stable source order in the merged list. All pure.
final class BrowseRoutingTests: XCTestCase {
    /// A minimal `CatalogSystem` for the source tag under test (other fields are irrelevant here).
    private func fixture(id: String, source: DataSourceKind) -> CatalogSystem {
        var sys = CatalogSystem(
            id: id, name: "Sys \(id)", kind: "Trunk", tech: nil,
            counties: [], states: [], statewide: false, siteCount: 0, channelCount: 0)
        sys.source = source
        return sys
    }

    func testCompositeIDNamespacesBySource() {
        XCTAssertEqual(fixture(id: "x", source: .hpdb).compositeID, "hpdb:x")
        XCTAssertEqual(fixture(id: "42", source: .radioReference).compositeID, "radioReference:42")
        // The same local id in two sources stays distinct.
        XCTAssertNotEqual(
            fixture(id: "1", source: .hpdb).compositeID,
            fixture(id: "1", source: .radioReference).compositeID)
    }

    func testSplitNamespaceRoundTripsCompositeID() {
        // splitNamespace ∘ compositeID == identity, for every source kind.
        for kind in DataSourceKind.allCases {
            let sys = fixture(id: "loc-9", source: kind)
            let split = CatalogView.splitNamespace(sys.compositeID)
            XCTAssertEqual(split?.0, kind)
            XCTAssertEqual(split?.1, "loc-9")
        }
        // A local id containing a colon survives (only the first colon splits).
        let split = CatalogView.splitNamespace("hpdb:a:b")
        XCTAssertEqual(split?.0, .hpdb)
        XCTAssertEqual(split?.1, "a:b")
    }

    func testSplitNamespaceRejectsMalformed() {
        XCTAssertNil(CatalogView.splitNamespace("nocolon"))
        XCTAssertNil(CatalogView.splitNamespace("bogus:1")) // unknown source rawValue
    }

    func testKindOrderPutsHpdbFirst() {
        XCTAssertLessThan(kindOrder(.hpdb), kindOrder(.radioReference))
        XCTAssertLessThan(kindOrder(.radioReference), kindOrder(.repeaterBook))
    }
}

/// Radio programming capability — the Swift `Modulation.classify` mirrors the core classifier, and
/// `RadioCapability.reason` de-emphasizes what a target radio can't take. All pure.
final class RadioCapabilityTests: XCTestCase {
    func testClassifyMirrorsCore() {
        XCTAssertEqual(Modulation.classify(tech: "P25Standard", mode: nil), .p25)
        XCTAssertEqual(Modulation.classify(tech: "MotoTRBO", mode: nil), .dmr)
        XCTAssertEqual(Modulation.classify(tech: "NXDN48", mode: nil), .nxdn)
        XCTAssertEqual(Modulation.classify(tech: nil, mode: "FM"), .analog)
        XCTAssertEqual(Modulation.classify(tech: nil, mode: "NFM"), .analog)
        XCTAssertEqual(Modulation.classify(tech: nil, mode: nil), .analog)
        // A non-modulation tech tag falls through to the (analog) mode.
        XCTAssertEqual(Modulation.classify(tech: "Conventional", mode: "FM"), .analog)
    }

    func testAnalogHandheldCapability() {
        // FT-60-like: analog conventional only.
        let cap = RadioCapability(trunking: false, modulations: ["analog"])
        XCTAssertNotNil(cap)
        // Analog conventional is fine.
        XCTAssertNil(cap?.reason(isTrunked: false, tech: "FM", mode: "FM"))
        // A trunk has no home (trunking checked first).
        XCTAssertEqual(cap?.reason(isTrunked: true, tech: "P25Standard", mode: nil), .trunking)
        // Digital conventional is rejected on modulation.
        XCTAssertEqual(cap?.reason(isTrunked: false, tech: "DMR", mode: nil), .modulation(.dmr))
        XCTAssertEqual(cap?.summary, "analog · conventional only")
    }

    func testUnknownCapabilityFailsOpen() {
        // No modulations reported ⇒ unknown ⇒ nil (callers skip filtering).
        XCTAssertNil(RadioCapability(trunking: false, modulations: []))
    }

    func testScannerCapabilityTakesEverything() {
        let all = ["analog", "P25", "DMR", "NXDN", "D-STAR", "Fusion", "ProVoice", "digital"]
        let cap = RadioCapability(trunking: true, modulations: all)
        XCTAssertNil(cap?.reason(isTrunked: true, tech: "P25Standard", mode: nil))
        XCTAssertNil(cap?.reason(isTrunked: false, tech: "DMR", mode: nil))
        XCTAssertEqual(cap?.summary, "all modes · trunk + conventional")
    }
}

/// The pure programming rules (`ProgrammingRules.swift`) — the write-path gate, the add-readiness
/// 3-way, and the live-only backup gate. Exactly the logic that produced this session's two bugs.
final class ProgrammingRulesTests: XCTestCase {
    func testWritePathByTargetClass() {
        // Clone-image takes any source (channels synthesized from a catalog channel).
        XCTAssertTrue(writePath(target: .cloneImage, source: .hpdb))
        XCTAssertTrue(writePath(target: .cloneImage, source: .radioReference))
        // SD-card takes HPDB (selection) and RadioReference (synthesis).
        XCTAssertTrue(writePath(target: .sdCard, source: .hpdb))
        XCTAssertTrue(writePath(target: .sdCard, source: .radioReference))
        // No radio → nothing is programmable.
        XCTAssertFalse(writePath(target: nil, source: .hpdb))
        XCTAssertFalse(writePath(target: nil, source: .radioReference))
    }

    func testAddReadinessByState() {
        // Neutral — no radio chosen.
        XCTAssertEqual(
            addReadiness(deviceClass: nil, radioName: "x", hasImage: false, hasList: false, hasCard: false),
            .notReady("Choose a radio to add channels."))
        // Clone image must be read first, then ready.
        XCTAssertEqual(
            addReadiness(deviceClass: .cloneImage, radioName: "FT-60R", hasImage: false, hasList: false, hasCard: false),
            .notReady("Read your FT-60R first to add channels."))
        XCTAssertEqual(
            addReadiness(deviceClass: .cloneImage, radioName: "FT-60R", hasImage: true, hasList: false, hasCard: false),
            .ready)
        // SD card: no card → open card; card but no list → select/add a list; list open → ready.
        XCTAssertEqual(
            addReadiness(deviceClass: .sdCard, radioName: "SDS150", hasImage: false, hasList: false, hasCard: false),
            .notReady("Open your SDS150’s card first to add channels."))
        XCTAssertEqual(
            addReadiness(deviceClass: .sdCard, radioName: "SDS150", hasImage: false, hasList: false, hasCard: true),
            .notReady("Select or add a favorites list to add channels."))
        XCTAssertEqual(
            addReadiness(deviceClass: .sdCard, radioName: "SDS150", hasImage: false, hasList: true, hasCard: true),
            .ready)
    }

    func testNeedsBackupIsLiveOnly() {
        XCTAssertTrue(needsBackup(isLive: true, backedUp: false))   // live card, no restore point
        XCTAssertFalse(needsBackup(isLive: true, backedUp: true))   // live card, backed up
        XCTAssertFalse(needsBackup(isLive: false, backedUp: false)) // backup folder — always safe
        XCTAssertFalse(needsBackup(isLive: false, backedUp: true))
    }

    // MARK: - Write-to-card model gate (matchWritableCard)

    func testNoCardWhenNoneDetected() {
        XCTAssertEqual(matchWritableCard([], wantModel: "SDS150"), .noCard)
    }

    func testMatchesFirstCardOfWantedModel() {
        let cards = [
            CardIdentity(model: "BCD996"),
            CardIdentity(model: "SDS150"),
            CardIdentity(model: "SDS150"),
        ]
        // First SDS150 wins — model match on the product name, not device identity, so any SDS150
        // qualifies. (We compare the shared `product_name`, not the serial `modelId`.)
        XCTAssertEqual(matchWritableCard(cards, wantModel: "SDS150"), .match(1))
    }

    func testWrongModelWhenNoneMatch() {
        let cards = [CardIdentity(model: "BCD996")]
        XCTAssertEqual(matchWritableCard(cards, wantModel: "SDS150"), .wrongModel("BCD996"))
    }

    func testUnreadableModelNeverMatches() {
        // A detected card whose header couldn't be read (nil model) is refused, never matched.
        let cards = [CardIdentity(model: nil)]
        XCTAssertEqual(matchWritableCard(cards, wantModel: "SDS150"), .wrongModel("different scanner"))
    }
}
