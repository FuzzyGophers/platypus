// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import XCTest

@testable import PlatypusMac

/// `FTTone` value-carrier parsing + display (pure).
final class FTToneTests: XCTestCase {
    func testParseCtcss() {
        XCTAssertEqual(FTTone.parse("TONE=C156.7"), .ctcss(156.7))
        XCTAssertEqual(FTTone.parse("C100.0"), .ctcss(100.0)) // prefix optional
        XCTAssertEqual(FTTone.parse("TONE=C156.7").display, "CTCSS 156.7 Hz")
        XCTAssertEqual(FTTone.parse("TONE=C156.7").modeLabel, "CTCSS")
    }

    func testParseDcs() {
        XCTAssertEqual(FTTone.parse("TONE=D023"), .dcs(23))
        XCTAssertEqual(FTTone.parse("TONE=D023").display, "DCS 023")
        XCTAssertEqual(FTTone.parse("TONE=D023").modeLabel, "DCS")
    }

    func testParseOffAndGarbage() {
        XCTAssertEqual(FTTone.parse(nil), .off)
        XCTAssertEqual(FTTone.parse(""), .off)
        XCTAssertEqual(FTTone.parse("   "), .off)
        XCTAssertEqual(FTTone.parse("TONE=X"), .off) // unrecognized → off, not a crash
        XCTAssertEqual(FTTone.off.display, "—")
        XCTAssertEqual(FTTone.off.modeLabel, "Off")
    }

    func testCrossCarriesBothValues() {
        let t = FTTone.cross(ctcss: 100.0, dcs: 23)
        XCTAssertEqual(t.display, "CTCSS 100.0 Hz · DCS 023")
        XCTAssertEqual(t.modeLabel, "Cross")
        XCTAssertNotEqual(t, .cross(ctcss: 100.0, dcs: 25)) // both halves are identity-bearing
    }
}

/// `Ft60Options` code↔label lookups + duplex semantics (pure over an injected list).
final class Ft60OptionsTests: XCTestCase {
    private let opts = Ft60Options.shared
    private let modes = [
        Ft60Option(label: "FM", code: 0, valueKind: nil),
        Ft60Option(label: "NFM", code: 1, valueKind: nil),
        Ft60Option(label: "AM", code: 2, valueKind: nil),
    ]

    func testLabelAndOptionLookup() {
        XCTAssertEqual(opts.label(modes, 1), "NFM")
        XCTAssertEqual(opts.label(modes, 99), "") // unknown code → empty
        XCTAssertEqual(opts.option(modes, 2)?.label, "AM")
        XCTAssertNil(opts.option(modes, 99))
    }

    func testCodeByLabel() {
        XCTAssertEqual(opts.code(modes, label: "NFM"), 1)
        XCTAssertNil(opts.code(modes, label: "ZZ"))
    }

    func testDuplexSemantics() {
        XCTAssertEqual(Ft60Options.duplexSimplex, 0)
        XCTAssertTrue(opts.duplexNeedsOffset(2)) // −
        XCTAssertTrue(opts.duplexNeedsOffset(3)) // +
        XCTAssertFalse(opts.duplexNeedsOffset(0)) // simplex
        XCTAssertFalse(opts.duplexNeedsOffset(4)) // split
        XCTAssertTrue(opts.duplexIsSplit(4))
        XCTAssertFalse(opts.duplexIsSplit(0))
    }
}

/// `FT60Memory` bank/slot/capacity logic + `makeFromCatalog` mapping.
final class FT60MemoryTests: XCTestCase {
    private func mem(channels: Int = 3, banks: Int = 10, nameLen: Int = 6) -> FT60Memory {
        FT60Memory(capacity: FTCapacity(channels: channels, banks: banks, nameLen: nameLen))
    }

    private func chan(_ slot: Int) -> FT60Channel {
        FT60Channel(
            slot: slot, name: "CH\(slot)", freqHz: 146_520_000, modeCode: 0, tone: .off,
            banks: [], skip: false, powerCode: 0, duplexCode: 0, offsetHz: 0, serviceType: nil)
    }

    func testBankLabel() {
        XCTAssertEqual(FT60Memory.bankLabel(0), "A")
        XCTAssertEqual(FT60Memory.bankLabel(9), "J")
        XCTAssertEqual(FT60Memory.bankLabel(-1), "?")
        XCTAssertEqual(FT60Memory.bankLabel(26), "?")
    }

    func testAppendRespectsCapacity() {
        let m = mem(channels: 2)
        XCTAssertTrue(m.append({ self.chan($0) }, toBank: nil))
        XCTAssertTrue(m.append({ self.chan($0) }, toBank: nil))
        XCTAssertFalse(m.append({ self.chan($0) }, toBank: nil)) // full → no-op false
        XCTAssertEqual(m.count(inBank: nil), 2)
    }

    func testBankMembershipAndUnbanked() {
        let m = mem()
        XCTAssertTrue(m.append({ self.chan($0) }, toBank: 1))
        XCTAssertTrue(m.append({ self.chan($0) }, toBank: nil))
        XCTAssertEqual(m.count(inBank: 1), 1)
        XCTAssertEqual(m.unbanked.count, 1)
        // toggleBank flips membership.
        let slot = m.channels[0].slot
        m.toggleBank(slot: slot, bank: 1)
        XCTAssertEqual(m.count(inBank: 1), 0)
    }

    func testMakeFromCatalogMapsAndTruncates() {
        let m = mem(nameLen: 6)
        m.append(
            m.makeFromCatalog(
                name: "Fireground-Main", freqHz: 146_520_000, mode: "NFM",
                tone: "TONE=C100.0", serviceType: 3),
            toBank: 2)
        let ch = m.channels[0]
        XCTAssertEqual(ch.name, "Firegr") // truncated to nameLen
        XCTAssertEqual(Ft60Options.shared.label(Ft60Options.shared.modes, ch.modeCode), "NFM")
        // A catalog CTCSS tone → TSQL squelch sub-kind + the CTCSS value.
        XCTAssertEqual(
            Ft60Options.shared.label(Ft60Options.shared.toneModes, ch.toneModeCode), "TSQL")
        XCTAssertEqual(ch.tone, .ctcss(100.0))
        XCTAssertTrue(ch.banks.contains(2))
        XCTAssertEqual(ch.serviceType, 3)
    }

    func testFrequencyFormatting() {
        XCTAssertEqual(chan(0).freqMHz, "146.5200")
        XCTAssertTrue(chan(0).detail.contains("146.5200 MHz"))
    }
}

/// PMS band-edge pair grouping (interleaved) + the editor's model mutations.
final class FT60PmsTests: XCTestCase {
    func testGroupInterleaved() {
        // record 2p = lower, 2p+1 = upper (confirmed on hardware).
        let edges = [
            FT60PmsEdgeDTO(index: 0, freqHz: 144_000_000, step: 0),
            FT60PmsEdgeDTO(index: 1, freqHz: 148_000_000, step: 0),
            FT60PmsEdgeDTO(index: 2, freqHz: 440_000_000, step: 3),
        ]
        let pairs = FT60PmsPair.group(edges)
        XCTAssertEqual(pairs.count, 2)
        XCTAssertEqual(pairs[0].pair, 0)
        XCTAssertEqual(pairs[0].lowerHz, 144_000_000)
        XCTAssertEqual(pairs[0].upperHz, 148_000_000)
        XCTAssertEqual(pairs[0].lowerIndex, 0)
        XCTAssertEqual(pairs[0].upperIndex, 1)
        // pair 1 has only a lower edge so far (record index 2).
        XCTAssertEqual(pairs[1].lowerHz, 440_000_000)
        XCTAssertNil(pairs[1].upperHz)
    }

    func testMemoryMutations() {
        let cap = FTCapacity(channels: 1000, banks: 10, nameLen: 6)
        let mem = FT60Memory(
            capacity: cap,
            pms: [FT60PmsPair(pair: 0, lowerHz: 144_000_000, upperHz: 148_000_000)])
        XCTAssertEqual(mem.nextFreePmsPair, 1)
        // edit in place
        mem.updatePms(FT60PmsPair(pair: 0, lowerHz: 145_000_000, upperHz: 147_000_000))
        XCTAssertEqual(mem.pms.count, 1)
        XCTAssertEqual(mem.pms[0].lowerHz, 145_000_000)
        // add a new pair → kept sorted, next free advances
        mem.updatePms(FT60PmsPair(pair: 1, lowerHz: 440_000_000, upperHz: 445_000_000))
        XCTAssertEqual(mem.pms.map { $0.pair }, [0, 1])
        XCTAssertEqual(mem.nextFreePmsPair, 2)
    }
}

/// FT-60 set-mode settings model (pick-list value/label) + editor mutation.
final class FT60SettingsTests: XCTestCase {
    func testValueLabelAndMutation() {
        let lamp = FT60Setting(key: "lamp", label: "Lamp", value: 0, options: ["Key", "5 sec", "Toggle"])
        XCTAssertEqual(lamp.valueLabel, "Key")
        // out-of-range value falls back to the raw number (never crashes).
        XCTAssertEqual(FT60Setting(key: "x", label: "X", value: 9, options: ["a"]).valueLabel, "9")

        let mem = FT60Memory(
            capacity: FTCapacity(channels: 1000, banks: 10, nameLen: 6), settings: [lamp])
        mem.updateSetting(key: "lamp", value: 2)
        XCTAssertEqual(mem.settings[0].value, 2)
        XCTAssertEqual(mem.settings[0].valueLabel, "Toggle")
        mem.updateSetting(key: "missing", value: 1)  // unknown key → no-op
        XCTAssertEqual(mem.settings.count, 1)
    }
}
