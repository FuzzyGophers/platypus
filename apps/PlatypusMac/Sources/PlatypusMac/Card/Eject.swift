// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import AppKit
import Foundation

/// Safe ejection of the SD card volume — the #1 Uniden write gotcha is *eject
/// before reconnecting to the scanner*; a buffered write that isn't flushed is
/// lost or half-applied (corrupting the card). The app drives this so the user
/// never has to remember it.
/// A card's filesystem root: the mount point (or, in tests, any directory) and its display
/// name. Real callers resolve one from a file on the card via `CardVolume.forFile`; tests
/// construct one pointing at a temp directory so the backup/restore drivers run against a fake
/// card instead of a mounted volume.
struct CardRoot {
    let url: URL
    let name: String
}

enum CardVolume {
    /// The mount point and display name of the volume that contains `path`.
    static func forFile(_ path: String) -> CardRoot? {
        let url = URL(fileURLWithPath: path)
        guard
            let values = try? url.resourceValues(forKeys: [.volumeURLKey, .volumeNameKey]),
            let volumeURL = values.volume
        else { return nil }
        return CardRoot(url: volumeURL, name: values.volumeName ?? volumeURL.lastPathComponent)
    }

    /// Flush + unmount + eject the volume holding `path`. On success the card is
    /// safe to remove and the scanner may leave USB Mass Storage. Throws if the
    /// volume can't be found or the eject fails (e.g. still busy) — in which case
    /// the caller must NOT tell the user the card is safe.
    static func eject(fileOnCard path: String) throws -> String {
        guard let volume = forFile(path) else {
            throw NSError(
                domain: "Platypus", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Couldn't locate the card's volume."])
        }
        try NSWorkspace.shared.unmountAndEjectDevice(at: volume.url)
        return volume.name
    }
}
