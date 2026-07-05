// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import Foundation

/// One backup we've created, recorded so the app can later answer "does the card
/// in hand already match a backup?" The `signature` is the card's content
/// fingerprint at the moment of backup (see `CardBackup.BackupSignature`).
struct BackupRecord: Codable, Identifiable {
    let folder: String  // absolute path to the backup folder
    let timestamp: String  // "yyyy-MM-dd HHmm" — sorts chronologically
    let model: String  // e.g. "SDS150"
    let volumeName: String  // the card's FAT label (weak id; signature is authoritative)
    let signature: CardBackup.BackupSignature  // content hash (authoritative)
    /// The card's metadata fingerprint (size + mtime) at backup time — the fast
    /// "is the card still unchanged?" check at load. Optional for legacy records.
    var meta: CardBackup.BackupSignature?
    /// Metadata fingerprint of **just the browse DB** (the `…/<MODEL>/HPDB` dir) at
    /// backup time. Gates loading the heavy browse library from this backup (fast
    /// SSD) instead of re-reading the slow card, when the card's browse DB is
    /// unchanged since backup. Favorites are always read live from the card, so this
    /// deliberately ignores favorites/resume churn. Optional for legacy records.
    var hpdbMeta: CardBackup.BackupSignature?
    var id: String { folder }
}

/// Where backups live and the index of the ones we've made. The index is the only
/// way to know whether a card is *already* backed up — cards have no stable serial
/// (model id is static, FAT labels aren't unique), so identity is by content
/// `signature`, with model + volume name used only to narrow the candidate list.
enum BackupStore {
    private static let rootKey = "backupRoot"
    private static let indexKey = "backupIndex"

    /// The managed backups folder. Defaults to `~/Documents/Platypus Backups`,
    /// overridable (the user can move it). Created on demand.
    static var root: URL {
        if let custom = UserDefaults.standard.string(forKey: rootKey) {
            return URL(fileURLWithPath: custom, isDirectory: true)
        }
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Documents")
        return docs.appendingPathComponent("Platypus Backups", isDirectory: true)
    }

    /// Point the managed location somewhere else (persisted).
    static func setRoot(_ url: URL) {
        UserDefaults.standard.set(url.path, forKey: rootKey)
    }

    /// Ensure `root` exists, returning it.
    static func ensureRoot() throws -> URL {
        let r = root
        try FileManager.default.createDirectory(at: r, withIntermediateDirectories: true)
        return r
    }

    // MARK: - Index

    static func load() -> [BackupRecord] {
        guard let data = UserDefaults.standard.data(forKey: indexKey) else { return [] }
        return (try? JSONDecoder().decode([BackupRecord].self, from: data)) ?? []
    }

    static func append(_ record: BackupRecord) {
        var all = load()
        all.append(record)
        save(all)
    }

    /// Fill in the metadata fingerprint of an existing (legacy) record, so future
    /// loads can use the fast metadata check instead of re-hashing the card.
    static func backfillMeta(folder: String, meta: CardBackup.BackupSignature) {
        var all = load()
        guard let i = all.firstIndex(where: { $0.folder == folder }), all[i].meta == nil else { return }
        all[i].meta = meta
        save(all)
    }

    /// Re-point an existing backup's fingerprints after a card write was **mirrored**
    /// into it (the favorites save replays the same writes into the backup folder so
    /// it stays a faithful copy). Updating `meta`/`signature` to the post-save card
    /// keeps the load-time "up to date" check green — so the next launch loads from
    /// the SSD copy instead of re-reading the whole card. The timestamp (the restore
    /// point's identity) is preserved.
    static func updateState(
        folder: String, signature: CardBackup.BackupSignature, meta: CardBackup.BackupSignature?
    ) {
        var all = load()
        guard let i = all.firstIndex(where: { $0.folder == folder }) else { return }
        // Preserve hpdbMeta: a favorites save doesn't touch the browse DB, so the
        // backup's browse copy still matches the card.
        all[i] = BackupRecord(
            folder: all[i].folder, timestamp: all[i].timestamp, model: all[i].model,
            volumeName: all[i].volumeName, signature: signature, meta: meta,
            hpdbMeta: all[i].hpdbMeta)
        save(all)
    }

    private static func save(_ records: [BackupRecord]) {
        if let data = try? JSONEncoder().encode(records) {
            UserDefaults.standard.set(data, forKey: indexKey)
        }
    }

    /// Records for a given card (model + volume), newest first.
    static func records(model: String, volumeName: String) -> [BackupRecord] {
        load()
            .filter { $0.model == model && $0.volumeName == volumeName }
            .sorted { $0.timestamp > $1.timestamp }
    }

    /// The most recent backup recorded for a given card, if any.
    static func latest(model: String, volumeName: String) -> BackupRecord? {
        records(model: model, volumeName: volumeName).first
    }
}
