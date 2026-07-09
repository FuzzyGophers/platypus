// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import AppKit
import SwiftUI

// The multi-source browse model. Platypus merges several data sources into one browse/map view;
// each record carries provenance. A source is either local (a loaded card/folder) or networked
// (credential + live fetch); a source whose backend hasn't landed yet is a toggle + credential shell.
// "Data source" (read/browse) is a separate axis from the "target radio" (write) picked by the
// radio switcher.

/// A browsable data source kind.
enum DataSourceKind: String, CaseIterable, Identifiable {
    case hpdb
    case repeaterBook
    case radioReference

    var id: String { rawValue }

    var name: String {
        switch self {
        case .hpdb: return "Uniden"
        case .repeaterBook: return "Repeater Book"
        case .radioReference: return "Radio Reference"
        }
    }

    /// One-word origin note shown under the name.
    var detail: String {
        switch self {
        case .hpdb: return "card · offline"
        case .repeaterBook: return "ham repeaters"
        case .radioReference: return "public safety"
        }
    }

    /// Provenance label — spelled out (no abbreviations), matching how we refer to each source.
    var badge: String {
        switch self {
        case .hpdb: return "Uniden"
        case .repeaterBook: return "Repeater Book"
        case .radioReference: return "Radio Reference"
        }
    }

    var tint: Color {
        switch self {
        case .hpdb: return Theme.accent
        case .repeaterBook: return Color(hex: 0x34c759)
        case .radioReference: return Color(hex: 0xff9f0a)
        }
    }

    /// External sources need a token/key before they can fetch.
    var needsCredential: Bool { self != .hpdb }

    /// Whether this source is browsed by **querying a location** (fetch systems for a place) rather
    /// than loading a static dataset. HPDB is static (a loaded card/folder); RadioReference and
    /// RepeaterBook are location-queryable — they share the location-chip browse UI.
    var isLocationQueryable: Bool {
        switch self {
        case .hpdb: return false
        case .repeaterBook, .radioReference: return true
        }
    }

    /// Label for the missing-credential state ("needs token" / "needs key").
    var credentialNoun: String { self == .repeaterBook ? "token" : "key" }

    /// Attribution line shown when adding / crediting the source.
    var credit: String {
        switch self {
        case .hpdb: return "Database from Uniden Sentinel."
        case .repeaterBook: return "Repeater data courtesy of RepeaterBook.com."
        case .radioReference: return "Data provided by RadioReference.com."
        }
    }

    /// The source's public website.
    var website: URL {
        switch self {
        case .hpdb: return URL(string: "https://www.uniden.com")!
        case .repeaterBook: return URL(string: "https://www.repeaterbook.com")!
        case .radioReference: return URL(string: "https://www.radioreference.com")!
        }
    }

    var websiteLabel: String {
        switch self {
        case .hpdb: return "uniden.com"
        case .repeaterBook: return "repeaterbook.com"
        case .radioReference: return "radioreference.com"
        }
    }

    /// Where to obtain the credential this source needs.
    var credentialURL: URL? {
        switch self {
        case .hpdb: return nil
        case .repeaterBook: return URL(string: "https://www.repeaterbook.com/api/token_request.php")
        case .radioReference: return URL(string: "https://www.radioreference.com/account/api")
        }
    }

    var credentialLinkText: String {
        switch self {
        case .repeaterBook: return "Get an app token"
        case .radioReference: return "Request an app key"
        case .hpdb: return ""
        }
    }

    /// Bundled logo asset name (in the app's Resources), if any. These are the services'
    /// own icons, shown to identify/credit each source.
    var logoName: String? {
        switch self {
        case .hpdb: return nil
        case .repeaterBook: return "repeaterbook"
        case .radioReference: return "radioreference"
        }
    }

    /// The bundled logo image, if present. Loaded from the SwiftPM resource bundle (`Bundle.module`)
    /// so it resolves under `swift run` and the packaged app alike.
    var logo: NSImage? {
        guard let name = logoName,
              let url = Bundle.module.url(forResource: name, withExtension: "png")
        else { return nil }
        return NSImage(contentsOf: url)
    }
}

/// One configured source: whether it's enabled and any captured credentials. A networked source's
/// login is validated live and stored in the Keychain; a not-yet-implemented source's config is held
/// in-memory (shell) until its backend lands.
struct DataSource: Identifiable {
    let kind: DataSourceKind
    /// Whether the user has added this source. Nothing is added by default: HPDB is added by
    /// picking a folder of `s_*.hpd` files; external sources by capturing credentials.
    var added: Bool
    var enabled: Bool
    var token: String?
    var email: String?
    var appKey: String?
    /// RadioReference: the signed-in Premium username (the password lives in the Keychain).
    var username: String?
    /// For HPDB: the picked folder of `s_*.hpd` files that feeds the map (its "credential").
    var folderPath: String?

    var id: String { kind.id }

    /// True once the source has whatever it needs — a folder (HPDB) or a credential (external).
    var configured: Bool {
        switch kind {
        case .hpdb: return folderPath != nil
        case .repeaterBook: return !(token ?? "").isEmpty
        case .radioReference: return !(username ?? "").isEmpty
        }
    }

    /// The status line shown in the dropdown.
    var statusText: String {
        if kind == .hpdb {
            guard let folderPath else { return "no folder" }
            // Show the folder plus its parent (e.g. "MyCard · HPDB") — the leaf is usually "HPDB".
            let leaf = (folderPath as NSString).lastPathComponent
            let parent = ((folderPath as NSString).deletingLastPathComponent as NSString).lastPathComponent
            return parent.isEmpty ? leaf : "\(parent) · \(leaf)"
        }
        if kind == .radioReference {
            guard let username, !username.isEmpty else { return "needs sign-in" }
            return "signed in · \(username)"
        }
        if !configured { return "needs \(kind.credentialNoun)" }
        return "ready · stub"  // configured but no live fetch yet
    }
}

/// The set of configured sources + the enabled subset that merges into browse.
final class DataSourceStore: ObservableObject {
    @Published var sources: [DataSource]

    init() {
        // Nothing is added by default. HPDB is added by picking a folder (map data, separate
        // from the target radio); the external sources by capturing credentials via the Add sheet.
        sources = [
            DataSource(kind: .hpdb, added: false, enabled: false),
            DataSource(kind: .repeaterBook, added: false, enabled: false),
            DataSource(kind: .radioReference, added: false, enabled: false),
        ]
        // Restore a previously signed-in RadioReference login from the Keychain (username only —
        // the password stays in the Keychain and is read at fetch time).
        if let creds = RadioReferenceKeychain.load(), let i = index(.radioReference) {
            sources[i].username = creds.username
            sources[i].added = true
        }
    }

    private func index(_ kind: DataSourceKind) -> Int? {
        sources.firstIndex { $0.kind == kind }
    }

    func source(_ kind: DataSourceKind) -> DataSource? {
        index(kind).map { sources[$0] }
    }

    /// Toggle a configured source on/off in the merge (multi-active — several sources can be on at
    /// once; they merge into one location-first list + map). No-op if not configured.
    func toggleEnabled(_ kind: DataSourceKind) {
        guard let i = index(kind), sources[i].configured else { return }
        sources[i].enabled.toggle()
    }

    /// Enable a configured source (idempotent).
    func setEnabled(_ kind: DataSourceKind, _ on: Bool) {
        guard let i = index(kind), sources[i].configured else { return }
        sources[i].enabled = on
    }

    /// Add (or re-point) the HPDB source to a picked folder of `s_*.hpd` files, and enable it.
    func addHpdb(folderPath: String) {
        guard let i = index(.hpdb) else { return }
        sources[i].folderPath = folderPath
        sources[i].added = true
        sources[i].enabled = true
    }

    /// Forget the HPDB source (e.g. the card was ejected and no map folder remains).
    func signOutHpdb() {
        guard let i = index(.hpdb) else { return }
        sources[i].folderPath = nil
        sources[i].added = false
        sources[i].enabled = false
    }

    /// Sign in to RadioReference: persist the login to the Keychain and enable the source.
    /// Credentials are assumed already validated by the Add sheet.
    func signInRadioReference(username: String, password: String) {
        RadioReferenceKeychain.save(RadioReferenceCredentials(username: username, password: password))
        guard let i = index(.radioReference) else { return }
        sources[i].username = username
        sources[i].added = true
        sources[i].enabled = true
    }

    /// Sign out of RadioReference: clear the Keychain and forget the source.
    func signOutRadioReference() {
        RadioReferenceKeychain.clear()
        guard let i = index(.radioReference) else { return }
        sources[i].username = nil
        sources[i].added = false
        sources[i].enabled = false
    }

    /// Store captured RepeaterBook credentials (shell — no validation yet).
    func configure(_ kind: DataSourceKind, token: String? = nil, email: String? = nil,
                   appKey: String? = nil)
    {
        guard let i = index(kind) else { return }
        if let token { sources[i].token = token }
        if let email { sources[i].email = email }
        if let appKey { sources[i].appKey = appKey }
        if sources[i].configured {
            sources[i].added = true
            sources[i].enabled = true
        }
    }

    /// Enabled + configured sources — the merge set (may be several at once).
    var activeKinds: [DataSourceKind] { sources.filter { $0.enabled && $0.configured }.map(\.kind) }
    var activeCount: Int { activeKinds.count }
    /// Whether a given kind is currently enabled + configured.
    func isActive(_ kind: DataSourceKind) -> Bool {
        source(kind).map { $0.enabled && $0.configured } ?? false
    }

    /// The added sources, alphabetized by name — drives the dropdown list.
    var addedSources: [DataSource] {
        sources.filter(\.added)
            .sorted { $0.kind.name.localizedCaseInsensitiveCompare($1.kind.name) == .orderedAscending }
    }

    /// Kinds not yet added — drives the "Add a source…" submenu (HPDB adds by folder, the
    /// external sources by credential).
    var addableKinds: [DataSourceKind] {
        sources.filter { !$0.added }
            .map(\.kind)
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }
}

/// Captures the credentials an external source needs. A source with a live backend validates them
/// against the network and stores the login in the Keychain; a shell source just holds its config
/// in-memory until its backend lands.
struct AddSourceSheet: View {
    let kind: DataSourceKind
    /// The relevant fields for `kind` are non-nil: RepeaterBook → token/email; RadioReference →
    /// username/password (validated live before this fires).
    let onAdd: (_ token: String?, _ email: String?, _ appKey: String?, _ username: String?, _ password: String?) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var token = ""
    @State private var email = ""
    @State private var appKey = ""
    @State private var username = ""
    @State private var password = ""
    @State private var validating = false
    @State private var errorMessage: String?

    private var ready: Bool {
        switch kind {
        case .repeaterBook: return !token.trimmingCharacters(in: .whitespaces).isEmpty
        case .radioReference:
            return RadioReferenceKey.isConfigured
                && !username.trimmingCharacters(in: .whitespaces).isEmpty
                && !password.isEmpty
        case .hpdb: return true
        }
    }

    /// Validate RadioReference credentials off the main thread, then sign in on success.
    private func addRadioReference() {
        guard let appKey = RadioReferenceKey.current else {
            errorMessage = "This build has no Radio Reference app key."
            return
        }
        let user = username.trimmingCharacters(in: .whitespaces)
        let pass = password
        validating = true
        errorMessage = nil
        DispatchQueue.global(qos: .userInitiated).async {
            let result = RadioReferenceSource.validate(appKey: appKey, username: user, password: pass)
            DispatchQueue.main.async {
                validating = false
                switch result {
                case .success:
                    onAdd(nil, nil, nil, user, pass)
                    dismiss()
                case .failure(let err):
                    errorMessage = err.message
                }
            }
        }
    }

    var body: some View {
        VStack(spacing: 14) {
            // Centered header — logo-ready mark + name + origin
            VStack(spacing: 7) {
                sourceMark(kind)
                Text("Add \(kind.name)").font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.fg)
                Text(kind.detail).font(.system(size: 11)).foregroundStyle(Theme.fg3)
            }
            .frame(maxWidth: .infinity)

            // Centered credit + website / credential links
            VStack(spacing: 6) {
                Text(kind.credit).font(.system(size: 11)).foregroundStyle(Theme.fg2)
                    .multilineTextAlignment(.center)
                HStack(spacing: 16) {
                    Link(destination: kind.website) {
                        Label(kind.websiteLabel, systemImage: "safari").font(.system(size: 11))
                    }
                    if let cred = kind.credentialURL {
                        Link(destination: cred) {
                            Label(kind.credentialLinkText, systemImage: "key.fill").font(.system(size: 11))
                        }
                    }
                }
                .buttonStyle(.plain).foregroundStyle(Theme.accent)
            }
            .frame(maxWidth: .infinity)
            .padding(10)
            .background(Theme.bg2).clipShape(RoundedRectangle(cornerRadius: Theme.rField))
            .overlay(RoundedRectangle(cornerRadius: Theme.rField).stroke(Theme.border))

            // Credential fields (labels read better left-aligned)
            VStack(alignment: .leading, spacing: 10) {
                switch kind {
                case .repeaterBook:
                    field("App token", "rbuapp_…", $token)
                    field("Contact email", "you@example.org", $email)
                    Text("Both are sent as the required User-Agent on every request.")
                        .font(.system(size: 10)).foregroundStyle(Theme.fg3)
                case .radioReference:
                    field("Username", "your Radio Reference username", $username)
                    secureField("Password", "your Radio Reference password", $password)
                    if !RadioReferenceKey.isConfigured {
                        Text("This build has no Radio Reference app key — set RR_APP_KEY (dev) or bundle it.")
                            .font(.system(size: 10)).foregroundStyle(Color(hex: 0xff9f0a))
                    } else {
                        Text("Requires a Radio Reference Premium subscription.")
                            .font(.system(size: 10)).foregroundStyle(Theme.fg3)
                    }
                case .hpdb:
                    Text("Open a card or backup folder from the Sources menu.")
                        .font(.system(size: 11)).foregroundStyle(Theme.fg3)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if let errorMessage {
                Text(errorMessage)
                    .font(.system(size: 10.5)).foregroundStyle(Color(hex: 0xff453a))
                    .multilineTextAlignment(.center).frame(maxWidth: .infinity)
            } else if kind == .radioReference {
                Text("Your login is stored in the macOS Keychain. Fetches run over TLS.")
                    .font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
                    .multilineTextAlignment(.center).frame(maxWidth: .infinity)
            } else {
                Text("Stored locally for this session — Keychain + live validation land with the network backend.")
                    .font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
                    .multilineTextAlignment(.center).frame(maxWidth: .infinity)
            }

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large).disabled(validating)
                Button(kind == .radioReference ? "Sign in" : "Add") {
                    if kind == .radioReference {
                        addRadioReference()
                    } else {
                        onAdd(token.isEmpty ? nil : token, email.isEmpty ? nil : email,
                              appKey.isEmpty ? nil : appKey, nil, nil)
                        dismiss()
                    }
                }
                .buttonStyle(.borderedProminent).controlSize(.large).disabled(!ready || validating)
                if validating { ProgressView().controlSize(.small).padding(.leading, 2) }
                Spacer()
            }
        }
        .padding(18).frame(width: 380)
        .background(Theme.panel)
    }

    /// The source's bundled logo if present, else a tinted badge with its short label.
    @ViewBuilder
    private func sourceMark(_ kind: DataSourceKind) -> some View {
        if let logo = kind.logo {
            Image(nsImage: logo).resizable().interpolation(.high).scaledToFit()
                .frame(width: 46, height: 46)
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.border))
        } else {
            Text(kind.badge)
                .font(.system(size: 13, weight: .heavy)).foregroundStyle(.white)
                .frame(width: 46, height: 46)
                .background(Circle().fill(kind.tint))
                .overlay(Circle().stroke(.white.opacity(0.25), lineWidth: 1))
        }
    }

    /// A themed labeled text field (matches the app's dark chrome, not a grey grouped form).
    private func field(_ label: String, _ placeholder: String, _ text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.system(size: 10.5, weight: .medium)).foregroundStyle(Theme.fg3)
            TextField(placeholder, text: text)
                .textFieldStyle(.plain).font(.system(size: 12)).foregroundStyle(Theme.fg)
                .padding(.horizontal, 9).padding(.vertical, 7)
                .background(Theme.bg3).clipShape(RoundedRectangle(cornerRadius: Theme.rField))
                .overlay(RoundedRectangle(cornerRadius: Theme.rField).stroke(Theme.border))
        }
    }

    /// A themed labeled secure (password) field.
    private func secureField(_ label: String, _ placeholder: String, _ text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.system(size: 10.5, weight: .medium)).foregroundStyle(Theme.fg3)
            SecureField(placeholder, text: text)
                .textFieldStyle(.plain).font(.system(size: 12)).foregroundStyle(Theme.fg)
                .padding(.horizontal, 9).padding(.vertical, 7)
                .background(Theme.bg3).clipShape(RoundedRectangle(cornerRadius: Theme.rField))
                .overlay(RoundedRectangle(cornerRadius: Theme.rField).stroke(Theme.border))
        }
    }
}
