// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Country→State→County name masters (`hpdb.cfg`), loadable off the main thread.
struct Masters {
    var stateNames: [UInt64: String] = [:]
    var stateAbbr: [UInt64: String] = [:]
    var stateToCountry: [UInt64: UInt64] = [:]
    var countyNames: [UInt64: String] = [:]
    var countyToState: [UInt64: UInt64] = [:]

    static func load(fromDirectory dir: String) -> Masters {
        let cfg = (dir as NSString).appendingPathComponent("hpdb.cfg")
        guard FileManager.default.fileExists(atPath: cfg) else { return Masters() }
        var m = Masters()
        let states = Hpdb.states(hpdbCfgPath: cfg)
        m.stateNames = Dictionary(states.map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        m.stateAbbr = Dictionary(states.map { ($0.id, $0.abbr) }, uniquingKeysWith: { a, _ in a })
        m.stateToCountry = Dictionary(states.map { ($0.id, $0.country) }, uniquingKeysWith: { a, _ in a })
        let counties = Hpdb.counties(hpdbCfgPath: cfg)
        m.countyNames = Dictionary(counties.map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        m.countyToState = Dictionary(
            counties.compactMap { c in c.state.map { (c.id, $0) } }, uniquingKeysWith: { a, _ in a })
        return m
    }
}

/// Severity of the card-operation feedback banner (drives icon + tint).
enum BannerLevel { case progress, success, warning, error, info }

/// A leveled message for the always-visible card-operation banner.
struct CardBanner: Equatable {
    let text: String
    let level: BannerLevel
}

/// One favorites list in the staged working set. `slot == nil` means a new list
/// (no card slot yet); `originalName == nil` marks it as new (for rename detection).
/// Edited content lives in `CatalogView.contentById[id]`; `contentDirty` marks it
/// for a slot-file write on Save.
struct WorkingList: Identifiable {
    let id: UUID
    var slot: UInt32?
    var name: String
    let originalName: String?
    var contentDirty: Bool
    /// The list's **Monitor** flag (staged); `originalMonitor` is the on-card
    /// baseline for change detection. New lists default to monitored.
    var monitor: Bool
    let originalMonitor: Bool
    /// The list's **quick key** / **number tag** (0–99, nil = "Off"), staged; the
    /// `original*` values are the on-card baseline for change detection.
    var quickKey: Int?
    let originalQuickKey: Int?
    var numberTag: Int?
    let originalNumberTag: Int?
}

/// How the "Nationwide & Interstate" bucket (the `_MultipleStates` systems that
/// don't belong to a single state) is split when drilled into.
enum NationwideGroup: CaseIterable {
    case us, canada, crossborder

    var name: String {
        switch self {
        case .us: return "United States"
        case .canada: return "Canada"
        case .crossborder: return "Cross-border"
        }
    }

    var icon: String {
        switch self {
        case .us, .canada: return "flag"
        case .crossborder: return "globe.americas"
        }
    }
}

/// Direction 1B — catalog-first, navigated as a **drill-down**: Country → State →
/// County → System → Channel. Each level is small, so it's instant and every row is
/// a full-width tap target (no hunting for a chevron). A live search collapses the
/// hierarchy into a flat result list. Filtering runs in the Rust core.
struct CatalogView: View {
    @State private var library: ScannerLibrary?
    @State private var stats: LibraryStats?
    @State private var filter = FilterState()
    @State private var rows: [CatalogSystem] = []
    @State private var expanded: Set<String> = []
    @State private var channelCache: [String: [CatalogChannel]] = [:]
    @State private var status = "Open a card or backup folder — or add a data source — to browse systems by location."
    @State private var searchDebounce: DispatchWorkItem?

    // Country → State → County masters (names) + the drill-down position.
    @State private var stateNames: [UInt64: String] = [:]
    @State private var stateAbbr: [UInt64: String] = [:]
    @State private var stateToCountry: [UInt64: UInt64] = [:]
    @State private var countyNames: [UInt64: String] = [:]
    @State private var countyToState: [UInt64: UInt64] = [:]
    @State private var selectedCountry: UInt64?
    @State private var selectedState: UInt64?
    @State private var selectedCounty: UInt64?
    /// Within the "Nationwide & Interstate" bucket (country id 0): the chosen
    /// country split (US / Canada / Cross-border).
    @State private var selectedNationwideGroup: NationwideGroup?

    @State private var libraryDir: String?
    /// A dedicated map data source (an added HPDB folder), independent of the editor `library`
    /// and the target radio — so you can map one dataset while writing to a different radio.
    @State private var mapLibrary: ScannerLibrary?
    @State private var mapDir: String?
    @State private var lens: Lens = .list
    @State private var detectedCards: [DetectedCard] = []
    @State private var showingCardPicker = false
    @State private var loading = false
    @State private var loadTitle = "Loading the database"
    @State private var loadPhase = ""
    @State private var loadFraction: Double = 0

    // The card's favorites + the list currently open in the right-hand editor.
    @State private var cardInfo: CardInfo?
    @State private var cardModified = false
    /// A backup must be taken before editing a card (reset per card session).
    @State private var cardBackedUp = false
    /// Always-visible feedback for card operations (backup / restore / delete /
    /// sort) — `status` only shows in the empty-catalog placeholder, so card ops
    /// that run while systems are on screen would otherwise give no confirmation.
    @State private var cardStatus: CardBanner?
    /// Whether the loaded source is a live connected card vs a backup/library folder.
    @State private var isLiveCard = false
    /// The loaded card's volume label (weak id; backup matching is by signature).
    @State private var cardVolumeName = ""
    /// Blocking progress-modal state, shared by long card operations (backup /
    /// restore). `opToken` non-nil shows a Cancel button.
    @State private var opActive = false
    @State private var opTitle = ""
    @State private var opIcon = "externaldrive.badge.timemachine"
    @State private var opPhase = ""
    @State private var opFraction: Double = 0
    @State private var opToken: CardBackup.CancelToken?
    @State private var opNote = "Copying and verifying every file. Don't disconnect the card."
    // Unified staged favorites model: the working set of lists, each list's edited
    // content (lazy), and the currently-open list — all committed in one batched
    // Save (see `saveAll`). `workingLists` is re-baselined from the card on load.
    @State private var workingLists: [WorkingList] = []
    @State private var contentById: [UUID: Favorites] = [:]
    @State private var selectedListId: UUID?
    @State private var selectedSystems: [FavSystem] = []
    @State private var selectedTree: [FavSystemTree] = []
    /// The channel whose settings popover is open (by tree channel id), if any.
    @State private var settingsChannelID: String?
    /// The row (system / department / channel id) whose read-only details popover is open.
    @State private var detailsID: String?
    /// The user's radios (owned set + active target), persisted. The active radio's *class*
    /// drives the UI — never a hardcoded model check. First run is neutral (no radio).
    @StateObject private var radios = RadioStore()
    /// The "Manage radios…" sheet (pick which supported radios you own).
    @State private var showingManageRadios = false
    /// The clone-image radio's in-memory data (channels + captured image). Present only while a
    /// `.cloneImage` radio is active — a *consequence* of the active class, not the definition
    /// of which radio is active (that's `radios.active`). Kept in sync by `syncActiveRadio()`.
    @State private var ft60: FT60Memory?
    /// The FT-60 editor's active bank tab (nil = All); location-first adds land here.
    @State private var ft60Bank: Int?
    /// Guided "read from radio" sheet (it self-scans serial ports, live).
    @State private var showingFt60Read = false
    /// Guided "write to radio" sheet (clone-out; gated on a captured image).
    @State private var showingFt60Write = false
    /// SDS150 display-customization editor (gear popup).
    @State private var showDisplayEditor = false
    /// Browse data sources (merge model). HPDB real; RepeaterBook/RR stubs until their
    /// network backends land. Separate axis from the target radio.
    @StateObject private var dataSources = DataSourceStore()
    /// The source kind whose Add/config sheet is open, if any.
    @State private var addingSource: DataSourceKind?
    /// Whether the per-list settings popover (Monitor / Quick Key / Number Tag) is open.
    @State private var showListSettings = false
    @State private var selectedSummary: FavoritesSummary?
    @State private var saveStatus: String?

    enum Lens: String, CaseIterable { case list = "List", map = "Map" }

    private var searching: Bool { !filter.search.isEmpty }

    /// The card volume root (parent of the model folder), derived from the opened
    /// `…/<MODEL>/HPDB` directory — for reading/managing the card's favorites lists.
    private var cardMount: String? {
        guard let dir = libraryDir else { return nil }
        return ((dir as NSString).deletingLastPathComponent as NSString).deletingLastPathComponent
    }

    /// Editing (adding/removing/renaming) is allowed once a list is open AND the
    /// card has been backed up this session.
    // Editing is in-memory staging — allowed whenever a list is open. The card is
    // only ever written by Save (`saveAll`), which is what requires a backup.
    private var canEdit: Bool { selectedListId != nil }

    // MARK: - Active-radio (class-driven — no per-model hardcoding)

    /// The active radio's device class (nil = neutral, no radio chosen).
    private var activeClass: RadioClass? { radios.active?.deviceClass }
    /// The library the map reads from: an enabled HPDB source if one's added, else the editor
    /// library (so opening an SD card still shows on the map). Independent of the target radio.
    private var mapSourceLibrary: ScannerLibrary? {
        if dataSources.source(.hpdb)?.enabled == true, let mapLibrary { return mapLibrary }
        return library
    }
    /// A clone-image (memory) radio is active — the flat editor + conventional-only browse.
    private var cloneActive: Bool { activeClass == .cloneImage }
    /// Browse is limited to conventional systems for the active radio's class.
    private var conventionalOnly: Bool { radios.active?.conventionalOnly ?? false }
    /// Where an "add" lands, by class: the clone editor's memory, or an open SD favorites list.
    /// Neutral (no radio) ⇒ nothing to add to.
    private var canAdd: Bool {
        switch activeClass {
        case .cloneImage: return ft60 != nil
        case .sdCard: return selectedListId != nil
        case nil: return false
        }
    }
    /// Human name of the current add target ("FT-60R" / "list") for button labels.
    private var addTargetName: String { cloneActive ? (radios.active?.name ?? "radio") : "list" }

    // MARK: - Staged working-set accessors

    private var selectedIndex: Int? {
        selectedListId.flatMap { id in workingLists.firstIndex { $0.id == id } }
    }
    private var selectedList: WorkingList? { selectedIndex.map { workingLists[$0] } }
    private var selectedContent: Favorites? { selectedListId.flatMap { contentById[$0] } }

    /// The card's lists at load time — the baseline pending changes are measured from.
    private var baseline: [(slot: UInt32, name: String)] {
        cardInfo?.lists.map { (slot: $0.slot, name: $0.name) } ?? []
    }

    /// A new list, a deletion, a reorder, or a rename relative to the card.
    private var structuralChanged: Bool {
        if workingLists.contains(where: { $0.slot == nil }) { return true }
        let cur = workingLists.map { (slot: $0.slot, name: $0.name) }
        let base = baseline.map { (slot: Optional($0.slot), name: $0.name) }
        if cur.count != base.count { return true }
        return zip(cur, base).contains { $0.slot != $1.slot || $0.name != $1.name }
    }
    private var contentChanged: Bool { workingLists.contains { $0.contentDirty } }
    /// A staged Monitor / quick-key / number-tag change on any existing list.
    private var settingsChanged: Bool {
        workingLists.contains {
            $0.monitor != $0.originalMonitor || $0.quickKey != $0.originalQuickKey
                || $0.numberTag != $0.originalNumberTag
        }
    }
    private var anyPending: Bool { structuralChanged || contentChanged || settingsChanged }

    /// A short human summary of what Save will write.
    private var pendingSummary: String {
        let baseSlots = Set(baseline.map { $0.slot })
        let curSlots = Set(workingLists.compactMap { $0.slot })
        let deleted = baseSlots.subtracting(curSlots).count
        let added = workingLists.filter { $0.slot == nil }.count
        let edited = workingLists.filter { $0.contentDirty && $0.slot != nil && $0.originalName != nil }.count
        let renamed = workingLists.filter { wl in
            wl.originalName != nil && wl.name != wl.originalName
        }.count
        let settingsEdited = workingLists.filter {
            $0.monitor != $0.originalMonitor || $0.quickKey != $0.originalQuickKey
                || $0.numberTag != $0.originalNumberTag
        }.count
        var parts: [String] = []
        if deleted > 0 { parts.append("\(deleted) deleted") }
        if added > 0 { parts.append("\(added) new") }
        if edited > 0 { parts.append("\(edited) edited") }
        if renamed > 0 { parts.append("\(renamed) renamed") }
        if settingsEdited > 0 { parts.append("\(settingsEdited) settings") }
        // Reorder with no other change.
        if parts.isEmpty && structuralChanged { parts.append("reordered") }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        // Keep the app-level quit guard current with the staged-edit state every render,
        // so ⌘Q / closing the window warns before discarding unsaved changes. Deterministic
        // (unlike `.onChange` on a computed value); writing to the plain guard is side-effect
        // free for SwiftUI. See QuitGuard.
        let _ = QuitGuard.shared.update(dirty: anyPending, summary: pendingSummary)
        return VStack(spacing: 0) {
            header
            Divider().overlay(Theme.border)
            if let cardStatus {
                cardStatusBar(cardStatus)
                Divider().overlay(Theme.border)
            }
            HStack(spacing: 0) {
                filterSidebar.frame(width: 220)
                Divider().overlay(Theme.border)
                Group {
                    if lens == .map {
                        if mapSourceLibrary == nil {
                            mapEmptyState
                        } else {
                            MapLensView(library: mapSourceLibrary, filter: filter, canAdd: canAdd,
                                        conventionalOnly: conventionalOnly,
                                        listName: cloneActive ? radios.active?.name : selectedList?.name) { id, lat, lon, miles in
                                addSystemRadiusToSelected(id, lat: lat, lon: lon, miles: miles)
                            }
                        }
                    } else {
                        catalog
                    }
                }
                .frame(minWidth: 380, maxWidth: .infinity)
                Divider().overlay(Theme.border)
                Group {
                    switch activeClass {
                    case .cloneImage:
                        if let ft60 {
                            FT60EditorView(
                                memory: ft60, radioName: radios.active?.name ?? "Radio",
                                symbol: radios.active?.symbol ?? "dot.radiowaves.left.and.right",
                                accent: radios.active?.accent ?? Theme.accent,
                                selectedBank: $ft60Bank,
                                onRead: { showingFt60Read = true },
                                onWrite: {
                                    if ft60.canWrite {
                                        showingFt60Write = true
                                    } else {
                                        status = "Read the radio first — Write sends back the captured image."
                                    }
                                },
                                onOpenBackup: openFt60Backup)
                        }
                    case .sdCard:
                        editorColumn
                    case nil:
                        neutralRadioColumn
                    }
                }
                .frame(width: 300)
            }
        }
        .background(Theme.bg)
        .foregroundStyle(Theme.fg)
        .preferredColorScheme(.dark)
        .frame(minWidth: 1040, minHeight: 640)
        .overlay { if loading { loadingOverlay } }
        .overlay { if opActive { operationOverlay } }
        .confirmationDialog("Choose a card", isPresented: $showingCardPicker, titleVisibility: .visible) {
            ForEach(detectedCards) { card in
                Button(card.volumeName) { openLibrary(path: card.hpdbDir, title: "Loading the card") }
            }
        }
        .sheet(isPresented: $showDisplayEditor) {
            DisplaySettingsSheet(cardMount: cardMount ?? "", isLive: isLiveCard) { edits in
                applyDisplayEdits(edits)
            }
        }
        .sheet(item: $addingSource) { kind in
            AddSourceSheet(kind: kind) { token, email, appKey in
                dataSources.configure(kind, token: token, email: email, appKey: appKey)
            }
        }
        .sheet(isPresented: $showingFt60Read) {
            FT60ReadSheet { port in readFT60(port: port) }
        }
        .sheet(isPresented: $showingFt60Write) {
            FT60WriteSheet(channelCount: ft60?.count(inBank: nil) ?? 0) { port in writeFT60(port: port) }
        }
        .sheet(isPresented: $showingManageRadios) {
            ManageRadiosSheet(radios: radios)
        }
        .onChange(of: radios.activeID) { syncActiveRadio() }
        .onAppear {
            // Reconcile the clone-image data with the restored active radio (if any).
            syncActiveRadio()
            // No card is auto-detected/opened on launch — the user opens and reads on demand from
            // the editor's action bar (Read / Open Backup). `PLATYPUS_LIBRARY` stays as an explicit
            // env override for dev/testing.
            if library == nil, let p = ProcessInfo.processInfo.environment["PLATYPUS_LIBRARY"] {
                openLibrary(path: p, title: ScannerCard.isLiveCard(hpdbDir: p) ? "Loading the card" : "Loading the database")
            }
        }
    }

    /// The always-visible card-operation feedback strip (under the header).
    private func cardStatusBar(_ banner: CardBanner) -> some View {
        let icon: String
        let tint: Color
        switch banner.level {
        case .progress: (icon, tint) = ("arrow.triangle.2.circlepath", Theme.fg2)
        case .success: (icon, tint) = ("checkmark.circle.fill", Color(hex: 0x34c759))
        case .warning: (icon, tint) = ("exclamationmark.triangle.fill", Theme.warn)
        case .error: (icon, tint) = ("xmark.octagon.fill", Theme.warn)
        case .info: (icon, tint) = ("info.circle.fill", Theme.fg2)
        }
        let dismissable = banner.level != .progress
        return HStack(spacing: 8) {
            Image(systemName: icon).font(.system(size: 11)).foregroundStyle(tint)
            Text(banner.text).font(.system(size: 11.5)).foregroundStyle(Theme.fg).lineLimit(1)
            Spacer()
            if dismissable {
                Button { cardStatus = nil } label: {
                    Image(systemName: "xmark").font(.system(size: 10)).foregroundStyle(Theme.fg3)
                }.buttonStyle(.plain).help("Dismiss")
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 7)
        .background(tint.opacity(0.12))
    }

    /// Show a banner that auto-clears after a few seconds — for confirmations that
    /// shouldn't linger. Warnings/errors are NOT routed here; they stay until the
    /// user acts or dismisses them.
    private func flashStatus(_ text: String, level: BannerLevel) {
        let banner = CardBanner(text: text, level: level)
        cardStatus = banner
        DispatchQueue.main.asyncAfter(deadline: .now() + 6) {
            if cardStatus == banner { cardStatus = nil }
        }
    }

    private var loadingOverlay: some View {
        ZStack {
            Theme.bg.opacity(0.85).ignoresSafeArea()
            VStack(spacing: 14) {
                Image(systemName: "antenna.radiowaves.left.and.right")
                    .font(.system(size: 30)).foregroundStyle(Theme.accent)
                Text(loadTitle).font(.system(size: 15, weight: .semibold))
                ProgressView(value: loadFraction).progressViewStyle(.linear).frame(width: 300)
                Text("\(loadPhase)  \(Int(loadFraction * 100))%")
                    .font(.system(size: 12)).monospacedDigit().foregroundStyle(Theme.fg2)
            }
            .padding(32)
            .background(Theme.panel).clipShape(RoundedRectangle(cornerRadius: Theme.rCard))
            .overlay(RoundedRectangle(cornerRadius: Theme.rCard).stroke(Theme.border))
        }
    }

    /// Blocking progress modal for long card operations (backup / restore). The dark
    /// fill captures clicks, so the rest of the window is inert until it finishes. A
    /// Cancel button appears only when `opToken` is set (backup — restore writes the
    /// card and must not be interrupted half-way).
    private var operationOverlay: some View {
        ZStack {
            Theme.bg.opacity(0.85).ignoresSafeArea()
            VStack(spacing: 14) {
                Image(systemName: opIcon).font(.system(size: 30)).foregroundStyle(Theme.accent)
                Text(opTitle).font(.system(size: 15, weight: .semibold))
                ProgressView(value: opFraction).progressViewStyle(.linear).frame(width: 320)
                Text("\(opPhase)  \(Int(opFraction * 100))%")
                    .font(.system(size: 12)).monospacedDigit().foregroundStyle(Theme.fg2)
                Text(opNote)
                    .font(.system(size: 11)).foregroundStyle(Theme.fg3)
                if let token = opToken {
                    Button(role: .cancel) {
                        token.cancel()
                        opPhase = "Cancelling…"
                    } label: {
                        Text("Cancel").frame(width: 130).padding(.vertical, 4)
                    }
                    .keyboardShortcut(.cancelAction)
                    .disabled(opPhase == "Cancelling…")
                    .padding(.top, 2)
                }
            }
            .padding(32)
            .background(Theme.panel).clipShape(RoundedRectangle(cornerRadius: Theme.rCard))
            .overlay(RoundedRectangle(cornerRadius: Theme.rCard).stroke(Theme.border))
        }
    }

    // MARK: - Radio switcher (manual target selection — no auto-detect)

    /// Top-left dropdown to switch the active radio. Lists only the user's owned radios
    /// ("My radios"); "Manage radios…" opens the supported-radio catalog. Header actions +
    /// right column adapt to the active radio's class. Source (browse database) opens here too.
    private var radioSwitcher: some View {
        Menu {
            Section("My radios") {
                if radios.owned.isEmpty {
                    Text("No radios yet — add one below").foregroundStyle(Theme.fg3)
                } else {
                    ForEach(radios.owned) { radio in
                        Button { radios.setActive(radio.id) } label: {
                            Label(radio.menuTitle,
                                  systemImage: radios.activeID == radio.id ? "checkmark.circle.fill" : "circle")
                        }
                    }
                }
            }
            Divider()
            Button { showingManageRadios = true } label: {
                Label("Manage radios…", systemImage: "slider.horizontal.3")
            }
        } label: {
            HStack(spacing: 5) {
                Image(systemName: radios.active?.symbol ?? "dot.radiowaves.left.and.right")
                    .foregroundStyle(radios.active?.accent ?? Theme.fg3)
                Text(radios.active?.name ?? "Add a radio").font(.system(size: 12, weight: .medium))
            }
        }
        .menuStyle(.borderlessButton).fixedSize()
        .help("Switch the active radio (manual — no auto-detect).")
    }

    /// Reconcile the clone-image data (`ft60`) with the active radio's class: a `.cloneImage`
    /// radio gets an empty memory (filled by Read/hand-add); anything else clears it. Called
    /// whenever the active radio changes (and once on appear).
    private func syncActiveRadio() {
        if activeClass == .cloneImage {
            if ft60 == nil, let cap = radios.active?.capacity {
                ft60 = FT60Memory(capacity: cap)
                ft60Bank = nil
            }
        } else if ft60 != nil {
            ft60 = nil
            ft60Bank = nil
        }
        refresh()
    }

    /// Read the real radio over `port`: a clone-in behind the operation overlay (progress +
    /// cancel), then populate the clone memory from the decoded channels. Read-only.
    private func readFT60(port: String) {
        let radioName = radios.active?.name ?? "radio"
        guard let capacity = radios.active?.capacity else { return }
        // Every Read leaves a restore point: the raw image is backed up (Rust-side, fsync'd) to
        // the radio's own folder (<backups>/<model>/) before the data is surfaced.
        let backupDir = BackupStore.modelRoot(radioName).path
        let backupStem = "\(radioName) \(Self.timestamp())"
        let token = CardBackup.CancelToken()
        opTitle = "Reading \(radioName)"
        opIcon = "antenna.radiowaves.left.and.right"
        opNote = "Hold PTT on the radio to send. Keep the cable connected."
        opPhase = "Waiting for the radio…"
        opFraction = 0
        opToken = token
        opActive = true
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                let result = try Ft60.read(
                    port: port, backupDir: backupDir, backupStem: backupStem,
                    progress: { frac in
                        DispatchQueue.main.async { opPhase = "Reading"; opFraction = frac }
                    },
                    isCancelled: { token.isCancelled })
                DispatchQueue.main.async {
                    ft60 = FT60Memory(
                        capacity: capacity, channels: result.channels, image: result.image,
                        pms: result.pms, settings: result.settings)
                    ft60Bank = nil
                    opActive = false
                    let base = "\(radioName): read \(result.channels.count) channels"
                    if let be = result.backupError {
                        status = "\(base) — ⚠︎ backup failed: \(be)"
                    } else if let bp = result.backupPath {
                        status = "\(base) · backed up to \((bp as NSString).lastPathComponent)."
                    } else {
                        status = "\(base) from the radio."
                    }
                    refresh()
                }
            } catch {
                DispatchQueue.main.async {
                    opActive = false
                    status = "\(radioName) read failed: \(error.localizedDescription)"
                }
            }
        }
    }

    /// Write the captured image back to the radio over `port`: a clone-out behind the operation
    /// overlay (progress + cancel). Writes the exact bytes just read (byte-for-byte), so a
    /// partial/failed transfer can't corrupt the radio. No cancel token surfaced mid-write —
    /// like a card write, it must not be interrupted (the sheet's confirmation is the gate).
    private func writeFT60(port: String) {
        let radioName = radios.active?.name ?? "radio"
        guard let ft60, ft60.canWrite else {
            status = "Read the radio first — Write sends back the captured image."
            return
        }
        let base = ft60.image
        let channels = ft60.channels
        let pms = ft60.pms
        let settings = ft60.settings
        let count = ft60.count(inBank: nil)
        opTitle = "Writing \(radioName)"
        opIcon = "square.and.arrow.up.on.square"
        opNote = "Keep the radio in -WAIT- and the cable connected. Don't power off."
        opPhase = "Waiting for the radio…"
        opFraction = 0
        opToken = nil  // a clone-out must not be interrupted half-way
        opActive = true
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                try Ft60.write(
                    port: port,
                    channels: channels,
                    pms: pms,
                    settings: settings,
                    baseImage: base,
                    progress: { frac in
                        DispatchQueue.main.async { opPhase = "Writing"; opFraction = frac }
                    },
                    isCancelled: { false })
                DispatchQueue.main.async {
                    opActive = false
                    status = "\(radioName): wrote \(count) channels to the radio."
                }
            } catch {
                DispatchQueue.main.async {
                    opActive = false
                    status = "\(radioName) write failed: \(error.localizedDescription)"
                }
            }
        }
    }

    /// Open a saved FT-60 backup **into the editor** (editable), defaulting the picker to this
    /// radio's backups folder. The raw image becomes the base memory; the decoded channels/PMS/
    /// settings are editable, and Write clones the (possibly edited) image out.
    private func openFt60Backup() {
        let radioName = radios.active?.name ?? "radio"
        guard let capacity = radios.active?.capacity else { return }
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [UTType(filenameExtension: "img") ?? .data]
        panel.directoryURL = BackupStore.modelRoot(radioName)
        panel.prompt = "Open Backup"
        panel.message = "Open an \(radioName) backup image to edit, then Write."
        guard panel.runModal() == .OK, let url = panel.url, let data = try? Data(contentsOf: url)
        else { return }
        let bytes = [UInt8](data)
        do {
            let loaded = try Ft60.load(image: bytes)
            ft60 = FT60Memory(
                capacity: capacity, channels: loaded.channels, image: bytes,
                pms: loaded.pms, settings: loaded.settings)
            ft60Bank = nil
            status = "\(radioName): opened backup \(url.lastPathComponent) — edit, then Write."
            refresh()
        } catch {
            status = "Couldn't open backup: \(error.localizedDescription)"
        }
    }

    // MARK: - Data sources (browse) — merge model, separate from the target radio

    /// Top dropdown listing the browse data sources. Enabled+configured sources merge into
    /// the catalog/map. External sources capture credentials via the Add sheet first.
    private var sourcesSwitcher: some View {
        Menu {
            Section("Data sources") {
                ForEach(dataSources.addedSources) { src in
                    Button { toggleSource(src.kind) } label: {
                        Label("\(src.kind.name) — \(src.statusText)",
                              systemImage: src.enabled ? "checkmark.circle.fill" : "circle")
                    }
                }
            }
            if dataSources.source(.hpdb)?.added == true {
                Divider()
                Button("Change HPDB folder…") { openMapSource() }
            }
            if !dataSources.addableKinds.isEmpty {
                Divider()
                Menu("Add a source…") {
                    ForEach(dataSources.addableKinds) { kind in
                        Button(kind.name) { kind == .hpdb ? openMapSource() : (addingSource = kind) }
                    }
                }
            }
            Button("Manage sources…") {}.disabled(true)
        } label: {
            HStack(spacing: 5) {
                Image(systemName: "square.stack.3d.up")
                Text("Sources").font(.system(size: 12, weight: .medium))
            }
        }
        .menuStyle(.borderlessButton).fixedSize()
        .help("Data sources that merge into browse (separate from the target radio).")
    }

    /// Shown in the map lens when no data source is loaded — a discoverable way to open an HPDB
    /// folder (mirrors the editor action bars).
    private var mapEmptyState: some View {
        VStack(spacing: 12) {
            Image(systemName: "map").font(.system(size: 34)).foregroundStyle(Theme.fg3)
            Text("No map source").font(.system(size: 14, weight: .semibold)).foregroundStyle(Theme.fg)
            Text("Open a Sentinel HPDB folder to plot systems by location.")
                .font(.system(size: 11)).foregroundStyle(Theme.fg3)
                .multilineTextAlignment(.center)
            Button { openMapSource() } label: {
                Label("Open an HPDB folder", systemImage: "folder")
            }.controlSize(.large)
        }
        .padding(24).frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.bg)
    }

    /// Toggle a source on/off. HPDB drives the map handle (falls back to the editor library when
    /// off); the external stubs just flip their enabled flag.
    private func toggleSource(_ kind: DataSourceKind) {
        if kind == .hpdb {
            let on = dataSources.source(.hpdb)?.enabled ?? false
            dataSources.setHpdb(enabled: !on)
        } else {
            dataSources.toggle(kind)
        }
    }

    /// A small provenance chip (tinted dot + short label), reusable in details footers/rows.
    private func sourceBadge(_ kind: DataSourceKind) -> some View {
        HStack(spacing: 3) {
            Circle().fill(kind.tint).frame(width: 6, height: 6)
            Text(kind.badge).font(.system(size: 9, weight: .bold)).foregroundStyle(Theme.fg2)
        }
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 12) {
            Text("Platypus").font(.system(size: 16, weight: .semibold))
            radioSwitcher
            sourcesSwitcher
            Picker("", selection: $lens) {
                ForEach(Lens.allCases, id: \.self) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented).fixedSize().disabled(mapSourceLibrary == nil)
            Spacer()
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").foregroundStyle(Theme.fg3)
                TextField("Search systems, talkgroups…", text: $filter.search)
                    .textFieldStyle(.plain).frame(width: 280)
                    .onChange(of: filter.search) { debouncedRefresh() }
                if searching {
                    Button { filter.search = ""; refresh() } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.fg3)
                    }.buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background(Theme.bg3).clipShape(RoundedRectangle(cornerRadius: Theme.rField))
            .overlay(RoundedRectangle(cornerRadius: Theme.rField).stroke(Theme.border))
            Spacer()
            if cloneActive, let radio = radios.active {
                let n = ft60?.channels.count ?? 0
                Label("\(radio.name) · \(radio.maker) · \(radio.transport)"
                    + (n > 0 ? " · \(n) ch" : ""), systemImage: radio.symbol)
                    .font(.system(size: 10.5)).foregroundStyle(Theme.fg3)
                    .help("Read/Write the radio over the serial clone cable from the editor.")
            } else if let stats {
                Text("\(stats.systems) systems · \(stats.channels) channels")
                    .font(.system(size: 11)).foregroundStyle(Theme.fg2)
            }
        }
        .padding(.horizontal, 14).frame(height: 46).background(Theme.titlebar)
    }

    // MARK: - Filter sidebar

    private var filterSidebar: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                section("SERVICE TYPE") {
                    ForEach(ServiceType.filterOrderAlphabetical, id: \.self) { code in
                        let info = ServiceType.info(code)
                        Button { toggleService(code) } label: {
                            HStack(spacing: 8) {
                                Image(systemName: info.symbol).foregroundStyle(info.color).frame(width: 16)
                                Text(info.name).font(.system(size: 12)).foregroundStyle(Theme.fg)
                                Spacer()
                                if filter.services.contains(code) {
                                    Image(systemName: "checkmark").font(.system(size: 10, weight: .bold))
                                        .foregroundStyle(Theme.accent)
                                }
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                section("TECHNOLOGY") {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 64), spacing: 6)], alignment: .leading, spacing: 6) {
                        ForEach(TechFilter.allAlphabetical, id: \.self) { techPill($0) }
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(14)
        }
        .background(Theme.bg2)
    }

    // MARK: - Favorites list selector (right pane top bar)

    /// A dropdown to pick which favorites list to edit (scales to the 256-list max),
    /// plus New list and a ⋯ menu (sort all, delete this).
    private var listBar: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Menu {
                    if workingLists.isEmpty {
                        Text("No lists").disabled(true)
                    } else {
                        ForEach(workingLists) { l in
                            Button { selectList(l.id) } label: {
                                Text(listMenuLabel(l))
                            }
                        }
                    }
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "star.fill").foregroundStyle(Theme.accent).font(.system(size: 11))
                        Text(selectedList.map { $0.name.isEmpty ? "(unnamed)" : $0.name } ?? "Choose a list…")
                            .font(.system(size: 12, weight: .medium)).lineLimit(1)
                    }
                }
                .menuStyle(.borderlessButton)
                Button { newList() } label: { Image(systemName: "plus") }
                    .disabled(cardInfo == nil).help("New list")
                Menu {
                    Button("Sort all lists A→Z") { sortListsAZ() }
                        .disabled(workingLists.count < 2)
                    if let sel = selectedList {
                        Button("Delete “\(sel.name)”", role: .destructive) { deleteSelectedList() }
                    }
                } label: { Image(systemName: "ellipsis.circle") }
                .menuStyle(.borderlessButton).fixedSize()
            }
            if let info = cardInfo {
                Text("\(info.model) · \(workingLists.count) / \(info.maxFavorites) lists")
                    .font(.system(size: 10))
                    .foregroundStyle(workingLists.count >= info.maxFavorites ? Theme.warn : Theme.fg3)
            }
        }
    }

    /// Dropdown label for a working list, with a pending marker.
    private func listMenuLabel(_ l: WorkingList) -> String {
        var s = l.name.isEmpty ? "(unnamed)" : l.name
        if l.slot == nil { s += "  ·  new" } else if l.contentDirty { s += "  ·  edited" } else if let o = l.originalName, o != l.name { s += "  ·  renamed" }
        return s
    }

    private func techPill(_ t: String) -> some View {
        let on = filter.techs.contains(t)
        return Button { toggleTech(t) } label: {
            Text(t).font(.system(size: 11, weight: .semibold))
                .padding(.horizontal, 9).padding(.vertical, 5).frame(maxWidth: .infinity)
                .background(on ? Theme.accent : Theme.chip)
                .foregroundStyle(on ? .white : Theme.fg2)
                .clipShape(RoundedRectangle(cornerRadius: Theme.rField))
        }
        .buttonStyle(.plain)
    }

    // MARK: - Catalog (drill-down)

    private var catalog: some View {
        VStack(alignment: .leading, spacing: 0) {
            breadcrumb
            Divider().overlay(Theme.border)
            if rows.isEmpty {
                emptyCatalog
            } else if searching {
                systemList(systemsMatching, county: nil)
            } else if selectedCountry == 0 {
                // "Nationwide & Interstate" bucket: split by country, then systems.
                // Channels load in full (county: nil) on expand.
                if let g = selectedNationwideGroup {
                    systemList(nationwideSystems(in: g), county: nil)
                } else {
                    nationwideGroupList
                }
            } else if let county = selectedCounty {
                // county == 0 is the "Statewide" entry (no-county channels).
                let list = county == 0 ? statewideSystems(selectedState ?? 0) : systems(inCounty: county)
                systemList(list, county: county)
            } else if let state = selectedState {
                // Some states (notably the `_MultipleStates` pseudo-state that holds
                // nationwide systems) have no county breakdown — their channels are
                // geo-placed into real counties. List the systems directly so the
                // node isn't a dead end; expanding shows each system's full channels.
                if countiesLevel(state).isEmpty && statewideSystems(state).isEmpty {
                    systemList(systemsInState(state), county: nil)
                } else {
                    countyList(state)
                }
            } else if let country = selectedCountry {
                stateList(country)
            } else {
                countryList
            }
        }
    }

    private var breadcrumb: some View {
        HStack(spacing: 6) {
            crumb("All", active: selectedCountry == nil && !searching) {
                selectedCountry = nil; selectedState = nil; selectedCounty = nil
                selectedNationwideGroup = nil
            }
            if searching {
                crumbChevron; Text("Search “\(filter.search)”").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.fg)
            } else if selectedCountry == 0 {
                // Nationwide & Interstate bucket: bucket crumb, then the country split.
                crumbChevron
                crumb(Country.name(0), active: selectedNationwideGroup == nil) {
                    selectedNationwideGroup = nil
                }
                if let g = selectedNationwideGroup {
                    crumbChevron
                    Text(g.name).font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.fg)
                }
            } else {
                if let country = selectedCountry {
                    crumbChevron
                    crumb(Country.name(country), active: selectedState == nil) { selectedState = nil; selectedCounty = nil }
                }
                if let s = selectedState {
                    crumbChevron
                    crumb(stateNames[s] ?? "State \(s)", active: selectedCounty == nil) { selectedCounty = nil }
                }
                if let c = selectedCounty {
                    crumbChevron
                    Text(c == 0 ? "Statewide" : (countyNames[c] ?? "County \(c)"))
                        .font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.fg)
                }
            }
            Spacer()
            if canAdd {
                Button("Add all to \(addTargetName)", action: addAllInView)
                    .buttonStyle(.plain).font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Theme.accent).disabled(rows.isEmpty)
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 9).background(Theme.bg2)
    }

    private func crumb(_ text: String, active: Bool, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(text).font(.system(size: 11, weight: .semibold))
                .foregroundStyle(active ? Theme.fg : Theme.accent)
        }.buttonStyle(.plain)
    }

    private var crumbChevron: some View {
        Image(systemName: "chevron.right").font(.system(size: 8)).foregroundStyle(Theme.fg3)
    }

    private var emptyCatalog: some View {
        VStack(spacing: 8) {
            Image(systemName: "tray").font(.system(size: 26)).foregroundStyle(Theme.fg3)
            Text(status).font(.system(size: 12)).foregroundStyle(Theme.fg3)
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // Level 0 — countries.
    private var countryList: some View {
        List(countriesLevel, id: \.id) { c in
            drillRow(icon: "globe.americas.fill", title: c.name, trailing: nil, count: c.count) {
                selectedCountry = c.id
            }
        }
        .listStyle(.plain).scrollContentBackground(.hidden).background(Theme.bg)
    }

    // Level 1 — states/provinces in a country.
    private func stateList(_ country: UInt64) -> some View {
        List(statesLevel(country), id: \.id) { s in
            drillRow(icon: "map", title: s.name, trailing: s.abbr, count: s.count) {
                selectedState = s.id
            }
        }
        .listStyle(.plain).scrollContentBackground(.hidden).background(Theme.bg)
    }

    // The "Nationwide & Interstate" bucket split by country (US / Canada / Cross-border).
    private var nationwideGroupList: some View {
        let byGroup = Dictionary(grouping: systemsInState(0), by: nationwideGroup(of:))
        return List {
            ForEach(NationwideGroup.allCases, id: \.self) { g in
                if let items = byGroup[g], !items.isEmpty {
                    drillRow(icon: g.icon, title: g.name, trailing: nil, count: items.count) {
                        selectedNationwideGroup = g
                    }
                }
            }
        }
        .listStyle(.plain).scrollContentBackground(.hidden).background(Theme.bg)
    }

    // Level 2 — counties in a state (plus a "Statewide" entry for no-county systems).
    private func countyList(_ state: UInt64) -> some View {
        let statewide = statewideSystems(state)
        return List {
            if !statewide.isEmpty {
                drillRow(icon: "flag.fill", title: "Statewide", trailing: nil, count: statewide.count) {
                    selectedCounty = 0
                }
            }
            ForEach(countiesLevel(state), id: \.id) { c in
                drillRow(icon: "mappin.and.ellipse", title: c.name, trailing: nil, count: c.count) {
                    selectedCounty = c.id
                }
            }
        }
        .listStyle(.plain).scrollContentBackground(.hidden).background(Theme.bg)
    }

    // Level 3/4 — systems (+ inline channels), channels scoped to `county` (nil = all).
    private func systemList(_ systems: [CatalogSystem], county: UInt64?) -> some View {
        List {
            ForEach(systems) { sys in
                systemRow(sys, county: county)
                if expanded.contains(sys.id) {
                    ForEach(channelCache[cacheKey(sys, county)] ?? []) { ch in channelRow(ch, sys) }
                }
            }
        }
        .listStyle(.plain).scrollContentBackground(.hidden).background(Theme.bg)
    }

    /// A full-width navigation row (state / county). The whole row is tappable.
    private func drillRow(icon: String, title: String, trailing: String?, count: Int, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: icon).font(.system(size: 14)).foregroundStyle(Theme.fg2).frame(width: 18)
                Text(title).font(.system(size: 13, weight: .medium))
                if let trailing, !trailing.isEmpty {
                    Text(trailing).font(.system(size: 10, weight: .bold)).foregroundStyle(Theme.fg3)
                }
                Spacer()
                Text("\(count)").font(.system(size: 11)).foregroundStyle(Theme.fg3)
                Image(systemName: "chevron.right").font(.system(size: 10)).foregroundStyle(Theme.fg3)
            }
            .padding(.vertical, 8).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .listRowBackground(Theme.bg)
    }

    private func systemRow(_ sys: CatalogSystem, county: UInt64?) -> some View {
        HStack(spacing: 8) {
            // "+" adds the system's channels to the open list (when one is open).
            Button { addSystemToSelected(sys, county: county) } label: {
                Image(systemName: "plus.circle").font(.system(size: 14)).foregroundStyle(Theme.accent)
            }
            .buttonStyle(.plain).disabled(!canAdd)
            .help(cloneActive ? "Add this system's channels to the \(addTargetName)" : "Add this system's channels to the open list")
            Button { toggleExpand(sys, county: county) } label: {
                HStack(spacing: 8) {
                    Image(systemName: sys.isTrunk ? "antenna.radiowaves.left.and.right" : "radio")
                        .font(.system(size: 13)).foregroundStyle(Theme.fg2).frame(width: 16)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(sys.name).font(.system(size: 13, weight: .medium)).lineLimit(1)
                        Text(sys.isTrunk ? "\(sys.siteCount) site\(sys.siteCount == 1 ? "" : "s")" : "conventional")
                            .font(.system(size: 10.5)).foregroundStyle(Theme.fg3)
                    }
                    Spacer()
                    // Flag systems whose coverage spans many counties so it's clear
                    // they aren't county-exclusive (e.g. a statewide P25 network).
                    if county != nil && sys.multiCounty {
                        badge("WIDE")
                    }
                    if let tech = sys.tech { badge(tech) }
                    Image(systemName: expanded.contains(sys.id) ? "chevron.down" : "chevron.right")
                        .font(.system(size: 10)).foregroundStyle(Theme.fg3).frame(width: 12)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .padding(.vertical, 5)
        .listRowBackground(Theme.bg)
    }

    private func channelRow(_ ch: CatalogChannel, _ sys: CatalogSystem) -> some View {
        let info = ServiceType.info(ch.serviceType)
        return HStack(alignment: .top, spacing: 8) {
            Spacer().frame(width: 22)
            Button { addChannels([ch]) } label: {
                Image(systemName: "plus.circle").font(.system(size: 12)).foregroundStyle(Theme.accent)
            }
            .buttonStyle(.plain).disabled(!canAdd)
            .help(cloneActive ? "Add this channel to the \(addTargetName)" : "Add this channel to the open list")
            Image(systemName: info.symbol).font(.system(size: 12)).foregroundStyle(info.color)
                .frame(width: 14).padding(.top, 1)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(ch.name).font(.system(size: 12.5)).lineLimit(1)
                }
                Text(channelDetailLine(ch)).font(.system(size: 10.5)).foregroundStyle(Theme.fg3)
                    .lineLimit(1)
            }
            Spacer()
            // Primary identifier (TGID for trunked, frequency for conventional).
            Text(ch.detail).font(.system(size: 11.5, weight: .medium).monospacedDigit())
                .foregroundStyle(Theme.fg2).frame(width: 96, alignment: .trailing)
        }
        .padding(.vertical, 4)
        .listRowBackground(Theme.bg)
    }

    /// The grey sub-line under a channel: what it is + the key parameters
    /// (kind · service type · mode · the identifier the *primary* column doesn't show).
    private func channelDetailLine(_ ch: CatalogChannel) -> String {
        var parts: [String] = [ch.isTalkgroup ? "Talkgroup" : "Conventional"]
        parts.append(ServiceType.info(ch.serviceType).name)
        if let mode = ch.mode { parts.append(mode) }
        if let tone = ch.toneInline { parts.append(tone) }
        // Show the secondary identifier too (the column shows only one).
        if ch.tgid != nil, let f = ch.freqMHz { parts.append(f) }
        if let tg = ch.tgid, ch.freqHz != nil { parts.append("TG \(tg)") }
        return parts.joined(separator: "  ·  ")
    }

    // MARK: - Right column (neutral): no radio chosen yet

    /// Shown when no radio is active (first run / after removing the active radio). Browse still
    /// works; this guides the user to pick their radio. Adds are disabled until one is chosen.
    private var neutralRadioColumn: some View {
        VStack(spacing: 12) {
            Spacer()
            Image(systemName: "dot.radiowaves.left.and.right")
                .font(.system(size: 30)).foregroundStyle(Theme.fg3)
            Text("Choose your radio to begin")
                .font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.fg2)
            Button { showingManageRadios = true } label: {
                Label("Add a radio…", systemImage: "plus.circle")
            }.controlSize(.small).padding(.top, 2)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Right column = list selector (dropdown) + the open list editor

    private var editorColumn: some View {
        VStack(alignment: .leading, spacing: 0) {
            sdCardHeaderBar
            Divider().overlay(Theme.border)
            // Always-visible list selector so you can switch lists without going back.
            listBar.padding(14)
            Divider().overlay(Theme.border)

            if cardMount == nil {
                VStack(spacing: 8) {
                    Image(systemName: "sdcard").font(.system(size: 24)).foregroundStyle(Theme.fg3)
                    Text("Open a card or backup folder to view and edit its favorites.")
                        .font(.system(size: 12)).foregroundStyle(Theme.fg3).multilineTextAlignment(.center)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity).padding(.horizontal, 16)
            } else {
                if selectedListId != nil {
                    renameRow.padding(.horizontal, 14).padding(.top, 10)
                    editorGauge.padding(.horizontal, 14).padding(.top, 8)
                    Divider().overlay(Theme.border).padding(.vertical, 10)
                    if selectedSystems.isEmpty {
                        Text("Empty list — browse in the middle and **＋** channels into it.")
                            .font(.system(size: 11)).foregroundStyle(Theme.fg3).multilineTextAlignment(.center)
                            .frame(maxWidth: .infinity, maxHeight: .infinity).padding(.horizontal, 16)
                    } else {
                        ScrollView { editorList.padding(.horizontal, 12) }
                    }
                } else {
                    VStack(spacing: 8) {
                        Image(systemName: "star").font(.system(size: 24)).foregroundStyle(Theme.fg3)
                        Text("Pick a list above, or **＋ New list**, to edit.")
                            .font(.system(size: 12)).foregroundStyle(Theme.fg3).multilineTextAlignment(.center)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity).padding(.horizontal, 16)
                }
                Divider().overlay(Theme.border)
                saveFooter.padding(14)
            }
        }
        .background(Theme.bg2)
    }

    /// The SD-card editor's action bar — the same Read / Write / Open-Backup shape as the FT-60
    /// header, with the card verbs (Read = mount, Write = save changes, Open Backup = a backup
    /// folder). Always visible so loading + saving live in one consistent place per radio.
    private var sdCardHeaderBar: some View {
        RadioActionBar(
            symbol: radios.active?.symbol ?? "sdcard",
            accent: radios.active?.accent ?? Theme.accent,
            name: radios.active?.name ?? "SDS150", subtitle: sdCardSubtitle,
            warning: cardModified ? "modified — eject before reconnecting" : nil,
            onSettings: { showDisplayEditor = true }, settingsDisabled: cardMount == nil,
            onOpen: openBackupCard, onRead: openCard, onWrite: { saveAll() },
            writeDisabled: cardMount == nil || !anyPending || !cardBackedUp,
            writeHelp: cardMount != nil && anyPending && !cardBackedUp
                ? "Back up the card first, then Write" : "Save changes to the card",
            onEject: ejectCard, ejectDisabled: cardMount == nil, ejectModified: cardModified,
            menuItems: cardMenuItems)
    }

    /// The SD-card overflow (⋯) menu — card ops available when a card's loaded.
    private var cardMenuItems: [RadioBarMenuItem] {
        guard cardMount != nil else { return [] }
        return [RadioBarMenuItem(title: "Restore…") { restoreCard() }]
    }

    /// Subtitle for the SD-card header: the card/volume + list count + live-vs-backup, or "no card".
    private var sdCardSubtitle: String {
        Self.sdCardSubtitle(
            hasCard: cardMount != nil, volume: cardVolumeName,
            lists: workingLists.count, isLive: isLiveCard)
    }

    /// Pure builder for the SD-card header subtitle (unit-tested).
    static func sdCardSubtitle(hasCard: Bool, volume: String, lists: Int, isLive: Bool) -> String {
        guard hasCard else { return "card · no card open" }
        let name = volume.isEmpty ? "card" : volume
        let origin = isLive ? "live card" : "backup folder"
        return "\(name) · \(lists) list\(lists == 1 ? "" : "s") · \(origin)"
    }

    /// Open Backup (SD card): pick a saved card backup folder, defaulting to this model's backups.
    private func openBackupCard() {
        guard let url = pickHpdbFolder(startAt: radios.active?.name ?? cardInfo?.model ?? "") else { return }
        let hpdb = ScannerCard.hpdbDir(volumeRoot: url.path) ?? url.path
        openLibrary(path: hpdb, title: "Loading the backup")
    }

    /// Shared folder picker for an SD-card backup / HPDB directory (`s_*.hpd`), starting in the
    /// given model's backups. Returns the picked URL, or nil if cancelled.
    private func pickHpdbFolder(startAt model: String) -> URL? {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.directoryURL = BackupStore.modelRoot(model)
        panel.prompt = "Open"
        panel.message = "Choose a card backup or HPDB folder."
        return panel.runModal() == .OK ? panel.url : nil
    }

    /// Add (or re-point) the HPDB **map source**: pick a folder of `s_*.hpd` files and open it on
    /// a dedicated map handle — independent of the editor library and the target radio, so its
    /// data feeds the map while you write to a different radio.
    private func openMapSource() {
        guard let url = pickHpdbFolder(startAt: radios.active?.name ?? "") else { return }
        let hpdb = ScannerCard.hpdbDir(volumeRoot: url.path) ?? url.path
        loading = true
        loadTitle = "Loading the map source"
        loadFraction = 0
        loadPhase = "Reading files…"
        status = ""
        DispatchQueue.global(qos: .userInitiated).async {
            let progress: (UInt32, UInt32, UInt32) -> Void = { phase, done, total in
                let frac = total > 0 ? Double(done) / Double(total) : 0
                DispatchQueue.main.async {
                    loadPhase = phase == 1 ? "Reading files…" : "Indexing coverage…"
                    loadFraction = phase == 1 ? 0.5 * frac : 0.5 + 0.5 * frac
                }
            }
            let lib = ScannerLibrary(directory: hpdb, progress: progress)
            DispatchQueue.main.async {
                loading = false
                guard let lib else {
                    status = "Could not read that folder."
                    return
                }
                self.mapLibrary = lib
                self.mapDir = hpdb
                self.dataSources.addHpdb(folderPath: hpdb)
                self.lens = .map
            }
        }
    }

    private var renameRow: some View {
        HStack(spacing: 8) {
            TextField("List name", text: selectedNameBinding)
                .textFieldStyle(.roundedBorder)
                .disabled(!canEdit)
            if selectedList?.monitor == true {
                Image(systemName: "antenna.radiowaves.left.and.right")
                    .font(.system(size: 10)).foregroundStyle(Theme.accent)
                    .help("Monitor on — always scanned")
            }
            Button { showListSettings = true } label: {
                Image(systemName: "gearshape").font(.system(size: 12))
                    .foregroundStyle(Theme.fg3)
            }
            .buttonStyle(.plain).disabled(!canEdit).help("List settings")
            .popover(isPresented: $showListSettings, arrowEdge: .bottom) {
                listSettingsPopover
            }
            Text(selectedList?.slot == nil ? "new" : "slot \(selectedList?.slot ?? 0)")
                .font(.system(size: 10)).foregroundStyle(Theme.fg3)
        }
    }

    /// Per-list F-List settings (Monitor, Quick Key, Number Tag) + the sort-systems
    /// action, in one compact popover — the same gear→popover pattern as channels.
    private var listSettingsPopover: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(selectedList.map { $0.name.isEmpty ? "(unnamed list)" : $0.name } ?? "List")
                .font(.system(size: 12.5, weight: .semibold)).lineLimit(1)
            Divider().overlay(Theme.border)
            settingRow("Monitor") {
                Toggle("", isOn: selectedMonitorBinding)
                    .labelsHidden().toggleStyle(.switch).controlSize(.mini)
                    .help(
                        "On = the radio always scans this list (Select Lists to Monitor). "
                            + "Off = controlled by its quick key.")
            }
            settingRow("Quick Key") { keyMenu(selectedQuickKeyBinding) }
            settingRow("Number Tag") { keyMenu(selectedNumberTagBinding) }
            Divider().overlay(Theme.border)
            Button { sortSelectedSystems() } label: {
                Label("Sort systems A→Z", systemImage: "arrow.up.arrow.down")
                    .font(.system(size: 12))
            }
            .buttonStyle(.plain).disabled(selectedSystems.count < 2)
        }
        .padding(14).frame(width: 224).disabled(!canEdit)
    }

    /// A compact "Off / 0–99" menu picker for an optional F-List key field.
    private func keyMenu(_ binding: Binding<Int?>) -> some View {
        Picker("", selection: binding) {
            Text("Off").tag(Int?.none)
            ForEach(0..<100, id: \.self) { Text("\($0)").tag(Int?.some($0)) }
        }
        .labelsHidden().pickerStyle(.menu).controlSize(.small).fixedSize()
        .disabled(!canEdit)
    }

    private var editorGauge: some View {
        let bytes = selectedSummary?.bytes ?? 0
        // Per-list byte budget from the active profile (surfaced by the FFI), not a hardcode.
        let maxBytes = cardInfo?.maxListBytes ?? 1_048_576
        let frac = min(1.0, Double(bytes) / Double(maxBytes))
        let count = selectedSystems.reduce(0) { $0 + $1.channels.count }
        return VStack(alignment: .leading, spacing: 6) {
            Text("\(count)").font(.system(size: 23, weight: .bold))
            Text("channels · \(String(format: "%.1f", Double(bytes) / 1024)) KB")
                .font(.system(size: 12)).foregroundStyle(Theme.fg2)
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 3).fill(Theme.chip)
                    RoundedRectangle(cornerRadius: 3).fill(frac > 0.85 ? Theme.warn : Theme.accent)
                        .frame(width: max(0, geo.size.width * frac))
                }
            }.frame(height: 6)
            Text("\(Int(frac * 100))% of list capacity · \(String(format: "%.0f", Double(maxBytes) / 1_048_576)) MB max")
                .font(.system(size: 11)).foregroundStyle(Theme.fg3)
            if avoidedChannelCount > 0 {
                HStack(spacing: 4) {
                    Image(systemName: "speaker.slash.fill").font(.system(size: 9))
                    Text("\(avoidedChannelCount) avoided (not scanned)").font(.system(size: 11))
                    Spacer()
                    Button(action: removeAvoided) {
                        Text("Remove").font(.system(size: 10.5, weight: .semibold))
                    }
                    .buttonStyle(.plain).foregroundStyle(Theme.accent).disabled(!canEdit)
                    .help("Drop the avoided channels from this list entirely (not just skip them).")
                }.foregroundStyle(Theme.warn)
            }
        }
    }

    /// A scan/avoid pill that toggles the flag at any level.
    private func avoidToggle(_ avoided: Bool, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 3) {
                Image(systemName: avoided ? "speaker.slash.fill" : "dot.radiowaves.left.and.right")
                Text(avoided ? "Avoided" : "Scan")
            }
            .font(.system(size: 9.5, weight: .semibold))
            .foregroundStyle(avoided ? Theme.warn : Color(hex: 0x34c759))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background((avoided ? Theme.warn : Color(hex: 0x34c759)).opacity(0.14))
            .clipShape(Capsule())
        }
        .buttonStyle(.plain).disabled(!canEdit)
        .help(avoided ? "Avoided — tap to scan" : "Scanned — tap to avoid")
    }

    /// A compact menu that sets a per-channel value field from a fixed option list.
    /// `fmt` maps a raw value to its display string. `current` is the channel's value.
    private func valueMenu(
        _ id: String, _ field: String, current: String?, options: [String],
        fmt: @escaping (String) -> String = { $0 }
    ) -> some View {
        Menu {
            ForEach(options, id: \.self) { v in
                Button(fmt(v)) {
                    mutateSelectedContent { $0.settingChannelValue(target: id, field: field, value: v) }
                }
            }
        } label: {
            Text(current.map(fmt) ?? "—")
                .font(.system(size: 11, weight: .medium)).monospacedDigit()
                .foregroundStyle(Theme.fg2)
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().disabled(!canEdit)
    }

    /// Editor body: system → department → channel, each with a scan/avoid toggle.
    /// Avoiding a system or department skips all its channels (shown greyed).
    private var editorList: some View {
        VStack(spacing: 8) {
            ForEach(selectedTree) { sys in
                VStack(spacing: 0) {
                    // System header
                    HStack(spacing: 8) {
                        Image(systemName: sys.isTrunk ? "antenna.radiowaves.left.and.right" : "radio")
                            .font(.system(size: 12)).foregroundStyle(Theme.fg2)
                        Text(sys.name).font(.system(size: 12, weight: .semibold)).lineLimit(1)
                            .foregroundStyle(sys.avoid ? Theme.fg3 : Theme.fg)
                        Spacer()
                        detailsButton(sys.id)
                            .popover(isPresented: detailsBinding(sys.id), arrowEdge: .trailing) {
                                systemDetailsPopover(sys)
                            }
                        avoidToggle(sys.avoid) { toggleAvoid(target: sys.id, currentlyAvoided: sys.avoid) }
                    }
                    .padding(8)

                    ForEach(sys.groups) { group in
                        if !group.name.isEmpty {
                            HStack(spacing: 6) {
                                Image(systemName: "folder").font(.system(size: 10)).foregroundStyle(Theme.fg3)
                                Text(group.name).font(.system(size: 11, weight: .medium)).lineLimit(1)
                                    .foregroundStyle((sys.avoid || group.avoid) ? Theme.fg3 : Theme.fg2)
                                Spacer()
                                detailsButton(group.id)
                                    .popover(isPresented: detailsBinding(group.id), arrowEdge: .trailing) {
                                        groupDetailsPopover(group)
                                    }
                                avoidToggle(group.avoid) {
                                    toggleAvoid(target: group.id, currentlyAvoided: group.avoid)
                                }
                            }
                            .padding(.horizontal, 8).padding(.vertical, 3)
                            .background(Theme.bg2)
                        }
                        ForEach(group.channels) { ch in
                            treeChannelRow(ch, system: sys, group: group)
                        }
                    }
                }
                .background(Theme.panel).clipShape(RoundedRectangle(cornerRadius: Theme.rCard))
                .overlay(RoundedRectangle(cornerRadius: Theme.rCard).stroke(Theme.border))
            }
        }
    }

    private func treeChannelRow(_ ch: FavTreeChannel, system: FavSystemTree, group: FavGroup)
        -> some View
    {
        let groupAvoided = group.avoid
        let parentAvoided = system.avoid || groupAvoided
        let effectiveAvoided = parentAvoided || ch.avoid
        return HStack(spacing: 6) {
            Image(systemName: ServiceType.info(ch.serviceType).symbol)
                .font(.system(size: 11)).foregroundStyle(ServiceType.info(ch.serviceType).color)
                .frame(width: 13).opacity(effectiveAvoided ? 0.5 : 1)
            VStack(alignment: .leading, spacing: 1) {
                Text(ch.name).font(.system(size: 11.5)).lineLimit(1)
                    .foregroundStyle(effectiveAvoided ? Theme.fg3 : Theme.fg)
                    .strikethrough(effectiveAvoided, color: Theme.fg3)
                if parentAvoided && !ch.avoid {
                    Text("Avoided via \(groupAvoided ? "department" : "system")")
                        .font(.system(size: 9)).foregroundStyle(Theme.warn)
                } else {
                    Text(ch.detail).font(.system(size: 9.5).monospacedDigit()).foregroundStyle(Theme.fg3)
                }
            }
            Spacer()
            // At-a-glance markers (read-only); the full controls live in the settings
            // popover. Per-channel settings are only meaningful when no parent avoids it.
            if !parentAvoided {
                if ch.priority {
                    Image(systemName: "star.fill").font(.system(size: 8.5))
                        .foregroundStyle(Color(hex: 0xffcf33)).help("Priority channel")
                }
                if ch.avoid {
                    Image(systemName: "speaker.slash.fill").font(.system(size: 8.5))
                        .foregroundStyle(Theme.warn).help("Avoided (not scanned)")
                }
                Button { settingsChannelID = ch.id } label: {
                    Image(systemName: "gearshape").font(.system(size: 11))
                        .foregroundStyle(Theme.fg3)
                }
                .buttonStyle(.plain).disabled(!canEdit).help("Channel settings")
                .popover(isPresented: settingsPopoverBinding(ch.id), arrowEdge: .trailing) {
                    channelSettingsPopover(ch.id)
                }
            }
            detailsButton(ch.id)
                .popover(isPresented: detailsBinding(ch.id), arrowEdge: .trailing) {
                    channelDetailsPopover(ch, system: system)
                }
            Button { removeFromSelected([ch.id]) } label: {
                Image(systemName: "minus.circle").font(.system(size: 11)).foregroundStyle(Theme.fg3)
            }.buttonStyle(.plain).disabled(!canEdit).help("Remove from list")
        }
        .padding(.horizontal, 8).padding(.vertical, 3)
    }

    /// Binding driving one channel's settings popover off the shared `settingsChannelID`.
    private func settingsPopoverBinding(_ id: String) -> Binding<Bool> {
        Binding(get: { settingsChannelID == id }, set: { if !$0 { settingsChannelID = nil } })
    }

    // MARK: - Read-only details popovers (ⓘ)

    /// The trailing ⓘ button that opens a row's read-only details popover.
    private func detailsButton(_ id: String) -> some View {
        Button { detailsID = id } label: {
            Image(systemName: "info.circle").font(.system(size: 11)).foregroundStyle(Theme.fg3)
        }.buttonStyle(.plain).help("Details")
    }

    /// Binding driving one row's details popover off the shared `detailsID`.
    private func detailsBinding(_ id: String) -> Binding<Bool> {
        Binding(get: { detailsID == id }, set: { if !$0 { detailsID = nil } })
    }

    /// The editable per-channel value fields, mapped to friendly labels in a stable
    /// display order, for the read-only details popover. Only keys present are returned.
    private func channelSettingRows(_ settings: [String: String]) -> [(String, String)] {
        let order: [(String, String)] = [
            ("modulation", "Modulation"), ("audioType", "Audio"), ("attenuator", "Attenuator"),
            ("delay", "Delay"), ("volumeOffset", "Volume offset"), ("numberTag", "Number tag"),
            ("alertTone", "Alert tone"), ("alertVolume", "Alert volume"),
            ("alertColor", "Alert light"), ("alertPattern", "Alert pattern"),
        ]
        return order.compactMap { key, label in
            settings[key].map { (label, $0) }
        }
    }

    /// A read-only label / value line for the details popovers.
    private func detailRow(_ label: String, _ value: String) -> some View {
        HStack {
            Text(label).font(.system(size: 11)).foregroundStyle(Theme.fg3)
            Spacer()
            Text(value).font(.system(size: 11.5, weight: .medium).monospacedDigit())
                .foregroundStyle(Theme.fg2)
        }
    }

    private func channelDetailsPopover(_ ch: FavTreeChannel, system: FavSystemTree) -> some View {
        let info = ServiceType.info(ch.serviceType)
        return VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 7) {
                Image(systemName: info.symbol).foregroundStyle(info.color)
                VStack(alignment: .leading, spacing: 1) {
                    Text(ch.name).font(.system(size: 12.5, weight: .semibold)).lineLimit(2)
                    Text(info.name).font(.system(size: 10)).foregroundStyle(Theme.fg3)
                }
            }
            Divider().overlay(Theme.border)
            detailRow("Type", ch.isTalkgroup ? "Talkgroup" : "Conventional")
            if let tgid = ch.tgid { detailRow("Talkgroup ID", tgid) }
            if let freqHz = ch.freqHz {
                detailRow("Frequency", String(format: "%.4f MHz", Double(freqHz) / 1_000_000))
            }
            if let mode = ch.mode, !mode.isEmpty { detailRow("Mode", mode) }
            if let (label, value) = AudioOption.parse(ch.tone) { detailRow(label, value) }
            detailRow("Service type", ServiceType.info(ch.serviceType).name)
            // Every editable per-record value the model carries, shown read-only here (the
            // gear popover is where they're changed). Stable label order.
            ForEach(channelSettingRows(ch.settings), id: \.0) { row in detailRow(row.0, row.1) }
            detailRow("Priority", ch.priority ? "Yes" : "No")
            detailRow("Scanned", ch.avoid ? "No (avoided)" : "Yes")
            Divider().overlay(Theme.border)
            // Parent-net context: name + a tech pill only when it adds info (drop "Conventional",
            // which the Type row already states). Trunk is implied by a talkgroup Type / real tech.
            if let tech = system.tech, !tech.isEmpty,
                tech.caseInsensitiveCompare("Conventional") != .orderedSame
            {
                HStack(spacing: 6) { badge(tech); Spacer() }
            }
            Text(system.name).font(.system(size: 10.5)).foregroundStyle(Theme.fg3).lineLimit(2)
            Divider().overlay(Theme.border)
            HStack(spacing: 4) {
                Text("Source").font(.system(size: 11)).foregroundStyle(Theme.fg3)
                Spacer()
                sourceBadge(.hpdb)
            }
        }
        .padding(14).frame(width: 240)
    }

    private func systemDetailsPopover(_ sys: FavSystemTree) -> some View {
        let channelCount = sys.groups.reduce(0) { $0 + $1.channels.count }
        let deptCount = sys.groups.filter { !$0.name.isEmpty }.count
        return VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 7) {
                Image(systemName: sys.isTrunk ? "antenna.radiowaves.left.and.right" : "radio")
                    .foregroundStyle(Theme.fg2)
                VStack(alignment: .leading, spacing: 1) {
                    Text(sys.name).font(.system(size: 12.5, weight: .semibold)).lineLimit(2)
                    Text(sys.isTrunk ? "Trunk" : "Conventional")
                        .font(.system(size: 10)).foregroundStyle(Theme.fg3)
                }
            }
            Divider().overlay(Theme.border)
            if let tech = sys.tech, !tech.isEmpty {
                HStack { badge(tech); Spacer() }
            }
            if deptCount > 0 { detailRow("Departments", "\(deptCount)") }
            detailRow("Channels", "\(channelCount)")
            if sys.avoid { detailRow("Avoided", "Yes") }
        }
        .padding(14).frame(width: 240)
    }

    private func groupDetailsPopover(_ group: FavGroup) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 7) {
                Image(systemName: "folder").foregroundStyle(Theme.fg3)
                Text(group.name).font(.system(size: 12.5, weight: .semibold)).lineLimit(2)
            }
            Divider().overlay(Theme.border)
            detailRow("Channels", "\(group.channels.count)")
            if group.avoid { detailRow("Avoided", "Yes") }
        }
        .padding(14).frame(width: 240)
    }

    /// The current tree channel for `id` (re-read each render, since edits rebuild the
    /// tree — keeps the open popover in sync after a toggle).
    private func channelByID(_ id: String) -> FavTreeChannel? {
        selectedTree.flatMap(\.groups).flatMap(\.channels).first { $0.id == id }
    }

    /// The per-channel settings popover: avoid, priority, delay (and future per-channel
    /// attributes) in one compact place, so the rows stay uncluttered.
    private func channelSettingsPopover(_ id: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            if let ch = channelByID(id) {
                HStack(spacing: 7) {
                    Image(systemName: ServiceType.info(ch.serviceType).symbol)
                        .foregroundStyle(ServiceType.info(ch.serviceType).color)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(ch.name).font(.system(size: 12.5, weight: .semibold)).lineLimit(1)
                        Text(ch.detail).font(.system(size: 10).monospacedDigit())
                            .foregroundStyle(Theme.fg3)
                    }
                }
                Divider().overlay(Theme.border)
                settingRow("Scan") {
                    avoidToggle(ch.avoid) { toggleAvoid(target: ch.id, currentlyAvoided: ch.avoid) }
                }
                settingRow("Priority") {
                    Toggle(
                        "",
                        isOn: Binding(
                            get: { ch.priority },
                            set: { on in mutateSelectedContent { $0.settingPriority(target: ch.id, on: on) } }
                        )
                    )
                    .labelsHidden().toggleStyle(.switch).controlSize(.mini)
                }
                // Value settings — only those this record type exposes are present.
                let s = ch.settings
                if let v = s["modulation"] {
                    settingRow("Modulation") {
                        valueMenu(ch.id, "modulation", current: v, options: options("modulation"))
                    }
                }
                if let v = s["audioType"] {
                    settingRow("Audio") {
                        valueMenu(ch.id, "audioType", current: v, options: options("audioType"))
                    }
                }
                if let v = s["delay"] {
                    settingRow("Delay") {
                        valueMenu(
                            ch.id, "delay", current: v, options: options("delay"),
                            fmt: { "\($0) s" })
                    }
                }
                if let v = s["volumeOffset"] {
                    settingRow("Vol. Offset") {
                        valueMenu(
                            ch.id, "volumeOffset", current: v, options: options("volumeOffset"),
                            fmt: { $0 == "0" ? "0" : ($0.hasPrefix("-") ? $0 : "+\($0)") })
                    }
                }
                if s["attenuator"] != nil {
                    settingRow("Attenuator") {
                        Toggle(
                            "",
                            isOn: Binding(
                                get: { s["attenuator"] == "On" },
                                set: { on in
                                    mutateSelectedContent {
                                        $0.settingChannelValue(
                                            target: ch.id, field: "attenuator", value: on ? "On" : "Off")
                                    }
                                }
                            )
                        )
                        .labelsHidden().toggleStyle(.switch).controlSize(.mini)
                    }
                }
                if s["numberTag"] != nil {
                    settingRow("Number Tag") {
                        ChannelNumberField(current: s["numberTag"], maxValue: 999, enabled: canEdit) {
                            setChannelValue(ch.id, "numberTag", $0)
                        }
                    }
                }
                // Alert sub-section — the per-channel light/tone that fires when it keys up.
                if s["alertTone"] != nil || s["alertColor"] != nil {
                    Divider().overlay(Theme.border)
                    Text("ALERT").font(.system(size: 9.5, weight: .semibold))
                        .foregroundStyle(Theme.fg3).tracking(0.5)
                    if let v = s["alertTone"] {
                        settingRow("Tone") {
                            valueMenu(ch.id, "alertTone", current: v, options: options("alertTone"))
                        }
                    }
                    if let v = s["alertVolume"] {
                        settingRow("Volume") {
                            valueMenu(ch.id, "alertVolume", current: v, options: options("alertVolume"))
                        }
                    }
                    if let v = s["alertColor"] {
                        settingRow("Light") { alertColorMenu(ch.id, current: v) }
                    }
                    if let v = s["alertPattern"] {
                        settingRow("Pattern") {
                            valueMenu(ch.id, "alertPattern", current: v, options: options("alertPattern"))
                        }
                    }
                }
            } else {
                Text("Channel no longer in this list.")
                    .font(.system(size: 11)).foregroundStyle(Theme.fg3)
            }
        }
        .padding(14).frame(width: 240).fixedSize(horizontal: false, vertical: true)
        .disabled(!canEdit)
    }

    /// The selectable values for an editable per-channel value `field`, from the card's
    /// **core** profile (`CardInfo.channelValueOptions`) — the single source of truth, so the
    /// UI carries no attribute tables. Empty until a card is open.
    private func options(_ field: String) -> [String] {
        cardInfo?.valueOptions(field) ?? []
    }

    /// Swatch color for an alert-light color name.
    private func alertColorSwatch(_ name: String) -> Color {
        switch name {
        case "Blue": return .blue
        case "Red": return .red
        case "Magenta": return Color(hex: 0xff2fd0)
        case "Green": return .green
        case "Cyan": return .cyan
        case "Yellow": return .yellow
        case "White": return .white
        default: return Theme.fg3  // Off
        }
    }

    /// The alert-light color menu — a colored dot + name, so it reads at a glance.
    private func alertColorMenu(_ id: String, current: String?) -> some View {
        let cur = current ?? "Off"
        return Menu {
            ForEach(options("alertColor"), id: \.self) { name in
                Button(name) { setChannelValue(id, "alertColor", name) }
            }
        } label: {
            HStack(spacing: 4) {
                Image(systemName: cur == "Off" ? "circle" : "circle.fill")
                    .font(.system(size: 9)).foregroundStyle(alertColorSwatch(cur))
                Text(cur).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.fg2)
            }
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().disabled(!canEdit)
    }

    /// Stage an editable per-channel value into the open list's content.
    private func setChannelValue(_ id: String, _ field: String, _ value: String) {
        mutateSelectedContent { $0.settingChannelValue(target: id, field: field, value: value) }
    }

    /// A labeled row in the channel settings popover: label on the left, control right.
    private func settingRow(_ label: String, @ViewBuilder _ control: () -> some View)
        -> some View
    {
        HStack {
            Text(label).font(.system(size: 12)).foregroundStyle(Theme.fg2)
            Spacer()
            control()
        }
    }

    /// One unified Save for the whole card: all staged structural changes (delete /
    /// sort / new / rename) AND every open list's channel edits, written in one
    /// batched pass. Backup-gated.
    private var saveFooter: some View {
        VStack(spacing: 8) {
            HStack(spacing: 6) {
                if anyPending {
                    Circle().fill(Theme.warn).frame(width: 7, height: 7)
                    Text("Unsaved changes").font(.system(size: 11)).foregroundStyle(Theme.fg2)
                } else {
                    Image(systemName: "checkmark.circle.fill").font(.system(size: 11))
                        .foregroundStyle(Color(hex: 0x34c759))
                    Text("Saved to card").font(.system(size: 11)).foregroundStyle(Theme.fg3)
                }
                Spacer()
            }
            if anyPending {
                Text("Pending: \(pendingSummary)").font(.system(size: 11)).foregroundStyle(Theme.fg2)
                // Edits stage freely; saving them to the card requires a backup first. The Write
                // button lives in the header (disabled until backed up); this is the backup CTA.
                if !cardBackedUp { backupBanner }
            }
        }
    }

    /// Shown when there are unsaved edits but no backup yet — saving needs one.
    private var backupBanner: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "exclamationmark.shield.fill").foregroundStyle(Theme.warn)
                Text("Back up before saving").font(.system(size: 12, weight: .semibold))
            }
            Text("Your changes are staged but not written. Saving writes to the card — back up first (a full copy to a folder you pick), then Save.")
                .font(.system(size: 10.5)).foregroundStyle(Theme.fg3).fixedSize(horizontal: false, vertical: true)
            Button { backUpCard() } label: {
                Label("Back Up Card", systemImage: "externaldrive.badge.timemachine")
                    .frame(maxWidth: .infinity).padding(.vertical, 8)
                    .background(Theme.accent).foregroundStyle(.white)
                    .clipShape(RoundedRectangle(cornerRadius: Theme.rField))
            }.buttonStyle(.plain)
        }
        .padding(10).background(Theme.panel).clipShape(RoundedRectangle(cornerRadius: Theme.rCard))
        .overlay(RoundedRectangle(cornerRadius: Theme.rCard).stroke(Theme.warn.opacity(0.4)))
    }


    // MARK: - Reusable bits

    private func section<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.system(size: 10.5, weight: .bold)).tracking(0.6).foregroundStyle(Theme.fg3)
            content()
        }
    }

    private func badge(_ text: String) -> some View {
        Text(text).font(.system(size: 10, weight: .bold))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Theme.chip).foregroundStyle(Theme.fg2)
            .clipShape(RoundedRectangle(cornerRadius: Theme.rChip))
    }

    // MARK: - Hierarchy derivation (from the filtered rows + masters)
    //
    // A system carries its AreaState ids (`states`) and its *covered* counties
    // (`counties`, location-first), so it can appear under several states/counties.

    private func countries(_ sys: CatalogSystem) -> Set<UInt64> {
        Set(sys.states.compactMap { stateToCountry[$0] })
    }

    private var countriesLevel: [(id: UInt64, name: String, count: Int)] {
        var counts: [UInt64: Int] = [:]
        for sys in rows { for c in countries(sys) { counts[c, default: 0] += 1 } }
        return counts.map { (id: $0.key, name: Country.name($0.key), count: $0.value) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private func statesLevel(_ country: UInt64) -> [(id: UInt64, name: String, abbr: String, count: Int)] {
        var counts: [UInt64: Int] = [:]
        for sys in rows {
            for s in sys.states where stateToCountry[s] == country { counts[s, default: 0] += 1 }
        }
        return counts.map { (id: $0.key, name: stateNames[$0.key] ?? "State \($0.key)", abbr: stateAbbr[$0.key] ?? "", count: $0.value) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private func countiesLevel(_ state: UInt64) -> [(id: UInt64, name: String, count: Int)] {
        var counts: [UInt64: Int] = [:]
        for sys in rows {
            for c in sys.counties where countyToState[c] == state { counts[c, default: 0] += 1 }
        }
        return counts.map { (id: $0.key, name: countyNames[$0.key] ?? "County \($0.key)", count: $0.value) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private func systems(inCounty county: UInt64) -> [CatalogSystem] {
        rows.filter { $0.counties.contains(county) }.sorted { $0.name < $1.name }
    }

    /// Systems tagged with a given AreaState id (used for states with no county
    /// breakdown, e.g. the `_MultipleStates` nationwide bucket).
    private func systemsInState(_ state: UInt64) -> [CatalogSystem] {
        rows.filter { $0.states.contains(state) }.sorted { $0.name < $1.name }
    }

    /// Which country split a nationwide/interstate system belongs to. Primary
    /// signal is the countries its geo-placed counties fall in (trunked networks
    /// like ARMER/AWIN/Duke resolve here); reinforced/back-stopped by the
    /// RadioReference `- USA` / `- CAN` name-suffix convention.
    private func nationwideGroup(of sys: CatalogSystem) -> NationwideGroup {
        var countries = Set(
            sys.counties
                .compactMap { countyToState[$0] }
                .compactMap { stateToCountry[$0] })
        let n = sys.name.uppercased()
        if n.contains(" - USA") { countries.insert(1) }
        // The `- CAN` suffix plus Canada-specific tells: ICAO `(CZ…` airspace codes
        // (Edmonton (CZEG), Gander (CZQX)…), the word Canada, and parenthesized
        // provinces — these wide-area systems have no county placement to geo-locate.
        let caTells = [
            " - CAN", "(CZ", "CANADA", "CANADIAN", "(QUEBEC)", "(ONTARIO)", "(ALBERTA)",
            "(MANITOBA)", "(SASKATCHEWAN)", "(NOVA SCOTIA)", "(NEW BRUNSWICK)",
            "(BRITISH COLUMBIA)", "(NEWFOUNDLAND)",
        ]
        if caTells.contains(where: n.contains) { countries.insert(2) }
        let us = countries.contains(1)
        let ca = countries.contains(2)
        if us && ca { return .crossborder }
        if ca { return .canada }
        if us { return .us }
        return .crossborder  // unclassifiable → catch-all
    }

    /// The nationwide/interstate systems (`_MultipleStates`, AreaState 0) in one
    /// country split, alphabetical.
    private func nationwideSystems(in group: NationwideGroup) -> [CatalogSystem] {
        systemsInState(0).filter { nationwideGroup(of: $0) == group }
    }

    /// Systems with no-county (statewide-residual) channels for a given state.
    private func statewideSystems(_ state: UInt64) -> [CatalogSystem] {
        rows.filter { $0.statewide && $0.states.contains(state) }.sorted { $0.name < $1.name }
    }

    private var systemsMatching: [CatalogSystem] {
        rows.sorted { $0.name < $1.name }
    }

    /// Channel-cache key — channels differ per county scope (nil = all/flat).
    private func cacheKey(_ sys: CatalogSystem, _ county: UInt64?) -> String {
        "\(sys.id)|\(county.map(String.init) ?? "all")"
    }

    // MARK: - Actions

    /// Auto-detect a connected scanner card (USB mass storage) and open it.
    private func openCard() {
        let cards = ScannerCard.detect()
        switch cards.count {
        case 0:
            status = "No scanner card detected. Connect the scanner in USB mass-storage mode, "
                + "or use Open Library… for a backup folder."
        case 1:
            openLibrary(path: cards[0].hpdbDir, title: "Loading the card")
        default:
            detectedCards = cards
            showingCardPicker = true
        }
    }

    /// Load a library off the main thread with a progress bar — the full-USA parse +
    /// coverage index would otherwise beachball the window.
    private func openLibrary(path: String, title: String = "Loading the database") {
        loading = true
        loadTitle = title
        loadFraction = 0
        loadPhase = "Reading files…"
        status = ""
        let f = filter
        DispatchQueue.global(qos: .userInitiated).async {
            // Fast path: if this is a live card whose browse DB is unchanged since its
            // last backup, read the heavy browse library from the SSD backup instead
            // of the slow card. `path` (and thus `cardMount`, favorites, writes) still
            // points at the live card — only the browse *source* changes.
            let (browseDir, fromBackup) = self.resolveBrowseSource(cardHpdbDir: path)
            let progress: (UInt32, UInt32, UInt32) -> Void = { phase, done, total in
                let frac = total > 0 ? Double(done) / Double(total) : 0
                DispatchQueue.main.async {
                    if phase == 1 {
                        loadPhase = fromBackup ? "Reading from backup…" : "Reading files…"
                        loadFraction = 0.45 * frac
                    } else {
                        loadPhase = "Indexing coverage…"
                        loadFraction = 0.45 + 0.35 * frac
                    }
                }
            }
            // Open from the backup; if that somehow fails, fall back to the card.
            var openedFrom = browseDir
            var lib = ScannerLibrary(directory: browseDir, progress: progress)
            if lib == nil, fromBackup {
                openedFrom = path
                lib = ScannerLibrary(directory: path, progress: progress)
            }
            guard let lib else {
                DispatchQueue.main.async {
                    loading = false
                    status = "Could not read that folder."
                }
                return
            }
            let loadedFromBackup = fromBackup && openedFrom != path
            // Stats + masters + the first catalog query, still off-main.
            let stats = lib.stats()
            let masters = Masters.load(fromDirectory: openedFrom)
            let rows = lib.catalog(f)
            let cardEval = evaluateCardStatus(hpdbDir: path) { frac in
                DispatchQueue.main.async {
                    loadPhase = "Checking backup status…"
                    loadFraction = 0.80 + 0.20 * frac
                }
            }
            DispatchQueue.main.async {
                self.library = lib
                self.libraryDir = path
                self.stats = stats
                self.applyMasters(masters)
                self.rows = rows
                self.expanded.removeAll()
                self.channelCache.removeAll()
                self.selectedCountry = nil
                self.selectedState = nil
                self.selectedCounty = nil
                self.selectedNationwideGroup = nil
                self.cardModified = false
                self.applyCardLoadStatus(cardEval)
                self.reloadCardInfo()  // also re-baselines the working set
                self.loadFraction = 1
                self.loading = false
                self.status = rows.isEmpty ? "No systems found." : ""
                if loadedFromBackup {
                    self.flashStatus(
                        "Card browse data unchanged since backup — loaded instantly from your "
                            + "backup. Favorites read live from the card.",
                        level: .success)
                }
                // A live card was just read — back it up automatically (like the FT-60). We have
                // the data in hand; a fresh verified backup is cheap insurance and satisfies the
                // Write gate. Backup folders / map sources (not live) never trigger this.
                if cardEval?.isLive == true {
                    self.backUpCard(auto: true)
                }
            }
        }
    }

    /// Decide where to read the heavy browse library from. If `cardHpdbDir` is a live
    /// card whose **browse DB** is metadata-unchanged since its last backup, returns
    /// the backup's HPDB dir (a fast SSD read); otherwise the card's own dir. This
    /// deliberately checks only the browse files (`s_*.hpd` / `hpdb.cfg`) — favorites
    /// and resume state change often but are read live from the card, so their churn
    /// must not force a slow re-read. Any real browse-DB change fails the match and
    /// falls back to the card, so a delta is never missed. Touches only cheap
    /// directory metadata; the expensive content read is what we're skipping.
    private func resolveBrowseSource(cardHpdbDir: String) -> (dir: String, fromBackup: Bool) {
        let card = (cardHpdbDir, false)
        guard ScannerCard.isLiveCard(hpdbDir: cardHpdbDir) else { return card }
        let mount =
            ((cardHpdbDir as NSString).deletingLastPathComponent as NSString).deletingLastPathComponent
        guard let info = CardFavorites.read(cardMount: mount) else { return card }
        let volumeName = CardVolume.forFile(cardHpdbDir)?.name ?? (mount as NSString).lastPathComponent
        guard let latest = BackupStore.latest(model: info.model, volumeName: volumeName),
            let wantBrowse = latest.hpdbMeta,
            FileManager.default.fileExists(atPath: latest.folder),
            let backupHpdb = ScannerCard.hpdbDir(volumeRoot: latest.folder),
            let haveBrowse = try? CardBackup.metaFingerprint(
                ofRoot: URL(fileURLWithPath: cardHpdbDir, isDirectory: true)),
            haveBrowse == wantBrowse
        else { return card }
        return (backupHpdb, true)
    }

    /// What a freshly-loaded source is (computed off-main during `openLibrary`).
    private struct CardLoadStatus {
        let model: String
        let volumeName: String
        let isLive: Bool
        let hasBackup: Bool
        let upToDate: Bool
        let lastBackup: String?
    }

    /// Off-main: is `path` a card? is it live (radio plugged in)? does its current
    /// content match the latest recorded backup? Returns nil if the source isn't
    /// card-shaped (a plain library folder). Touches no `@State` — safe off-main.
    private func evaluateCardStatus(
        hpdbDir path: String, progress: @escaping (Double) -> Void = { _ in }
    ) -> CardLoadStatus? {
        let mount =
            ((path as NSString).deletingLastPathComponent as NSString).deletingLastPathComponent
        guard let info = CardFavorites.read(cardMount: mount) else { return nil }
        let volumeName = CardVolume.forFile(path)?.name ?? (mount as NSString).lastPathComponent
        let isLive = ScannerCard.isLiveCard(hpdbDir: path)
        let root = URL(fileURLWithPath: mount, isDirectory: true)
        let latest = BackupStore.latest(model: info.model, volumeName: volumeName)
        var upToDate = false
        if let latest {
            if let meta = latest.meta {
                // Fast path: metadata-only (size + mtime) — no file content reads, so
                // it's quick even on a slow card.
                progress(0.5)
                upToDate = (try? CardBackup.metaFingerprint(ofRoot: root)) == meta
                progress(1)
            } else if let inv = try? CardBackup.inventory(ofRoot: root),
                // Legacy record (no metadata): fall back to a content compare, gated
                // by the cheap inventory so we only full-hash when it could match.
                inv.files == latest.signature.files, inv.bytes == latest.signature.bytes,
                let sig = try? CardBackup.signature(ofRoot: root, onFraction: progress)
            {
                upToDate = (sig == latest.signature)
                // Backfill metadata so the next load skips the content hash.
                if upToDate, let m = try? CardBackup.metaFingerprint(ofRoot: root) {
                    BackupStore.backfillMeta(folder: latest.folder, meta: m)
                }
            }
        }
        return CardLoadStatus(
            model: info.model, volumeName: volumeName, isLive: isLive,
            hasBackup: latest != nil, upToDate: upToDate, lastBackup: latest?.timestamp)
    }

    /// Reflect the load evaluation into the gate + the always-visible banner.
    private func applyCardLoadStatus(_ s: CardLoadStatus?) {
        guard let s else {
            isLiveCard = false
            cardVolumeName = ""
            cardBackedUp = false
            cardStatus = nil
            return
        }
        isLiveCard = s.isLive
        cardVolumeName = s.volumeName
        if !s.isLive {
            cardBackedUp = false
            flashStatus("Opened backup folder: \(s.model) (not a live card).", level: .info)
        } else if s.upToDate {
            cardBackedUp = true
            flashStatus(
                "Live card: \(s.model) — up to date (backed up \(s.lastBackup ?? "?")). Editing enabled.",
                level: .success)
        } else if s.hasBackup {
            cardBackedUp = false
            cardStatus = CardBanner(
                text:
                    "Live card: \(s.model) — changed since last backup (\(s.lastBackup ?? "?")). Back up before editing.",
                level: .warning)
        } else {
            cardBackedUp = false
            cardStatus = CardBanner(
                text: "Live card: \(s.model) — no backup found. Back up before editing.",
                level: .warning)
        }
    }

    /// Apply the Country→State→County name masters (loaded off-main) to the @State.
    private func applyMasters(_ m: Masters) {
        stateNames = m.stateNames
        stateAbbr = m.stateAbbr
        stateToCountry = m.stateToCountry
        countyNames = m.countyNames
        countyToState = m.countyToState
    }

    private func toggleService(_ code: Int) {
        if filter.services.contains(code) { filter.services.remove(code) } else { filter.services.insert(code) }
        refresh()
    }

    private func toggleTech(_ t: String) {
        if filter.techs.contains(t) { filter.techs.remove(t) } else { filter.techs.insert(t) }
        refresh()
    }

    private func debouncedRefresh() {
        searchDebounce?.cancel()
        let work = DispatchWorkItem { refresh() }
        searchDebounce = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.18, execute: work)
    }

    private func refresh() {
        guard let library else { return }
        // Conventional-only radios (clone-image HTs) can't use trunked systems — hide them so
        // browse/map show only what the active radio can program.
        rows = conventionalOnly ? library.catalog(filter).filter { !$0.isTrunk } : library.catalog(filter)
        channelCache.removeAll()
        expanded.removeAll()
        status = rows.isEmpty ? "No systems match the current filters." : ""
    }

    /// Load a system's channels, scoped to `county` (nil = all; 0 = no-county/statewide).
    /// A nationwide/multi-state system's channels are bucketed by geography, so a
    /// given county/statewide scope can legitimately be empty even though the system
    /// has channels — in that case fall back to the system's full channel list so a
    /// drill-in always shows what's there (rather than an empty expand).
    private func loadChannels(_ sys: CatalogSystem, county: UInt64?) -> [CatalogChannel] {
        let key = cacheKey(sys, county)
        if let cached = channelCache[key] { return cached }
        var chans: [CatalogChannel] = []
        if let county {
            chans = library?.countyChannels(systemID: sys.id, county: county, filter) ?? []
            if chans.isEmpty {
                chans = library?.channels(systemID: sys.id, filter) ?? []
            }
        } else {
            chans = library?.channels(systemID: sys.id, filter) ?? []
        }
        channelCache[key] = chans
        return chans
    }

    private func toggleExpand(_ sys: CatalogSystem, county: UInt64?) {
        if expanded.contains(sys.id) {
            expanded.remove(sys.id)
        } else {
            _ = loadChannels(sys, county: county) // prefetch before the view reads the cache
            expanded.insert(sys.id)
        }
    }

    // MARK: - Adding to the open list ("add this to my favorite here")

    /// Add a catalog system's (county-scoped) channels to the open favorites list.
    private func addSystemToSelected(_ sys: CatalogSystem, county: UInt64?) {
        addChannels(loadChannels(sys, county: county))
    }

    /// Add a map system's channels scoped to the map center + radius — only the
    /// departments near the point, not the whole (possibly statewide) system.
    private func addSystemRadiusToSelected(_ id: String, lat: Double, lon: Double, miles: Double) {
        // Expand from the library the map is actually showing (an added HPDB source, or the
        // editor library) so pin ids resolve; a clone radio rebuilds channels self-contained.
        guard let src = mapSourceLibrary else { return }
        addChannels(src.radiusChannels(systemID: id, lat: lat, lon: lon, miles: miles, filter))
    }

    /// Route a set of catalog channels to the active target: a clone-image radio's memory
    /// (conventional frequencies only, into the active bank tab) or the open SD favorites list.
    private func addChannels(_ chans: [CatalogChannel]) {
        if let ft60 {
            for ch in chans where !ch.isTalkgroup {
                guard let hz = ch.freqHz else { continue }
                ft60.append(
                    ft60.makeFromCatalog(
                        name: ch.name, freqHz: hz, mode: ch.mode, tone: ch.tone,
                        serviceType: ch.serviceType),
                    toBank: ft60Bank)
            }
        } else {
            addToSelected(chans.map { $0.id })
        }
    }

    /// Append library channel ids to the open list (staged; deduped in the core).
    private func addToSelected(_ channelIDs: [String]) {
        guard let library, !channelIDs.isEmpty else { return }
        mutateSelectedContent { library.appendToFavorites($0, channelIDs: channelIDs, departmentsOn: false) }
    }

    /// Add every system currently in view to the open list.
    private func addAllInView() {
        let scope: [CatalogSystem]
        if searching {
            scope = systemsMatching
        } else if let c = selectedCounty {
            scope = c == 0 ? statewideSystems(selectedState ?? 0) : systems(inCounty: c)
        } else {
            scope = []
        }
        let county: UInt64? = searching ? nil : selectedCounty
        var chans: [CatalogChannel] = []
        for sys in scope { chans += loadChannels(sys, county: county) }
        addChannels(chans)
    }

    // MARK: - The staged working set (right editor)

    /// Rebuild the working set from the card (the baseline). Clears any pending edits
    /// — called on card (re)load and after a successful Save.
    private func syncWorkingLists() {
        workingLists = (cardInfo?.lists ?? []).map {
            WorkingList(
                id: UUID(), slot: $0.slot, name: $0.name, originalName: $0.name,
                contentDirty: false, monitor: $0.monitor, originalMonitor: $0.monitor,
                quickKey: $0.quickKey, originalQuickKey: $0.quickKey,
                numberTag: $0.numberTag, originalNumberTag: $0.numberTag)
        }
        contentById = [:]
        selectedListId = nil
        selectedSystems = []
        selectedSummary = nil
    }

    /// Open a list in the editor, lazy-loading its content from the card.
    private func selectList(_ id: UUID) {
        selectedListId = id
        if contentById[id] == nil,
            let wl = workingLists.first(where: { $0.id == id }), let slot = wl.slot,
            let mount = cardMount
        {
            contentById[id] = Favorites.open(cardMount: mount, slot: slot)
        }
        refreshSelectedView()
    }

    private func refreshSelectedView() {
        selectedSystems = selectedContent?.systems() ?? []
        selectedTree = selectedContent?.tree() ?? []
        selectedSummary = selectedContent?.summary()
    }

    /// Toggle the avoid (scan/skip) flag of a system / department / channel. Stages
    /// into the open list's content (written by the unified Save).
    private func toggleAvoid(target: String, currentlyAvoided: Bool) {
        mutateSelectedContent { $0.settingAvoid(target: target, avoid: !currentlyAvoided) }
    }

    /// Channel ids the radio will skip in the open list (own / department / system avoid).
    private var avoidedChannelIDs: [String] {
        selectedTree.flatMap { sys in
            sys.groups.flatMap { g in
                g.channels.filter { sys.avoid || g.avoid || $0.avoid }.map { $0.id }
            }
        }
    }
    private var avoidedChannelCount: Int { avoidedChannelIDs.count }

    /// Turn the avoids into a real exclusion: drop every effectively-avoided channel
    /// (the now-empty departments/systems prune themselves in the core). Staged →
    /// written by the unified Save.
    private func removeAvoided() {
        removeFromSelected(avoidedChannelIDs)
    }

    /// Transform the open list's content in memory (no card write); marks it dirty.
    private func mutateSelectedContent(_ transform: (Favorites) -> Favorites?) {
        guard let id = selectedListId, let cur = contentById[id],
            let next = transform(cur), let i = selectedIndex
        else { return }
        contentById[id] = next
        workingLists[i].contentDirty = true
        refreshSelectedView()
    }

    private func removeFromSelected(_ favIDs: [String]) {
        mutateSelectedContent { $0.removing(channelIDs: favIDs) }
    }
    private func sortSelectedSystems() {
        mutateSelectedContent { $0.sortedBySystem() }
    }

    /// Binding for the open list's name (rename is staged; applied on Save).
    private var selectedNameBinding: Binding<String> {
        Binding(
            get: { selectedList?.name ?? "" },
            set: { v in if let i = selectedIndex { workingLists[i].name = v } })
    }

    /// Binding for the open list's Monitor flag (staged; applied on Save).
    private var selectedMonitorBinding: Binding<Bool> {
        Binding(
            get: { selectedList?.monitor ?? false },
            set: { v in if let i = selectedIndex { workingLists[i].monitor = v } })
    }

    /// Binding for the open list's quick key (0–99, nil = Off; staged).
    private var selectedQuickKeyBinding: Binding<Int?> {
        Binding(
            get: { selectedList?.quickKey },
            set: { v in if let i = selectedIndex { workingLists[i].quickKey = v } })
    }

    /// Binding for the open list's number tag (0–99, nil = Off; staged).
    private var selectedNumberTagBinding: Binding<Int?> {
        Binding(
            get: { selectedList?.numberTag },
            set: { v in if let i = selectedIndex { workingLists[i].numberTag = v } })
    }

    private func newList() {
        guard cardMount != nil else { return }
        // New lists default to monitored, no quick key / number tag (always scanned).
        let wl = WorkingList(
            id: UUID(), slot: nil, name: "New List", originalName: nil,
            contentDirty: true, monitor: true, originalMonitor: true,
            quickKey: nil, originalQuickKey: nil, numberTag: nil, originalNumberTag: nil)
        workingLists.append(wl)
        contentById[wl.id] = Favorites.new()
        selectList(wl.id)
    }

    /// Stage a deletion of the open list (no card write until Save).
    private func deleteSelectedList() {
        guard let id = selectedListId else { return }
        workingLists.removeAll { $0.id == id }
        contentById[id] = nil
        selectedListId = nil
        refreshSelectedView()
    }

    /// Stage alphabetizing the lists (no card write until Save).
    private func sortListsAZ() {
        workingLists.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    /// Write every staged change in **one batched pass**: each changed list's slot
    /// file, then a single `f_list.cfg` rewrite (+ deleted-slot removals + one
    /// `app_data.cfg` delete). Minimizes SD-card writes. Runs behind the blocking
    /// progress modal (off the main thread) and confirms with a modal when done.
    /// `thenEject` chains an eject on success (the "Save & Eject" flow).
    private func saveAll(thenEject: Bool = false) {
        guard cardBackedUp, let mount = cardMount, anyPending else { return }

        // 1. Assign free slots to any new lists (mutates @State — main thread).
        var used = Set(workingLists.compactMap { $0.slot })
        for i in workingLists.indices where workingLists[i].slot == nil {
            var s: UInt32 = 1
            while used.contains(s) { s += 1 }
            workingLists[i].slot = s
            used.insert(s)
        }
        // 2. Snapshot the work so the background thread never touches @State: the
        //    dirty content slots to write, then the full layout to apply.
        let dirty: [(slot: UInt32, content: Favorites)] = workingLists.compactMap { wl in
            guard wl.contentDirty, let slot = wl.slot, let content = contentById[wl.id]
            else { return nil }
            return (slot, content)
        }
        let entries = workingLists.compactMap { wl in
            wl.slot.map {
                (slot: $0, name: wl.name, monitor: Bool?.some(wl.monitor),
                    quickKey: wl.quickKey, numberTag: wl.numberTag)
            }
        }
        let summary = pendingSummary
        // The latest backup for this card (if its folder still exists) — we mirror the
        // same writes into it so it stays a faithful, current copy and the next launch
        // can load from the SSD copy instead of re-reading the whole card.
        let model = cardInfo?.model ?? "Scanner"
        let mirror: URL? = BackupStore.latest(model: model, volumeName: cardVolumeName)
            .map { URL(fileURLWithPath: $0.folder, isDirectory: true) }
            .flatMap { FileManager.default.fileExists(atPath: $0.path) ? $0 : nil }
        let total = dirty.count + 1 + (mirror == nil ? 0 : 1)  // slots + layout (+ mirror)

        opTitle = "Saving to the card"
        opIcon = "square.and.arrow.down"
        opNote = "Writing your changes to the card. Don't disconnect the card."
        opToken = nil  // a card write must not be interrupted half-way
        opPhase = "Preparing…"
        opFraction = 0
        opActive = true
        cardStatus = nil

        DispatchQueue.global(qos: .userInitiated).async {
            // Changed content slot files.
            for (i, item) in dirty.enumerated() {
                DispatchQueue.main.async {
                    opPhase = "Writing list \(i + 1) of \(dirty.count)…"
                    opFraction = Double(i) / Double(total)
                }
                if let err = item.content.writeSlot(cardMount: mount, slot: item.slot) {
                    DispatchQueue.main.async {
                        opActive = false
                        cardStatus = CardBanner(text: "Save failed: \(err)", level: .error)
                    }
                    return
                }
            }
            // One structural pass: rewrite f_list.cfg, drop removed slots, delete app_data.
            DispatchQueue.main.async {
                opPhase = "Updating list index…"
                opFraction = Double(dirty.count) / Double(total)
            }
            if let err = CardFavorites.applyLayout(cardMount: mount, entries: entries) {
                DispatchQueue.main.async {
                    opActive = false
                    cardStatus = CardBanner(text: "Save failed: \(err)", level: .error)
                }
                return
            }
            // Mirror the SAME writes into the latest backup folder (it has an identical
            // model-folder layout, so the core write functions work on it unchanged),
            // then re-point its fingerprints to the post-save card — keeping it the
            // card's current match so the next load skips the slow card read.
            var mirrored = false
            if let mirror {
                DispatchQueue.main.async {
                    opPhase = "Updating backup…"
                    opFraction = Double(dirty.count + 1) / Double(total)
                }
                let bp = mirror.path
                var ok = true
                for item in dirty where ok {
                    if item.content.writeSlot(cardMount: bp, slot: item.slot) != nil { ok = false }
                }
                if ok, CardFavorites.applyLayout(cardMount: bp, entries: entries) != nil { ok = false }
                if ok, let sig = try? CardBackup.signature(ofRoot: mirror) {
                    let meta = try? CardBackup.metaFingerprint(
                        ofRoot: URL(fileURLWithPath: mount, isDirectory: true))
                    BackupStore.updateState(folder: bp, signature: sig, meta: meta)
                    mirrored = true
                }
            }
            // Done — re-baseline from the card (pending clears) and confirm.
            DispatchQueue.main.async {
                opPhase = "Done"
                opFraction = 1
                opActive = false
                cardModified = true
                reloadCardInfo()
                if mirrored {
                    flashStatus(
                        "Saved all changes — backup updated to match. Eject before reconnecting.",
                        level: .success)
                } else {
                    flashStatus("Saved all changes — eject before reconnecting.", level: .success)
                }
                if thenEject {
                    performEject()
                } else {
                    confirmSaved(summary: summary, mirrored: mirrored)
                }
            }
        }
    }

    /// The clear, unmistakable "it saved" confirmation. **Done** is the default —
    /// eject is offered but never forced, since the user may keep editing and saving
    /// and only ejects when they're ready.
    private func confirmSaved(summary: String, mirrored: Bool) {
        let target = cardVolumeName.isEmpty ? "the card" : "“\(cardVolumeName)”"
        let alert = NSAlert()
        alert.alertStyle = .informational
        alert.messageText = "Changes saved to the card"
        alert.informativeText =
            (summary.isEmpty ? "Your changes were written to \(target). "
                : "Saved to \(target): \(summary). ")
            + (mirrored ? "Your backup was updated to match. " : "")
            + "Keep editing as long as you like — just eject before reconnecting the card."
        alert.addButton(withTitle: "Done")  // first = default (Return)
        alert.addButton(withTitle: "Eject Card")
        if alert.runModal() == .alertSecondButtonReturn {
            performEject()
        }
    }

    private func reloadCardInfo() {
        cardInfo = cardMount.flatMap { CardFavorites.read(cardMount: $0) }
        syncWorkingLists()
    }

    // MARK: - Card backup / eject (header)

    /// Write display-customization edits to the card's `profile.cfg` (only the four display records
    /// change; everything else is preserved). Wrapped in the shared operation overlay; deletes
    /// `app_data.cfg` in the core and marks the card modified so the eject reminder shows.
    private func applyDisplayEdits(_ edits: [String]) {
        guard let mount = cardMount else { return }
        let live = isLiveCard
        opTitle = live ? "Writing display settings" : "Saving to backup"
        opIcon = "paintpalette"
        opNote = live
            ? "Updating profile.cfg on the card. Don't disconnect it."
            : "Updating profile.cfg in the backup folder."
        opToken = nil  // a config write is not cancelable
        opPhase = live ? "Writing…" : "Saving…"
        opFraction = 0
        opActive = true
        cardStatus = nil
        DispatchQueue.global(qos: .userInitiated).async {
            let err = DisplayBridge.apply(cardMount: mount, edits: edits)
            DispatchQueue.main.async {
                opActive = false
                if let err {
                    cardStatus = CardBanner(text: "Display write failed: \(err)", level: .error)
                } else if live {
                    cardModified = true  // a physical card was changed — eject before reconnecting
                    flashStatus("Display settings written — eject before reconnecting.", level: .success)
                } else {
                    flashStatus("Display settings saved to the backup.", level: .success)
                }
            }
        }
    }

    /// Back up the card to the managed backups folder. `auto` is the quiet variant fired on Read
    /// (no Finder reveal, gentler success flash), mirroring the FT-60's auto-backup on read.
    private func backUpCard(auto: Bool = false) {
        guard let mount = cardMount else { return }
        let model = cardInfo?.model ?? "Scanner"
        let parent: URL
        do {
            parent = try BackupStore.ensureModelRoot(model)
        } catch {
            cardStatus = CardBanner(text: "Backup failed: \(error.localizedDescription)", level: .error)
            return
        }
        let stamp = Self.timestamp()
        let volumeName =
            cardVolumeName.isEmpty ? (CardVolume.forFile(mount)?.name ?? "") : cardVolumeName
        let token = CardBackup.CancelToken()
        opTitle = "Backing up the card"
        opIcon = "externaldrive.badge.timemachine"
        opNote = "Copying and verifying every file. Don't disconnect the card."
        opToken = token
        opPhase = "Preparing…"
        opFraction = 0
        opActive = true
        cardStatus = nil
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                let r = try CardBackup.backUp(
                    fileOnCard: mount, into: parent, timestamp: stamp,
                    progress: { phase, fraction in
                        DispatchQueue.main.async {
                            opPhase = phase
                            opFraction = fraction
                        }
                    },
                    cancel: token)
                // Capture the card's metadata fingerprint (fast) so future loads can
                // tell "unchanged" without re-hashing the whole card.
                let meta = try? CardBackup.metaFingerprint(
                    ofRoot: URL(fileURLWithPath: mount, isDirectory: true))
                // …and a fingerprint of just the browse DB (HPDB dir), card-rooted, so
                // a later load can serve the heavy browse data from this backup when
                // the card's browse DB is unchanged (favorites still read live).
                let hpdbMeta = ScannerCard.hpdbDir(volumeRoot: mount).flatMap {
                    try? CardBackup.metaFingerprint(ofRoot: URL(fileURLWithPath: $0, isDirectory: true))
                }
                let record = BackupRecord(
                    folder: r.folder.path, timestamp: stamp, model: model,
                    volumeName: volumeName, signature: r.signature, meta: meta,
                    hpdbMeta: hpdbMeta)
                DispatchQueue.main.async {
                    opActive = false
                    BackupStore.append(record)
                    cardBackedUp = true
                    if auto {
                        flashStatus(
                            "Backed up to \(r.folder.lastPathComponent) — "
                                + "\(r.filesVerified) files verified.", level: .success)
                    } else {
                        flashStatus(
                            "Backed up to \(r.folder.lastPathComponent) — "
                                + "\(r.filesVerified) files verified. Up to date; editing enabled.",
                            level: .success)
                        NSWorkspace.shared.activateFileViewerSelecting([r.folder])
                    }
                }
            } catch {
                let isCancel = (error as NSError).code == NSUserCancelledError
                DispatchQueue.main.async {
                    opActive = false
                    if isCancel {
                        flashStatus("Backup cancelled — nothing was saved.", level: .info)
                    } else {
                        cardStatus = CardBanner(
                            text: "Backup failed: \(error.localizedDescription)", level: .error)
                    }
                }
            }
        }
    }

    private func restoreCard() {
        guard let mount = cardMount else { return }
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = false
        panel.prompt = "Restore From Here"
        panel.message = "Choose a backup folder to restore onto the card."
        guard panel.runModal() == .OK, let folder = panel.url else { return }

        guard CardBackup.looksLikeBackup(folder) else {
            cardStatus = CardBanner(
                text: "Restore failed: “\(folder.lastPathComponent)” isn't a card backup (no model folder inside).",
                level: .error)
            return
        }

        let confirm = NSAlert()
        confirm.alertStyle = .critical
        confirm.messageText = "Restore the card from this backup?"
        confirm.informativeText = "This overwrites the card's current programming with "
            + "“\(folder.lastPathComponent)”. Anything you haven't backed up will be lost. "
            + "Eject before reconnecting to the scanner."
        confirm.addButton(withTitle: "Restore")
        confirm.addButton(withTitle: "Cancel")
        guard confirm.runModal() == .alertFirstButtonReturn else { return }

        // No Cancel: restore writes the card; interrupting it half-way would leave
        // the card inconsistent. Show progress only.
        opTitle = "Restoring the card"
        opIcon = "arrow.uturn.backward"
        opNote = "Copying and verifying every file. Don't disconnect the card."
        opToken = nil
        opPhase = "Preparing…"
        opFraction = 0
        opActive = true
        cardStatus = nil
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                let r = try CardBackup.restore(
                    from: folder, toCardHolding: mount,
                    progress: { phase, fraction in
                        DispatchQueue.main.async {
                            opPhase = phase
                            opFraction = fraction
                        }
                    })
                DispatchQueue.main.async {
                    opActive = false
                    cardModified = true
                    reloadCardInfo()  // re-baselines the working set
                    flashStatus(
                        "Restored from \(r.folder.lastPathComponent) — "
                            + "\(r.filesVerified) files verified. Eject before reconnecting.",
                        level: .success)
                }
            } catch {
                DispatchQueue.main.async {
                    opActive = false
                    cardStatus = CardBanner(
                        text: "Restore failed: \(error.localizedDescription)", level: .error)
                }
            }
        }
    }

    private func ejectCard() {
        guard cardMount != nil else { return }

        // Don't silently throw away staged edits on eject.
        if anyPending {
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = "You have unsaved changes"
            alert.informativeText =
                "Ejecting now discards your pending changes (\(pendingSummary)). "
                + "Save them to the card first, or discard and eject."
            alert.addButton(withTitle: "Save & Eject")
            alert.addButton(withTitle: "Discard & Eject")
            alert.addButton(withTitle: "Cancel")
            switch alert.runModal() {
            case .alertFirstButtonReturn:
                saveAll(thenEject: true)  // async; ejects itself on success
                return
            case .alertSecondButtonReturn:
                break  // discard and proceed
            default:
                return  // cancel
            }
        }
        performEject()
    }

    private func performEject() {
        guard let mount = cardMount else { return }
        do {
            let vol = try CardVolume.eject(fileOnCard: mount)
            status = "Ejected “\(vol)”. Safe to reconnect."
            cardStatus = nil
            isLiveCard = false
            cardVolumeName = ""
            cardModified = false
            cardBackedUp = false
            // The card is gone — drop the library + card state.
            library = nil
            libraryDir = nil
            rows = []
            cardInfo = nil
            selectedCountry = nil; selectedState = nil; selectedCounty = nil
            selectedNationwideGroup = nil
            syncWorkingLists()  // cardInfo is nil now → clears the working set
        } catch {
            status = "Eject failed: \(error.localizedDescription) — close anything using the card and retry."
        }
    }

    private static func timestamp() -> String {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HHmm"
        return f.string(from: Date())
    }
}

/// A compact numeric field for a per-channel value (`0`…`maxValue`, empty = "Off").
/// Commits on Return or when focus leaves; reverts on an out-of-range entry.
private struct ChannelNumberField: View {
    let current: String?
    let maxValue: Int
    let enabled: Bool
    let onCommit: (String) -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        TextField("Off", text: $text)
            .textFieldStyle(.roundedBorder).frame(width: 56)
            .multilineTextAlignment(.trailing)
            .font(.system(size: 11)).monospacedDigit()
            .disabled(!enabled)
            .focused($focused)
            .onAppear { text = display(current) }
            .onSubmit(commit)
            .onChange(of: focused) { _, isFocused in
                if !isFocused { commit() }
            }
    }

    private func display(_ v: String?) -> String {
        (v == nil || v == "Off") ? "" : v!
    }

    private func commit() {
        let t = text.trimmingCharacters(in: .whitespaces)
        if t.isEmpty {
            onCommit("Off")
        } else if let n = Int(t), n >= 0, n <= maxValue {
            onCommit(String(n))
        } else {
            text = display(current)  // revert an out-of-range / non-numeric entry
        }
    }
}
