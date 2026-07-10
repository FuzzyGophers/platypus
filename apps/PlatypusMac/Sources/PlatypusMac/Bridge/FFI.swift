// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CPlatypusFFI
import Foundation

/// The one place the FFI's JSON-string convention is decoded: take a Rust-owned `char*` (from a
/// `platypus_*_json` call), decode it, and free it with `platypus_string_free`. Every Bridge type
/// funnels through these instead of repeating the guard/decode/free dance.
enum FFI {
    /// Decode a JSON **array** result; `[]` on a null pointer or a decode failure.
    static func decode<T: Decodable>(_ ptr: UnsafeMutablePointer<CChar>?) -> [T] {
        guard let ptr else { return [] }
        defer { platypus_string_free(ptr) }
        return (try? JSONDecoder().decode([T].self, from: Data(bytes: ptr, count: strlen(ptr)))) ?? []
    }

    /// Decode a JSON **object** result; `nil` on a null pointer or a decode failure.
    static func decodeOne<T: Decodable>(_ ptr: UnsafeMutablePointer<CChar>?) -> T? {
        guard let ptr else { return nil }
        defer { platypus_string_free(ptr) }
        return try? JSONDecoder().decode(T.self, from: Data(bytes: ptr, count: strlen(ptr)))
    }

    /// Take ownership of a Rust-owned `char*` as a Swift `String`, freeing it; `nil` if null.
    static func takeString(_ ptr: UnsafeMutablePointer<CChar>?) -> String? {
        guard let ptr else { return nil }
        defer { platypus_string_free(ptr) }
        return String(cString: ptr)
    }
}

/// Bridge an optional Swift `String` to a (possibly null) C string for an FFI argument — the
/// optional-arg companion to `String.withCString`.
func withOptionalCString<T>(_ s: String?, _ body: (UnsafePointer<CChar>?) -> T) -> T {
    if let s { return s.withCString(body) }
    return body(nil)
}
