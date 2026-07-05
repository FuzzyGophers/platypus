// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import Foundation

/// A scanner SD card discovered under `/Volumes` (mass-storage mode).
struct DetectedCard: Identifiable {
    let volumeName: String
    let hpdbDir: String // …/<MODEL>/HPDB — what the library opens from
    var id: String { hpdbDir }
}

enum ScannerCard {
    /// Scan mounted volumes for a connected scanner card. A volume qualifies if the
    /// core recognizes a model folder with an HPDB directory holding state files.
    static func detect() -> [DetectedCard] {
        let fm = FileManager.default
        let vols =
            fm.mountedVolumeURLs(
                includingResourceValuesForKeys: [.volumeNameKey], options: [.skipHiddenVolumes]) ?? []
        var out: [DetectedCard] = []
        for v in vols {
            guard let ptr = (v.path.withCString { platypus_card_hpdb_dir($0) }) else { continue }
            defer { platypus_string_free(ptr) }
            let hpdb = String(cString: ptr)
            let name = (try? v.resourceValues(forKeys: [.volumeNameKey]).volumeName) ?? nil
            out.append(DetectedCard(volumeName: name ?? v.lastPathComponent, hpdbDir: hpdb))
        }
        return out
    }

    /// The `…/<MODEL>/HPDB` directory under a card-shaped `volumeRoot` — a live card
    /// volume *or* a backup folder (a full copy has the same layout). Nil if it isn't
    /// a recognized scanner layout. Used to locate the browse DB inside a backup.
    static func hpdbDir(volumeRoot: String) -> String? {
        guard let ptr = (volumeRoot.withCString { platypus_card_hpdb_dir($0) }) else { return nil }
        defer { platypus_string_free(ptr) }
        return String(cString: ptr)
    }

    /// Whether `hpdbDir` is a **live connected card** (a recognized scanner volume
    /// currently mounted under `/Volumes`) as opposed to a backup/library folder
    /// that merely looks like one. Authoritative "the radio is plugged in" check.
    static func isLiveCard(hpdbDir: String) -> Bool {
        let target = URL(fileURLWithPath: hpdbDir).standardizedFileURL.path
        return detect().contains {
            URL(fileURLWithPath: $0.hpdbDir).standardizedFileURL.path == target
        }
    }
}
