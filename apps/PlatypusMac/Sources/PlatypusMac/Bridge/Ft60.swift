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
    /// `progress` is 0…1; `isCancelled` is polled to abort. Returns the decoded channels plus
    /// the raw image bytes (kept so the same image can be written back). Throws on failure.
    static func read(port: String,
                     progress: @escaping (Double) -> Void,
                     isCancelled: @escaping () -> Bool) throws
        -> (channels: [FT60Channel], image: [UInt8], pms: [FT60PmsPair]) {
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

        guard let cjson = platypus_ft60_memories_json(handle) else {
            throw ReadError(message: "couldn't decode the image")
        }
        defer { platypus_string_free(cjson) }
        let data = Data(String(cString: cjson).utf8)
        let dtos = try JSONDecoder().decode([FT60ChannelDTO].self, from: data)

        // Copy the raw image bytes out (valid only until the handle is freed).
        var len = 0
        let ptr = platypus_ft60_image_bytes(handle, &len)
        let image: [UInt8] = (ptr != nil && len > 0) ? Array(UnsafeBufferPointer(start: ptr, count: len)) : []

        // PMS band-edge memories (read-only display).
        var pms: [FT60PmsPair] = []
        if let pjson = platypus_ft60_pms_json(handle) {
            defer { platypus_string_free(pjson) }
            let pdata = Data(String(cString: pjson).utf8)
            if let edges = try? JSONDecoder().decode([FT60PmsEdgeDTO].self, from: pdata) {
                pms = FT60PmsPair.group(edges)
            }
        }

        return (dtos.map { $0.channel }, image, pms)
    }

    /// Write (clone-out) the edited `channels` to the radio over `port`. `baseImage` is the
    /// captured image the edits are applied onto (in core), preserving every byte we don't
    /// model. The radio must be in CLONE receive (`-WAIT-`). **Synchronous** — call off the
    /// main thread. Throws on failure/cancel.
    static func write(port: String, channels: [FT60Channel], baseImage: [UInt8],
                      progress: @escaping (Double) -> Void,
                      isCancelled: @escaping () -> Bool) throws {
        // 1) Apply the edits onto the base image (core), yielding a new handle whose bytes
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
        let image = Array(UnsafeBufferPointer(start: ptr, count: len))

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
        // tone_mode = TMODES index (the sub-kind); tone_value = CTCSS Hz×10 or DCS code.
        c.tone_mode = UInt8(truncatingIfNeeded: ch.toneModeCode)
        switch ch.tone {
        case .off: c.tone_value = 0
        case .ctcss(let f): c.tone_value = UInt16((f * 10).rounded())
        case .dcs(let code): c.tone_value = UInt16(code)
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
}

/// One programmed PMS band edge from `platypus_ft60_pms_json`.
struct FT60PmsEdgeDTO: Codable {
    let index: Int
    let freqHz: UInt64
    let step: Int
}

/// A PMS scan-limit pair (lower/upper band edge), grouped from the interleaved edge records.
struct FT60PmsPair: Identifiable {
    let pair: Int  // 0-based; UI shows pair+1 as L01/U01…
    var lowerHz: UInt64?
    var upperHz: UInt64?
    var id: Int { pair }
    var label: String { String(format: "PMS %02d", pair + 1) }

    /// Group interleaved edges (index 2p = lower, 2p+1 = upper) into pairs, sorted.
    static func group(_ edges: [FT60PmsEdgeDTO]) -> [FT60PmsPair] {
        var byPair: [Int: FT60PmsPair] = [:]
        for e in edges {
            let p = e.index / 2
            var pair = byPair[p] ?? FT60PmsPair(pair: p)
            if e.index % 2 == 0 { pair.lowerHz = e.freqHz } else { pair.upperHz = e.freqHz }
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
    let toneMode: String  // value kind: "off"/"ctcss"/"dcs"
    let toneValue: Int
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
