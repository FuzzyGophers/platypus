// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import Foundation
import Security

/// The user's RadioReference login (their own Premium credentials — *not* the app key, which is
/// build-injected). Stored in the macOS Keychain, never in UserDefaults or on disk in the clear.
struct RadioReferenceCredentials: Codable, Equatable {
    var username: String
    var password: String
}

/// Keychain-backed storage for the RadioReference login. One generic-password item holds the
/// username + password as a small JSON blob, so both come back together.
enum RadioReferenceKeychain {
    private static let service = "com.platypus.PlatypusMac.radioreference"
    private static let account = "credentials"

    private static var baseQuery: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    /// Store (or replace) the login. Returns false if the Keychain write fails.
    @discardableResult
    static func save(_ creds: RadioReferenceCredentials) -> Bool {
        guard let data = try? JSONEncoder().encode(creds) else { return false }
        SecItemDelete(baseQuery as CFDictionary)
        var add = baseQuery
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    /// The stored login, or nil if none / unreadable.
    static func load() -> RadioReferenceCredentials? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess,
            let data = out as? Data
        else { return nil }
        return try? JSONDecoder().decode(RadioReferenceCredentials.self, from: data)
    }

    /// Remove the stored login (e.g. on sign-out).
    static func clear() {
        SecItemDelete(baseQuery as CFDictionary)
    }
}
