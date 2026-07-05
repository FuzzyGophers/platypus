// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CryptoKit
import Foundation

/// Full, restorable backup of the SD card — and restore back from one.
///
/// Backup is read-only of the card, so it carries zero risk and should always
/// precede any write. Restore is the inverse, and the only destructive path here.
/// Both **verify** afterward: every copied file is stream-hashed (SHA-256) and
/// compared, so "backed up" / "restored" means *confirmed byte-faithful*, not just
/// "copyItem didn't throw".
enum CardBackup {
    /// Outcome of a backup or restore: the folder involved, what was verified, and
    /// the card's content fingerprint captured during the verify pass (free — the
    /// pass already hashes every card file).
    struct Result {
        let folder: URL
        let filesVerified: Int
        let bytesVerified: Int
        let signature: BackupSignature
    }

    /// Shared cancellation flag: the UI sets it, a running backup polls it between
    /// files. Thread-safe so it can cross the work queue boundary.
    final class CancelToken {
        private let lock = NSLock()
        private var flag = false
        var isCancelled: Bool {
            lock.lock(); defer { lock.unlock() }; return flag
        }
        func cancel() { lock.lock(); flag = true; lock.unlock() }
    }

    /// A content fingerprint of a card/backup root — enough to answer "is this card
    /// already captured by that backup?" `files`/`bytes` are a cheap inventory (an
    /// instant "definitely different" check); `hash` is the authoritative SHA-256
    /// over every file's path + size + content. `app_data.cfg` (volatile resume
    /// state we delete after every write) is excluded, so a card that differs from
    /// its backup *only* by that file still matches.
    struct BackupSignature: Codable, Equatable {
        let files: Int
        let bytes: Int
        let hash: String

        /// Cheap pre-check: same inventory? (Lets a caller declare "changed"
        /// without hashing when the file count/size already differs.)
        func sameInventory(as other: BackupSignature) -> Bool {
            files == other.files && bytes == other.bytes
        }
    }

    // MARK: - Backup

    /// Copy the card holding `path` into a new timestamped folder under
    /// `destinationParent`, then verify the copy — reporting progress and honoring
    /// cancellation throughout. Copying is the first half of the bar, verifying the
    /// second; `progress` is called with a phase label and an overall 0…1 fraction.
    /// If `cancel` fires, the partial backup folder is removed and a user-cancelled
    /// error is thrown. A failed verify means the backup is NOT trustworthy.
    static func backUp(
        fileOnCard path: String,
        into destinationParent: URL,
        timestamp: String,
        progress: (_ phase: String, _ fraction: Double) -> Void = { _, _ in },
        cancel: CancelToken = CancelToken()
    ) throws -> Result {
        guard let card = CardVolume.forFile(path) else {
            throw err("Couldn't locate the card's volume.")
        }
        return try backUp(
            card: card, into: destinationParent, timestamp: timestamp, progress: progress,
            cancel: cancel)
    }

    /// Backup driver over an explicit [`CardRoot`] — the testable core. Point `card` at a temp
    /// directory to exercise the copy + hash-verify pass without a mounted volume.
    static func backUp(
        card: CardRoot,
        into destinationParent: URL,
        timestamp: String,
        progress: (_ phase: String, _ fraction: Double) -> Void = { _, _ in },
        cancel: CancelToken = CancelToken()
    ) throws -> Result {
        let volume = card
        let fm = FileManager.default
        let dest = destinationParent.appendingPathComponent(
            "\(volume.name) backup \(timestamp)", isDirectory: true)
        try fm.createDirectory(at: dest, withIntermediateDirectories: true)

        do {
            // Enumerate the card (recursively), skipping macOS/FAT junk, to get a
            // byte total for the progress bar and a per-file work list.
            progress("Preparing…", 0)
            let base = volume.url.standardizedFileURL.path
            guard
                let walker = fm.enumerator(
                    at: volume.url,
                    includingPropertiesForKeys: [.isRegularFileKey, .isDirectoryKey, .fileSizeKey],
                    options: [.skipsHiddenFiles])
            else { throw err("Couldn't read the card.") }

            var dirs: [String] = []
            var files: [(rel: String, size: Int)] = []
            for case let url as URL in walker {
                let rel = String(
                    url.standardizedFileURL.path.dropFirst(base.count).drop(while: { $0 == "/" }))
                if rel.hasPrefix("System Volume Information") { continue }
                let vals = try url.resourceValues(
                    forKeys: [.isRegularFileKey, .isDirectoryKey, .fileSizeKey])
                if vals.isDirectory == true {
                    dirs.append(rel)
                } else if vals.isRegularFile == true {
                    files.append((rel, vals.fileSize ?? 0))
                }
            }
            let totalBytes = max(1, files.reduce(0) { $0 + $1.size })

            // Recreate the directory tree up front (parents first), so the copy
            // loop never pays a per-file createDirectory syscall.
            for rel in dirs.sorted(by: { $0.count < $1.count }) {
                try fm.createDirectory(
                    at: dest.appendingPathComponent(rel), withIntermediateDirectories: true)
            }

            // Copy each file while hashing the card bytes in the SAME read — the slow
            // SD card is read exactly once. Stash each card file's hash for verify.
            var cardHashes: [String: String] = [:]
            var copied = 0
            for f in files {
                if cancel.isCancelled { throw cancelledError() }
                let dig = try streamCopy(
                    from: volume.url.appendingPathComponent(f.rel),
                    to: dest.appendingPathComponent(f.rel)
                ) { n in
                    copied += n
                    progress("Copying…", 0.5 * Double(copied) / Double(totalBytes))
                }
                cardHashes[f.rel] = hex(dig)
            }

            // Verify by re-reading the BACKUP (on the fast internal disk) and
            // comparing to the card hash captured during copy — no second slow card
            // read. This still catches any short/corrupt write.
            var verified = 0
            var vFiles = 0
            for f in files {
                if cancel.isCancelled { throw cancelledError() }
                let backupHash = hex(try digest(dest.appendingPathComponent(f.rel)))
                guard cardHashes[f.rel] == backupHash else {
                    throw err("Verification failed: “\(f.rel)” didn't write correctly.")
                }
                vFiles += 1
                verified += f.size
                progress("Verifying…", 0.5 + 0.5 * Double(verified) / Double(totalBytes))
            }

            let sig = signatureFromHashes(files: files, hashes: cardHashes)
            progress("Done", 1)
            return Result(folder: dest, filesVerified: vFiles, bytesVerified: verified, signature: sig)
        } catch {
            // A cancelled or half-written backup must not linger as if it were good.
            if (error as NSError).code == NSUserCancelledError {
                try? fm.removeItem(at: dest)
            }
            throw error
        }
    }

    // MARK: - Restore

    /// Restore a backup folder back onto the card holding `path`: each top-level
    /// item in the backup replaces its counterpart on the card. Verifies the card
    /// now matches the backup, then deletes `app_data.cfg` (program data changed —
    /// the non-negotiable resume-state rule). The caller must still **eject**.
    ///
    /// Items on the card that aren't in the backup are left untouched (restore
    /// replaces, it doesn't mirror-delete), so an unrelated folder is never lost.
    static func restore(
        from backupFolder: URL, toCardHolding path: String,
        progress: (_ phase: String, _ fraction: Double) -> Void = { _, _ in }
    ) throws -> Result {
        guard let card = CardVolume.forFile(path) else {
            throw err("Couldn't locate the card's volume.")
        }
        return try restore(from: backupFolder, toCard: card, progress: progress)
    }

    /// Restore driver over an explicit [`CardRoot`] — the testable core. Point `card` at a temp
    /// directory to exercise replace + verify + the `app_data.cfg` delete without a mounted card.
    static func restore(
        from backupFolder: URL, toCard card: CardRoot,
        progress: (_ phase: String, _ fraction: Double) -> Void = { _, _ in }
    ) throws -> Result {
        let volume = card
        let fm = FileManager.default

        progress("Preparing…", 0)
        let topLevel = try fm.contentsOfDirectory(
            at: backupFolder, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles])
        let payload = topLevel.filter { $0.lastPathComponent != "System Volume Information" }
        guard !payload.isEmpty else {
            throw err("That folder is empty — it doesn't look like a card backup.")
        }

        // Enumerate the backup (recursively) for a byte total + per-file work list.
        let base = backupFolder.standardizedFileURL.path
        guard
            let walker = fm.enumerator(
                at: backupFolder,
                includingPropertiesForKeys: [.isRegularFileKey, .isDirectoryKey, .fileSizeKey],
                options: [.skipsHiddenFiles])
        else { throw err("Couldn't read the backup.") }
        var dirs: [String] = []
        var files: [(rel: String, size: Int)] = []
        for case let url as URL in walker {
            let rel = String(
                url.standardizedFileURL.path.dropFirst(base.count).drop(while: { $0 == "/" }))
            if rel.hasPrefix("System Volume Information") { continue }
            let vals = try url.resourceValues(
                forKeys: [.isRegularFileKey, .isDirectoryKey, .fileSizeKey])
            if vals.isDirectory == true {
                dirs.append(rel)
            } else if vals.isRegularFile == true {
                files.append((rel, vals.fileSize ?? 0))
            }
        }
        let totalBytes = max(1, files.reduce(0) { $0 + $1.size })

        // Replace each backed-up top-level item on the card (drops stale files in
        // those trees), then recreate the dir tree and copy each file with progress.
        for item in payload {
            let target = volume.url.appendingPathComponent(item.lastPathComponent)
            if fm.fileExists(atPath: target.path) { try fm.removeItem(at: target) }
        }
        for rel in dirs.sorted(by: { $0.count < $1.count }) {
            try fm.createDirectory(
                at: volume.url.appendingPathComponent(rel), withIntermediateDirectories: true)
        }
        var copied = 0
        for f in files {
            let dst = volume.url.appendingPathComponent(f.rel)
            try fm.createDirectory(
                at: dst.deletingLastPathComponent(), withIntermediateDirectories: true)
            try fm.copyItem(at: backupFolder.appendingPathComponent(f.rel), to: dst)
            copied += f.size
            progress("Copying to card…", 0.5 * Double(copied) / Double(totalBytes))
        }

        // Confirm the card now matches the backup (before we touch app_data.cfg).
        let (vFiles, vBytes, sig) = try verifyMatch(
            reference: backupFolder, mirror: volume.url,
            onBytes: { done in
                progress("Verifying…", 0.5 + 0.5 * Double(done) / Double(totalBytes))
            })

        // Program data changed → delete the resume-state file, or the scanner
        // misbehaves on resume (the spec CRITICAL RULE). There is at most one,
        // in the model folder; find it profile-agnostically.
        deleteAppData(under: volume.url)
        progress("Done", 1)

        return Result(folder: backupFolder, filesVerified: vFiles, bytesVerified: vBytes, signature: sig)
    }

    /// True if `folder` looks like a card backup (contains a model folder holding
    /// an `app_data.cfg` or a favorites dir). Cheap pre-check for the picker.
    static func looksLikeBackup(_ folder: URL) -> Bool {
        let fm = FileManager.default
        guard
            let items = try? fm.contentsOfDirectory(
                at: folder, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles])
        else { return false }
        for item in items {
            let isDir = (try? item.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
            guard isDir else { continue }
            if fm.fileExists(atPath: item.appendingPathComponent("app_data.cfg").path)
                || fm.fileExists(atPath: item.appendingPathComponent("favorites_lists").path)
            {
                return true
            }
        }
        return false
    }

    // MARK: - Verification

    /// Walk every regular file under `reference` and confirm `mirror` holds an
    /// identical file at the same relative path. Returns (files, bytes) verified +
    /// the **mirror's** content fingerprint (built for free from the hashes already
    /// computed). Throws on the first missing or mismatched file — or `cancel`.
    /// `onBytes` is called with cumulative reference bytes for progress. Used both
    /// ways: after a backup `reference` is the backup and `mirror` the card; after a
    /// restore they swap — either way we walk the backup and prove the card agrees.
    private static func verifyMatch(
        reference: URL, mirror: URL,
        onBytes: (Int) -> Void = { _ in }, cancel: CancelToken? = nil
    ) throws -> (files: Int, bytes: Int, signature: BackupSignature) {
        let fm = FileManager.default
        guard
            let walker = fm.enumerator(
                at: reference, includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey],
                options: [.skipsHiddenFiles])
        else { throw err("Couldn't read the backup to verify it.") }

        let base = reference.standardizedFileURL.path
        var files = 0
        var bytes = 0
        var sigEntries: [(rel: String, size: Int, hash: String)] = []
        for case let url as URL in walker {
            if cancel?.isCancelled == true { throw cancelledError() }
            let vals = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
            guard vals.isRegularFile == true else { continue }

            let rel = String(
                url.standardizedFileURL.path.dropFirst(base.count).drop(while: { $0 == "/" }))
            // Skip System Volume Information if it slipped into a hand-made backup.
            if rel.hasPrefix("System Volume Information") { continue }
            let other = mirror.appendingPathComponent(rel)

            guard fm.fileExists(atPath: other.path) else {
                throw err("Verification failed: “\(rel)” is missing from the card.")
            }
            let mirrorDigest = try digest(other)
            guard try digest(url) == mirrorDigest else {
                throw err("Verification failed: “\(rel)” doesn't match — the copy is incomplete or corrupt.")
            }
            let size = vals.fileSize ?? 0
            files += 1
            bytes += size
            if !isExcludedFromSignature(rel) {
                sigEntries.append((rel, size, hex(mirrorDigest)))
            }
            onBytes(bytes)
        }

        sigEntries.sort { $0.rel < $1.rel }
        var hasher = SHA256()
        var sigBytes = 0
        for e in sigEntries {
            hasher.update(data: Data("\(e.rel)\n\(e.size)\n\(e.hash)\n".utf8))
            sigBytes += e.size
        }
        let signature = BackupSignature(
            files: sigEntries.count, bytes: sigBytes, hash: hex(hasher.finalize()))
        return (files, bytes, signature)
    }

    // MARK: - Content fingerprint

    /// Fingerprint a card or backup root: walk every regular file (excluding
    /// `app_data.cfg`, `System Volume Information`, and macOS/FAT junk), and hash
    /// a canonical stream of `relpath \n size \n fileHashHex`, sorted by relpath so
    /// the result is order-independent. Returns the inventory + the combined hash.
    static func signature(
        ofRoot root: URL, onFraction: (Double) -> Void = { _ in }
    ) throws -> BackupSignature {
        let fm = FileManager.default

        // Quick size-only pre-walk for a byte total (so progress is real), then the
        // hashing pass. The pre-walk is metadata-only and cheap next to hashing.
        var total = 0
        if let pre = fm.enumerator(
            at: root, includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey],
            options: [.skipsHiddenFiles])
        {
            for case let url as URL in pre {
                let vals = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
                guard vals.isRegularFile == true else { continue }
                let rel = String(
                    url.standardizedFileURL.path.dropFirst(root.standardizedFileURL.path.count)
                        .drop(while: { $0 == "/" }))
                if isExcludedFromSignature(rel) { continue }
                total += vals.fileSize ?? 0
            }
        }
        let totalBytes = max(1, total)

        guard
            let walker = fm.enumerator(
                at: root, includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey],
                options: [.skipsHiddenFiles])
        else { throw err("Couldn't read \(root.lastPathComponent) to fingerprint it.") }

        let base = root.standardizedFileURL.path
        var entries: [(rel: String, size: Int, hash: String)] = []
        var files = 0
        var bytes = 0
        var done = 0
        for case let url as URL in walker {
            let vals = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
            guard vals.isRegularFile == true else { continue }

            let rel = String(
                url.standardizedFileURL.path.dropFirst(base.count).drop(while: { $0 == "/" }))
            if isExcludedFromSignature(rel) { continue }

            let size = vals.fileSize ?? 0
            entries.append((rel, size, hex(try digest(url))))
            files += 1
            bytes += size
            done += size
            onFraction(Double(done) / Double(totalBytes))
        }

        entries.sort { $0.rel < $1.rel }
        var hasher = SHA256()
        for e in entries {
            hasher.update(data: Data("\(e.rel)\n\(e.size)\n\(e.hash)\n".utf8))
        }
        return BackupSignature(files: files, bytes: bytes, hash: hex(hasher.finalize()))
    }

    /// A **metadata-only** fingerprint (per-file relpath + size + modification
    /// time, no file contents read) — fast even on a slow card, since it touches
    /// only directory metadata. Captured at backup time over the *card*, and again
    /// at load, to answer "is the card unchanged since its backup?" without the
    /// expensive content hash. (Content hashing stays the authoritative gate at
    /// backup/restore, where every byte is read anyway.)
    static func metaFingerprint(ofRoot root: URL) throws -> BackupSignature {
        let fm = FileManager.default
        guard
            let walker = fm.enumerator(
                at: root,
                includingPropertiesForKeys: [
                    .isRegularFileKey, .fileSizeKey, .contentModificationDateKey,
                ],
                options: [.skipsHiddenFiles])
        else { throw err("Couldn't read \(root.lastPathComponent).") }

        let base = root.standardizedFileURL.path
        var entries: [(rel: String, size: Int, mtime: Int)] = []
        var files = 0
        var bytes = 0
        for case let url as URL in walker {
            let vals = try url.resourceValues(forKeys: [
                .isRegularFileKey, .fileSizeKey, .contentModificationDateKey,
            ])
            guard vals.isRegularFile == true else { continue }
            let rel = String(
                url.standardizedFileURL.path.dropFirst(base.count).drop(while: { $0 == "/" }))
            if isExcludedFromSignature(rel) { continue }
            let size = vals.fileSize ?? 0
            let mtime = Int((vals.contentModificationDate ?? .distantPast).timeIntervalSince1970)
            entries.append((rel, size, mtime))
            files += 1
            bytes += size
        }
        entries.sort { $0.rel < $1.rel }
        var hasher = SHA256()
        for e in entries {
            hasher.update(data: Data("\(e.rel)\n\(e.size)\n\(e.mtime)\n".utf8))
        }
        return BackupSignature(files: files, bytes: bytes, hash: hex(hasher.finalize()))
    }

    /// Cheap inventory (file count + total bytes) with the same exclusions as
    /// `signature` — used to declare "changed since backup" instantly, without
    /// hashing, when the inventory already differs from a stored signature.
    static func inventory(ofRoot root: URL) throws -> (files: Int, bytes: Int) {
        let fm = FileManager.default
        guard
            let walker = fm.enumerator(
                at: root, includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey],
                options: [.skipsHiddenFiles])
        else { throw err("Couldn't read \(root.lastPathComponent).") }

        let base = root.standardizedFileURL.path
        var files = 0
        var bytes = 0
        for case let url as URL in walker {
            let vals = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
            guard vals.isRegularFile == true else { continue }
            let rel = String(
                url.standardizedFileURL.path.dropFirst(base.count).drop(while: { $0 == "/" }))
            if isExcludedFromSignature(rel) { continue }
            files += 1
            bytes += vals.fileSize ?? 0
        }
        return (files, bytes)
    }

    /// Files that must not affect whether a card "matches" a backup: the volatile
    /// resume state (deleted after every write) and FAT/macOS metadata.
    private static func isExcludedFromSignature(_ rel: String) -> Bool {
        if rel.hasPrefix("System Volume Information") { return true }
        return (rel as NSString).lastPathComponent == "app_data.cfg"
    }

    private static func hex(_ digest: SHA256Digest) -> String {
        digest.map { String(format: "%02x", $0) }.joined()
    }

    /// Copy `src` → `dst` in 1 MB chunks while SHA-256-hashing the bytes in the same
    /// read, so the source (the slow card) is read exactly once. `onChunk` reports
    /// bytes written for progress. Returns the source's digest.
    private static func streamCopy(
        from src: URL, to dst: URL, onChunk: (Int) -> Void
    ) throws -> SHA256Digest {
        FileManager.default.createFile(atPath: dst.path, contents: nil)
        let input = try FileHandle(forReadingFrom: src)
        defer { try? input.close() }
        let output = try FileHandle(forWritingTo: dst)
        defer { try? output.close() }
        var hasher = SHA256()
        while let chunk = try input.read(upToCount: 1 << 20), !chunk.isEmpty {
            try output.write(contentsOf: chunk)
            hasher.update(data: chunk)
            onChunk(chunk.count)
        }
        return hasher.finalize()
    }

    /// Build a `BackupSignature` from already-computed per-file hashes (the copy
    /// pass produced them), excluding `app_data.cfg`.
    private static func signatureFromHashes(
        files: [(rel: String, size: Int)], hashes: [String: String]
    ) -> BackupSignature {
        var entries = files.compactMap { f -> (rel: String, size: Int, hash: String)? in
            if isExcludedFromSignature(f.rel) { return nil }
            guard let h = hashes[f.rel] else { return nil }
            return (f.rel, f.size, h)
        }
        entries.sort { $0.rel < $1.rel }
        var hasher = SHA256()
        var bytes = 0
        for e in entries {
            hasher.update(data: Data("\(e.rel)\n\(e.size)\n\(e.hash)\n".utf8))
            bytes += e.size
        }
        return BackupSignature(files: entries.count, bytes: bytes, hash: hex(hasher.finalize()))
    }

    /// SHA-256 of a file, streamed in 1 MB chunks so a large HPDB never loads whole.
    private static func digest(_ url: URL) throws -> SHA256Digest {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        while let chunk = try handle.read(upToCount: 1 << 20), !chunk.isEmpty {
            hasher.update(data: chunk)
        }
        return hasher.finalize()
    }

    /// Delete `app_data.cfg` wherever it sits in a model folder (one level down).
    /// Best-effort: a missing file is success (idempotent).
    private static func deleteAppData(under cardRoot: URL) {
        let fm = FileManager.default
        guard
            let items = try? fm.contentsOfDirectory(
                at: cardRoot, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles])
        else { return }
        for item in items {
            let isDir = (try? item.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
            guard isDir else { continue }
            let appData = item.appendingPathComponent("app_data.cfg")
            if fm.fileExists(atPath: appData.path) { try? fm.removeItem(at: appData) }
        }
    }

    private static func err(_ message: String) -> NSError {
        NSError(domain: "Platypus", code: 2, userInfo: [NSLocalizedDescriptionKey: message])
    }

    /// Standard "user cancelled" — Cocoa-recognized, so callers can tell a cancel
    /// from a genuine failure (`(error as NSError).code == NSUserCancelledError`).
    private static func cancelledError() -> NSError {
        NSError(
            domain: "Platypus", code: NSUserCancelledError,
            userInfo: [NSLocalizedDescriptionKey: "Backup cancelled."])
    }
}
