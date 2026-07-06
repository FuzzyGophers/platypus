// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import Foundation

/// Swift bridge over the FT-60 serial clone-read FFI. Read-only — populates the FT-60 memory
/// model from the radio; no bytes are written to the radio's memory.
enum Ft60 {
    /// Candidate serial ports (`/dev/cu.*`).
    static func listPorts() -> [String] {
        guard let c = platypus_serial_ports_json() else { return [] }
        defer { platypus_string_free(c) }
        let data = Data(String(cString: c).utf8)
        return (try? JSONDecoder().decode([String].self, from: data)) ?? []
    }

    struct ReadError: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    /// Boxes the progress + cancel closures so they can cross the C ABI via one opaque ctx.
    final class Callbacks {
        let progress: (Double) -> Void
        let isCancelled: () -> Bool
        init(progress: @escaping (Double) -> Void, isCancelled: @escaping () -> Bool) {
            self.progress = progress
            self.isCancelled = isCancelled
        }
    }

    /// Read + decode an FT-60 image over `port`. **Synchronous** — call off the main thread.
    /// `progress` is 0…1; `isCancelled` is polled to abort. Returns the decoded channels plus the
    /// raw image bytes (kept so the same image can be written back). Every read is **backed up**:
    /// the raw image is saved to `<backupDir>/<backupStem>.img` by the core FFI (fsync'd) before the
    /// data is surfaced — `backupPath` is the saved file, or `backupError` explains why it couldn't
    /// be written (the read still succeeds). Throws only on read/decode failure.
    static func read(port: String, backupDir: String, backupStem: String,
                     progress: @escaping (Double) -> Void,
                     isCancelled: @escaping () -> Bool) throws
        -> (channels: [FT60Channel], image: [UInt8], pms: [FT60PmsPair], settings: [FT60Setting],
            backupPath: String?, backupError: String?) {
        let box = Unmanaged.passRetained(Callbacks(progress: progress, isCancelled: isCancelled))
        defer { box.release() }
        let ctx = box.toOpaque()

        var err: UnsafeMutablePointer<CChar>?
        let handle = port.withCString {
            platypus_ft60_read($0, ctx, ft60ProgressTrampoline, ft60CancelTrampoline, &err)
        }
        guard let handle else {
            let msg = err.map { String(cString: $0) } ?? "read failed"
            if let err { platypus_string_free(err) }
            throw ReadError(message: msg)
        }
        defer { platypus_ft60_free(handle) }

        // Restore point first: persist the raw image (Rust fsync'd write) before surfacing data.
        var backupPath: String?
        var backupError: String?
        var berr: UnsafeMutablePointer<CChar>?
        let bpath = backupDir.withCString { d in
            backupStem.withCString { s in platypus_ft60_backup(handle, d, s, &berr) }
        }
        if let bpath {
            backupPath = String(cString: bpath)
            platypus_string_free(bpath)
        } else {
            backupError = berr.map { String(cString: $0) } ?? "backup failed"
            if let berr { platypus_string_free(berr) }
        }

        let image = imageBytes(handle)
        return (decodeChannels(handle), image, decodePms(handle), decodeSettings(handle),
                backupPath, backupError)
    }

    /// Decode a saved backup **image** into the editable model — no serial, no backup. Validates by
    /// decoding it in core (apply with no edits = decode + no-op); throws on a bad/foreign image.
    /// The caller keeps the raw `image` as the base so a later Write applies edits onto it.
    static func load(image: [UInt8]) throws
        -> (channels: [FT60Channel], pms: [FT60PmsPair], settings: [FT60Setting]) {
        var err: UnsafeMutablePointer<CChar>?
        let handle = image.withUnsafeBufferPointer { base in
            platypus_ft60_apply_pms(base.baseAddress, base.count, nil, 0, &err)
        }
        guard let handle else {
            let msg = err.map { String(cString: $0) } ?? "not a valid FT-60 image"
            if let err { platypus_string_free(err) }
            throw ReadError(message: msg)
        }
        defer { platypus_ft60_free(handle) }
        return (decodeChannels(handle), decodePms(handle), decodeSettings(handle))
    }

    // Decode helpers — shared by `read` (from the radio) and `load` (from a backup image).
    private static func imageBytes(_ handle: OpaquePointer?) -> [UInt8] {
        var len = 0
        let ptr = platypus_ft60_image_bytes(handle, &len)
        return (ptr != nil && len > 0) ? Array(UnsafeBufferPointer(start: ptr, count: len)) : []
    }
    private static func decodeChannels(_ handle: OpaquePointer?) -> [FT60Channel] {
        guard let cjson = platypus_ft60_memories_json(handle) else { return [] }
        defer { platypus_string_free(cjson) }
        let data = Data(String(cString: cjson).utf8)
        let dtos = (try? JSONDecoder().decode([FT60ChannelDTO].self, from: data)) ?? []
        return dtos.map { $0.channel }
    }
    private static func decodePms(_ handle: OpaquePointer?) -> [FT60PmsPair] {
        guard let pjson = platypus_ft60_pms_json(handle) else { return [] }
        defer { platypus_string_free(pjson) }
        let pdata = Data(String(cString: pjson).utf8)
        let edges = (try? JSONDecoder().decode([FT60PmsEdgeDTO].self, from: pdata)) ?? []
        return FT60PmsPair.group(edges)
    }
    private static func decodeSettings(_ handle: OpaquePointer?) -> [FT60Setting] {
        guard let sjson = platypus_ft60_settings_json(handle) else { return [] }
        defer { platypus_string_free(sjson) }
        let sdata = Data(String(cString: sjson).utf8)
        return (try? JSONDecoder().decode([FT60Setting].self, from: sdata)) ?? []
    }

    /// Write (clone-out) the edited `channels` to the radio over `port`. `baseImage` is the
    /// captured image the edits are applied onto (in core), preserving every byte we don't
    /// model. The radio must be in CLONE receive (`-WAIT-`). **Synchronous** — call off the
    /// main thread. Throws on failure/cancel.
    static func write(port: String, channels: [FT60Channel], pms: [FT60PmsPair],
                      settings: [FT60Setting], baseImage: [UInt8],
                      progress: @escaping (Double) -> Void,
                      isCancelled: @escaping () -> Bool) throws {
        // 1) Apply the channel edits onto the base image (core), yielding a new handle whose bytes
        //    carry the edits + a valid checksum.
        var cchans = channels.map(Ft60.cChannel)
        var applyErr: UnsafeMutablePointer<CChar>?
        let handle = baseImage.withUnsafeBufferPointer { base in
            cchans.withUnsafeMutableBufferPointer { cbuf in
                platypus_ft60_apply(base.baseAddress, base.count, cbuf.baseAddress, cbuf.count, &applyErr)
            }
        }
        guard let handle else {
            let msg = applyErr.map { String(cString: $0) } ?? "couldn't apply edits"
            if let applyErr { platypus_string_free(applyErr) }
            throw ReadError(message: msg)
        }
        defer { platypus_ft60_free(handle) }

        var len = 0
        let ptr = platypus_ft60_image_bytes(handle, &len)
        guard let ptr, len > 0 else { throw ReadError(message: "no image to write") }
        let withChannels = Array(UnsafeBufferPointer(start: ptr, count: len))

        // 1b) Chain the PMS band-edge edits onto that image (partial-apply, so untouched pairs
        //     stay verbatim). Reuses the same apply → bytes flow as channels.
        var cpms = pms.flatMap(Ft60.cPmsEdges)
        var pmsErr: UnsafeMutablePointer<CChar>?
        let handle2 = withChannels.withUnsafeBufferPointer { base in
            cpms.withUnsafeMutableBufferPointer { ebuf in
                platypus_ft60_apply_pms(base.baseAddress, base.count, ebuf.baseAddress, ebuf.count, &pmsErr)
            }
        }
        guard let handle2 else {
            let msg = pmsErr.map { String(cString: $0) } ?? "couldn't apply scan-edge edits"
            if let pmsErr { platypus_string_free(pmsErr) }
            throw ReadError(message: msg)
        }
        defer { platypus_ft60_free(handle2) }

        var len2 = 0
        let ptr2 = platypus_ft60_image_bytes(handle2, &len2)
        guard let ptr2, len2 > 0 else { throw ReadError(message: "no image to write") }
        let withPms = Array(UnsafeBufferPointer(start: ptr2, count: len2))

        // 1c) Chain the set-mode settings edits (values in spec order), same partial-apply pattern.
        var svals = settings.map { UInt8(truncatingIfNeeded: $0.value) }
        var setErr: UnsafeMutablePointer<CChar>?
        let handle3 = withPms.withUnsafeBufferPointer { base in
            svals.withUnsafeBufferPointer { vbuf in
                platypus_ft60_apply_settings(base.baseAddress, base.count, vbuf.baseAddress, vbuf.count, &setErr)
            }
        }
        guard let handle3 else {
            let msg = setErr.map { String(cString: $0) } ?? "couldn't apply settings edits"
            if let setErr { platypus_string_free(setErr) }
            throw ReadError(message: msg)
        }
        defer { platypus_ft60_free(handle3) }

        var len3 = 0
        let ptr3 = platypus_ft60_image_bytes(handle3, &len3)
        guard let ptr3, len3 > 0 else { throw ReadError(message: "no image to write") }
        let image = Array(UnsafeBufferPointer(start: ptr3, count: len3))

        // 2) Clone the resulting image out to the radio (progress + cancel).
        let box = Unmanaged.passRetained(Callbacks(progress: progress, isCancelled: isCancelled))
        defer { box.release() }
        let ctx = box.toOpaque()
        var err: UnsafeMutablePointer<CChar>?
        let ok = port.withCString { p in
            image.withUnsafeBufferPointer { buf in
                platypus_ft60_write(p, buf.baseAddress, buf.count, ctx,
                                    ft60ProgressTrampoline, ft60CancelTrampoline, &err)
            }
        }
        if ok == 0 {
            let msg = err.map { String(cString: $0) } ?? "write failed"
            if let err { platypus_string_free(err) }
            throw ReadError(message: msg)
        }
    }

    /// Pack an editor channel into the C-ABI struct `platypus_ft60_apply` consumes. Every
    /// enumerated field is already an on-radio code (from the core option lists), so this is a
    /// direct copy — no attribute mapping in the app.
    private static func cChannel(_ ch: FT60Channel) -> PlatypusFt60Channel {
        var c = PlatypusFt60Channel()
        c.slot = UInt16(ch.slot)
        c.rx_hz = ch.freqHz
        c.mode = UInt8(truncatingIfNeeded: ch.modeCode)
        // tone_mode = TMODES index (the sub-kind); tone_value = CTCSS Hz×10 or DCS code;
        // tone_value2 = the DCS code for a cross mode (CTCSS lives in tone_value).
        c.tone_mode = UInt8(truncatingIfNeeded: ch.toneModeCode)
        switch ch.tone {
        case .off: c.tone_value = 0
        case .ctcss(let f): c.tone_value = UInt16((f * 10).rounded())
        case .dcs(let code): c.tone_value = UInt16(code)
        case .cross(let f, let code):
            c.tone_value = UInt16((f * 10).rounded())
            c.tone_value2 = UInt16(code)
        }
        c.duplex = UInt8(truncatingIfNeeded: ch.duplexCode)
        c.offset_hz = UInt32(truncatingIfNeeded: ch.offsetHz)
        c.tx_hz = UInt32(truncatingIfNeeded: ch.txHz)
        c.power = UInt8(truncatingIfNeeded: ch.powerCode)
        c.step = UInt8(truncatingIfNeeded: ch.stepCode)
        c.skip = ch.skipRaw
        var mask: UInt16 = 0
        for b in ch.banks where b >= 0 && b < 16 { mask |= (UInt16(1) << UInt16(b)) }
        c.banks = mask
        // name → the fixed 8-byte field (up to 6 chars, NUL-padded by the zero-init).
        let nameBytes = Array(ch.name.utf8.prefix(6))
        withUnsafeMutableBytes(of: &c.name) { raw in
            for (i, b) in nameBytes.enumerated() { raw[i] = b }
        }
        return c
    }

    /// Pack a PMS pair into its two C-ABI edge records (lower at index 2p, upper at 2p+1). A `nil`
    /// edge writes `used=0` (clears it); both edges share the pair's `step`.
    private static func cPmsEdges(_ p: FT60PmsPair) -> [PlatypusFt60PmsEdge] {
        let step = UInt8(truncatingIfNeeded: p.step)
        return [
            PlatypusFt60PmsEdge(
                index: UInt16(p.lowerIndex), used: p.lowerHz != nil ? 1 : 0,
                freq_hz: p.lowerHz ?? 0, step: step),
            PlatypusFt60PmsEdge(
                index: UInt16(p.upperIndex), used: p.upperHz != nil ? 1 : 0,
                freq_hz: p.upperHz ?? 0, step: step),
        ]
    }
}

/// One programmed PMS band edge from `platypus_ft60_pms_json`.
struct FT60PmsEdgeDTO: Codable {
    let index: Int
    let freqHz: UInt64
    let step: Int
}

/// One set-mode setting from `platypus_ft60_settings_json` — a labeled pick-list; `value` indexes
/// `options`. Order is the core's `settings_specs` order (the write sends values back in order).
struct FT60Setting: Codable, Identifiable {
    let key: String
    let label: String
    var value: Int
    let options: [String]
    var id: String { key }
    var valueLabel: String { options.indices.contains(value) ? options[value] : "\(value)" }
}

/// A PMS scan-limit pair (lower/upper band edge). Interleaved on the radio: record `2p` = lower,
/// `2p+1` = upper (confirmed on hardware). `nil` edge = unset; `step` is shared by both edges.
struct FT60PmsPair: Identifiable {
    let pair: Int  // 0-based; UI shows pair+1 as L01/U01…
    var lowerHz: UInt64?
    var upperHz: UInt64?
    var step: Int = 0
    var id: Int { pair }
    var label: String { String(format: "PMS %02d", pair + 1) }
    var lowerIndex: Int { pair * 2 }
    var upperIndex: Int { pair * 2 + 1 }

    /// Group interleaved edges (index 2p = lower, 2p+1 = upper) into pairs, sorted.
    static func group(_ edges: [FT60PmsEdgeDTO]) -> [FT60PmsPair] {
        var byPair: [Int: FT60PmsPair] = [:]
        for e in edges {
            let p = e.index / 2
            var pair = byPair[p] ?? FT60PmsPair(pair: p)
            if e.index % 2 == 0 { pair.lowerHz = e.freqHz } else { pair.upperHz = e.freqHz }
            pair.step = e.step
            byPair[p] = pair
        }
        return byPair.values.sorted { $0.pair < $1.pair }
    }
}

/// The FFI channel JSON shape (see `platypus_ft60_memories_json`), mapped into the editor
/// model. Every enumerated field arrives as its on-radio code — the app stores it verbatim,
/// resolving labels through the core option lists only for display.
private struct FT60ChannelDTO: Codable {
    let slot: Int
    let name: String
    let freqHz: UInt64
    let modeCode: Int  // MODES index
    let toneModeCode: Int  // TMODES index (sub-kind)
    let toneMode: String  // value kind: "off"/"ctcss"/"dcs"/"cross"
    let toneValue: Int
    let toneValue2: Int  // DCS code for a cross mode (CTCSS in toneValue); 0 otherwise
    let duplexCode: Int  // 0 simplex, 2 −, 3 +, 4 split
    let offsetHz: UInt64
    let txHz: UInt64
    let power: Int  // POWER_LEVELS index
    let step: Int  // STEPS index
    let skip: Int  // 0=none, 1=Skip, 2=Preferred
    let banks: [Int]

    var channel: FT60Channel {
        let tone: FTTone
        switch toneMode {
        case "ctcss": tone = .ctcss(Double(toneValue) / 10.0)
        case "dcs": tone = .dcs(toneValue)
        case "cross": tone = .cross(ctcss: Double(toneValue) / 10.0, dcs: toneValue2)
        default: tone = .off
        }
        return FT60Channel(
            slot: slot, name: name, freqHz: freqHz, modeCode: modeCode,
            toneModeCode: toneModeCode, tone: tone,
            banks: Set(banks), skip: skip != 0, skipRaw: UInt8(clamping: skip),
            powerCode: power, duplexCode: duplexCode, offsetHz: offsetHz, txHz: txHz,
            stepCode: step, serviceType: nil)
    }
}

// Capture-free trampolines → C function pointers.
private func ft60ProgressTrampoline(
    _ ctx: UnsafeMutableRawPointer?, _ phase: UInt32, _ done: UInt32, _ total: UInt32
) {
    guard let ctx else { return }
    let cb = Unmanaged<Ft60.Callbacks>.fromOpaque(ctx).takeUnretainedValue()
    cb.progress(total > 0 ? Double(done) / Double(total) : 0)
}
private func ft60CancelTrampoline(_ ctx: UnsafeMutableRawPointer?) -> UInt8 {
    guard let ctx else { return 0 }
    let cb = Unmanaged<Ft60.Callbacks>.fromOpaque(ctx).takeUnretainedValue()
    return cb.isCancelled() ? 1 : 0
}
