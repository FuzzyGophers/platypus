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
        store.setHpdb(enabled: false)
        XCTAssertEqual(store.activeCount, 0)                // no longer merges into the map
        XCTAssertEqual(store.source(.hpdb)?.folderPath, "/tmp/MyCard/HPDB")  // path retained
        store.setHpdb(enabled: true)
        XCTAssertEqual(store.activeCount, 1)
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
