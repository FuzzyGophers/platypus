// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import XCTest

@testable import PlatypusMac

/// The owned/active-radio state machine. Uses `UserDefaults.standard` (the test runner's own
/// domain, separate from the app's), cleared between tests for isolation.
final class RadioStoreTests: XCTestCase {
    private let ownedKey = "platypus_owned_radios"
    private let activeKey = "platypus_active_radio"

    override func setUp() {
        super.setUp()
        UserDefaults.standard.removeObject(forKey: ownedKey)
        UserDefaults.standard.removeObject(forKey: activeKey)
    }
    override func tearDown() {
        UserDefaults.standard.removeObject(forKey: ownedKey)
        UserDefaults.standard.removeObject(forKey: activeKey)
        super.tearDown()
    }

    func testFirstRunIsNeutral() {
        let store = RadioStore()
        XCTAssertTrue(store.ownedIDs.isEmpty)
        XCTAssertNil(store.activeID)
    }

    func testFirstOwnedAutoActivates() {
        let store = RadioStore()
        store.add("sds150")
        XCTAssertTrue(store.isOwned("sds150"))
        XCTAssertEqual(store.activeID, "sds150") // first owned becomes active
        store.add("ft60r")
        XCTAssertEqual(store.activeID, "sds150") // adding a second doesn't steal active
    }

    func testSetActiveRejectsUnowned() {
        let store = RadioStore()
        store.add("sds150")
        store.setActive("ft60r") // not owned → ignored
        XCTAssertEqual(store.activeID, "sds150")
        store.add("ft60r")
        store.setActive("ft60r")
        XCTAssertEqual(store.activeID, "ft60r")
    }

    func testRemoveActiveFallsBack() {
        let store = RadioStore()
        store.add("sds150")
        store.add("ft60r")
        store.setActive("ft60r")
        store.remove("ft60r")
        XCTAssertFalse(store.isOwned("ft60r"))
        XCTAssertEqual(store.activeID, "sds150") // fell back to the remaining owned
        store.remove("sds150")
        XCTAssertNil(store.activeID) // none left → neutral
    }

    func testPersistsAcrossInstances() {
        let a = RadioStore()
        a.add("sds150")
        let b = RadioStore() // reads the persisted defaults
        XCTAssertTrue(b.isOwned("sds150"))
        XCTAssertEqual(b.activeID, "sds150")
    }
}

/// The backups index (records CRUD, filtering, newest-first). Test-runner defaults domain.
final class BackupStoreTests: XCTestCase {
    private let indexKey = "backupIndex"

    override func setUp() {
        super.setUp()
        UserDefaults.standard.removeObject(forKey: indexKey)
    }
    override func tearDown() {
        UserDefaults.standard.removeObject(forKey: indexKey)
        super.tearDown()
    }

    private func sig(_ h: String) -> CardBackup.BackupSignature {
        CardBackup.BackupSignature(files: 1, bytes: 10, hash: h)
    }
    private func record(_ folder: String, _ ts: String, model: String = "SDS150", vol: String = "NO NAME")
        -> BackupRecord
    {
        BackupRecord(
            folder: folder, timestamp: ts, model: model, volumeName: vol,
            signature: sig(folder), meta: nil, hpdbMeta: nil)
    }

    func testAppendAndLoad() {
        XCTAssertTrue(BackupStore.load().isEmpty)
        BackupStore.append(record("/a", "2026-07-01 1000"))
        XCTAssertEqual(BackupStore.load().count, 1)
    }

    func testRecordsFilteredAndNewestFirst() {
        BackupStore.append(record("/a", "2026-07-01 1000"))
        BackupStore.append(record("/b", "2026-07-03 0900"))
        BackupStore.append(record("/c", "2026-07-02 0800", model: "FT-60R"))
        let sds = BackupStore.records(model: "SDS150", volumeName: "NO NAME")
        XCTAssertEqual(sds.map { $0.folder }, ["/b", "/a"]) // newest timestamp first, FT-60 excluded
        XCTAssertEqual(BackupStore.latest(model: "SDS150", volumeName: "NO NAME")?.folder, "/b")
        XCTAssertNil(BackupStore.latest(model: "SDS150", volumeName: "OTHER"))
    }

    func testUpdateStatePreservesTimestamp() {
        BackupStore.append(record("/a", "2026-07-01 1000"))
        BackupStore.updateState(folder: "/a", signature: sig("new"), meta: nil)
        let rec = BackupStore.load().first { $0.folder == "/a" }
        XCTAssertEqual(rec?.signature.hash, "new")
        XCTAssertEqual(rec?.timestamp, "2026-07-01 1000") // the restore-point identity is kept
    }
}

/// `CardBackup` content-fingerprint logic — the gate behind "is this card already backed up?"
/// The full backUp/restore drivers are volume-coupled (need a real card mount), so this covers
/// the pure-over-a-directory fingerprint + exclusion rules that decide byte-faithfulness.
final class CardBackupSignatureTests: XCTestCase {
    private var tmp: URL!

    override func setUpWithError() throws {
        tmp = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("platypus-backup-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
    }
    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: tmp)
    }

    /// A card-shaped tree: `<MODEL>/HPDB/s_1.hpd` + `<MODEL>/app_data.cfg`.
    private func makeCard(_ name: String, state: String = "statefile", appData: String) throws -> URL {
        let fm = FileManager.default
        let root = tmp.appendingPathComponent(name)
        let hpdb = root.appendingPathComponent("BCDx36HP/HPDB")
        try fm.createDirectory(at: hpdb, withIntermediateDirectories: true)
        try state.write(to: hpdb.appendingPathComponent("s_1.hpd"), atomically: true, encoding: .utf8)
        try appData.write(
            to: root.appendingPathComponent("BCDx36HP/app_data.cfg"), atomically: true,
            encoding: .utf8)
        return root
    }

    func testAppDataIsExcludedFromSignature() throws {
        // Two cards identical except the volatile resume file → same fingerprint.
        let a = try makeCard("a", appData: "resume-A")
        let b = try makeCard("b", appData: "totally-different-resume")
        XCTAssertEqual(try CardBackup.signature(ofRoot: a).hash, try CardBackup.signature(ofRoot: b).hash)
    }

    func testContentChangeChangesSignature() throws {
        let a = try makeCard("a", appData: "r")
        let c = try makeCard("c", state: "DIFFERENT DATA", appData: "r")
        XCTAssertNotEqual(try CardBackup.signature(ofRoot: a).hash, try CardBackup.signature(ofRoot: c).hash)
    }

    func testSystemVolumeInformationExcluded() throws {
        let a = try makeCard("a", appData: "r")
        let before = try CardBackup.signature(ofRoot: a).hash
        let svi = a.appendingPathComponent("System Volume Information")
        try FileManager.default.createDirectory(at: svi, withIntermediateDirectories: true)
        try "junk".write(to: svi.appendingPathComponent("x"), atomically: true, encoding: .utf8)
        XCTAssertEqual(try CardBackup.signature(ofRoot: a).hash, before)
    }

    func testInventoryExcludesAppData() throws {
        let a = try makeCard("a", state: "statefile", appData: "resume")
        let inv = try CardBackup.inventory(ofRoot: a)
        XCTAssertEqual(inv.files, 1) // only s_1.hpd counts; app_data.cfg excluded
        XCTAssertEqual(inv.bytes, "statefile".utf8.count)
    }

    func testLooksLikeBackup() throws {
        let a = try makeCard("a", appData: "r")
        XCTAssertTrue(CardBackup.looksLikeBackup(a))
        let empty = tmp.appendingPathComponent("empty")
        try FileManager.default.createDirectory(at: empty, withIntermediateDirectories: true)
        XCTAssertFalse(CardBackup.looksLikeBackup(empty))
    }

    func testSameInventory() {
        let s1 = CardBackup.BackupSignature(files: 3, bytes: 100, hash: "a")
        let s2 = CardBackup.BackupSignature(files: 3, bytes: 100, hash: "b")
        let s3 = CardBackup.BackupSignature(files: 4, bytes: 100, hash: "a")
        XCTAssertTrue(s1.sameInventory(as: s2)) // inventory ignores the hash
        XCTAssertFalse(s1.sameInventory(as: s3))
    }
}

/// The full `backUp` / `restore` drivers, run against an injected `CardRoot` pointing at a
/// temp directory (the `CardVolume` abstraction) — so the copy + hash-verify + the
/// `app_data.cfg`-delete CRITICAL RULE are exercised without a mounted card.
final class CardBackupDriverTests: XCTestCase {
    private var tmp: URL!
    private let fm = FileManager.default

    override func setUpWithError() throws {
        tmp = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("platypus-driver-tests-\(UUID().uuidString)")
        try fm.createDirectory(at: tmp, withIntermediateDirectories: true)
    }
    override func tearDownWithError() throws {
        try? fm.removeItem(at: tmp)
    }

    private func write(_ url: URL, _ content: String) throws {
        try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try content.write(to: url, atomically: true, encoding: .utf8)
    }
    private func read(_ url: URL) throws -> String { try String(contentsOf: url, encoding: .utf8) }
    private func exists(_ url: URL) -> Bool { fm.fileExists(atPath: url.path) }

    func testBackUpCopiesEveryFileAndVerifies() throws {
        let card = tmp.appendingPathComponent("card")
        try write(card.appendingPathComponent("BCDx36HP/HPDB/s_1.hpd"), "STATE")
        try write(card.appendingPathComponent("BCDx36HP/favorites_lists/f_1.hpd"), "FAV")
        try write(card.appendingPathComponent("BCDx36HP/app_data.cfg"), "resume")
        let parent = tmp.appendingPathComponent("backups")
        try fm.createDirectory(at: parent, withIntermediateDirectories: true)

        let r = try CardBackup.backUp(
            card: CardRoot(url: card, name: "NO NAME"), into: parent, timestamp: "2026-07-04 1200")

        XCTAssertEqual(r.folder.lastPathComponent, "NO NAME backup 2026-07-04 1200")
        XCTAssertEqual(r.filesVerified, 3) // every file copied *and* hash-verified
        XCTAssertEqual(try read(r.folder.appendingPathComponent("BCDx36HP/HPDB/s_1.hpd")), "STATE")
        XCTAssertEqual(try read(r.folder.appendingPathComponent("BCDx36HP/favorites_lists/f_1.hpd")), "FAV")
        XCTAssertEqual(try read(r.folder.appendingPathComponent("BCDx36HP/app_data.cfg")), "resume")
        // The captured fingerprint matches the source card's (app_data.cfg excluded from it).
        XCTAssertEqual(r.signature.hash, try CardBackup.signature(ofRoot: card).hash)
    }

    func testRestoreReplacesTreeDropsStaleDeletesAppDataAndKeepsUnrelated() throws {
        // A backup folder (card-shaped) with NEW content.
        let backup = tmp.appendingPathComponent("backup")
        try write(backup.appendingPathComponent("BCDx36HP/HPDB/s_1.hpd"), "NEW-STATE")
        try write(backup.appendingPathComponent("BCDx36HP/app_data.cfg"), "resume")
        // A target card: OLD content + a stale file + app_data + an unrelated top-level item.
        let card = tmp.appendingPathComponent("card")
        try write(card.appendingPathComponent("BCDx36HP/HPDB/s_1.hpd"), "OLD-STATE")
        try write(card.appendingPathComponent("BCDx36HP/HPDB/stale.hpd"), "STALE")
        try write(card.appendingPathComponent("BCDx36HP/app_data.cfg"), "old-resume")
        try write(card.appendingPathComponent("OTHER/keep.txt"), "KEEP")

        _ = try CardBackup.restore(from: backup, toCard: CardRoot(url: card, name: "NO NAME"))

        // NEW content restored; the stale file in the replaced tree is dropped.
        XCTAssertEqual(try read(card.appendingPathComponent("BCDx36HP/HPDB/s_1.hpd")), "NEW-STATE")
        XCTAssertFalse(exists(card.appendingPathComponent("BCDx36HP/HPDB/stale.hpd")))
        // app_data.cfg deleted after restore (the resume-state CRITICAL RULE).
        XCTAssertFalse(exists(card.appendingPathComponent("BCDx36HP/app_data.cfg")))
        // A top-level item not in the backup is left untouched (restore replaces, not mirrors).
        XCTAssertEqual(try read(card.appendingPathComponent("OTHER/keep.txt")), "KEEP")
    }

    func testBackUpThenRestoreRoundTrips() throws {
        let card = tmp.appendingPathComponent("card")
        try write(card.appendingPathComponent("BCDx36HP/HPDB/s_1.hpd"), "DATA")
        try write(card.appendingPathComponent("BCDx36HP/app_data.cfg"), "resume")
        let parent = tmp.appendingPathComponent("backups")
        try fm.createDirectory(at: parent, withIntermediateDirectories: true)

        let bk = try CardBackup.backUp(
            card: CardRoot(url: card, name: "NO NAME"), into: parent, timestamp: "t")

        // Restore that backup into a fresh, empty card.
        let card2 = tmp.appendingPathComponent("card2")
        try fm.createDirectory(at: card2, withIntermediateDirectories: true)
        _ = try CardBackup.restore(from: bk.folder, toCard: CardRoot(url: card2, name: "NO NAME"))

        XCTAssertEqual(try read(card2.appendingPathComponent("BCDx36HP/HPDB/s_1.hpd")), "DATA")
        XCTAssertFalse(exists(card2.appendingPathComponent("BCDx36HP/app_data.cfg"))) // deleted on restore
    }
}
