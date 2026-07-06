// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import AppKit
import SwiftUI

// The multi-source browse model (shell). Platypus merges several data sources into one
// browse/map view; each record carries provenance. Milestone: HPDB is the one real source;
// RepeaterBook + RadioReference are stubs (toggle + credential capture) until their network
// backends land. "Data source" (read/browse) is a separate axis from the "target radio"
// (write) picked by the radio switcher.

/// A browsable data source kind.
enum DataSourceKind: String, CaseIterable, Identifiable {
    case hpdb
    case repeaterBook
    case radioReference

    var id: String { rawValue }

    var name: String {
        switch self {
        case .hpdb: return "Sentinel HPDB"
        case .repeaterBook: return "RepeaterBook"
        case .radioReference: return "RadioReference"
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

    /// Short provenance badge label.
    var badge: String {
        switch self {
        case .hpdb: return "HPDB"
        case .repeaterBook: return "RB"
        case .radioReference: return "RR"
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

    /// The bundled logo image, if present in the app bundle.
    var logo: NSImage? {
        guard let name = logoName,
              let url = Bundle.main.url(forResource: name, withExtension: "png")
        else { return nil }
        return NSImage(contentsOf: url)
    }
}

/// One configured source: whether it's enabled and any captured credentials (shell only —
/// not yet stored in Keychain or validated against the network).
struct DataSource: Identifiable {
    let kind: DataSourceKind
    /// Whether the user has added this source. Nothing is added by default: HPDB is added by
    /// picking a folder of `s_*.hpd` files; external sources by capturing credentials.
    var added: Bool
    var enabled: Bool
    var token: String?
    var email: String?
    var appKey: String?
    /// For HPDB: the picked folder of `s_*.hpd` files that feeds the map (its "credential").
    var folderPath: String?

    var id: String { kind.id }

    /// True once the source has whatever it needs — a folder (HPDB) or a credential (external).
    var configured: Bool {
        switch kind {
        case .hpdb: return folderPath != nil
        case .repeaterBook: return !(token ?? "").isEmpty
        case .radioReference: return !(appKey ?? "").isEmpty
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
    }

    private func index(_ kind: DataSourceKind) -> Int? {
        sources.firstIndex { $0.kind == kind }
    }

    func source(_ kind: DataSourceKind) -> DataSource? {
        index(kind).map { sources[$0] }
    }

    /// Enable/disable a source (only meaningful once it's configured).
    func toggle(_ kind: DataSourceKind) {
        guard let i = index(kind) else { return }
        sources[i].enabled.toggle()
    }

    /// Add (or re-point) the HPDB source to a picked folder of `s_*.hpd` files — added + on.
    func addHpdb(folderPath: String) {
        guard let i = index(.hpdb) else { return }
        sources[i].folderPath = folderPath
        sources[i].added = true
        sources[i].enabled = true
    }

    /// Enable/disable the HPDB map source without dropping its picked folder.
    func setHpdb(enabled: Bool) {
        guard let i = index(.hpdb) else { return }
        sources[i].enabled = enabled
    }

    /// Store captured credentials and mark the source added + enabled (shell — no validation yet).
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

    /// Enabled + configured sources — the ones that actually merge into browse.
    var activeKinds: [DataSourceKind] { sources.filter { $0.enabled && $0.configured }.map(\.kind) }
    var activeCount: Int { activeKinds.count }

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

/// Captures the credentials an external source needs (shell — stored in the in-memory store,
/// not yet Keychain, and not validated against the network).
struct AddSourceSheet: View {
    let kind: DataSourceKind
    /// (token, email, appKey) — only the fields relevant to `kind` are non-nil.
    let onAdd: (String?, String?, String?) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var token = ""
    @State private var email = ""
    @State private var appKey = ""

    private var ready: Bool {
        switch kind {
        case .repeaterBook: return !token.trimmingCharacters(in: .whitespaces).isEmpty
        case .radioReference: return !appKey.trimmingCharacters(in: .whitespaces).isEmpty
        case .hpdb: return true
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
                    field("App key", "your RadioReference app key", $appKey)
                case .hpdb:
                    Text("Open a card or backup folder from the Sources menu.")
                        .font(.system(size: 11)).foregroundStyle(Theme.fg3)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Text("Stored locally for this session — Keychain + live validation land with the network backend.")
                .font(.system(size: 9.5)).foregroundStyle(Theme.fg3)
                .multilineTextAlignment(.center).frame(maxWidth: .infinity)

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large)
                Button("Add") {
                    onAdd(token.isEmpty ? nil : token, email.isEmpty ? nil : email,
                          appKey.isEmpty ? nil : appKey)
                    dismiss()
                }
                .buttonStyle(.borderedProminent).controlSize(.large).disabled(!ready)
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
}
