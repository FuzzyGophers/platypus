// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import AppKit
import SwiftUI

/// The location-first view: open an HPDB file, then narrow the systems by county
/// or by a point + radius. Filtering happens in the Rust core via the FFI.
struct ContentView: View {
    @State private var hpdb: Hpdb?
    @State private var loadedPath: String?
    @State private var systems: [ScannerSystem] = []
    @State private var status = "Open an HPDB .hpd file to begin."

    @State private var filter: FilterMode = .all
    @State private var countyText = ""
    @State private var latText = ""
    @State private var lonText = ""
    @State private var radiusText = "25"

    enum FilterMode: String, CaseIterable, Identifiable {
        case all = "All"
        case county = "County"
        case radius = "Radius"
        var id: String { rawValue }
    }

    var body: some View {
        NavigationSplitView {
            controls
                .padding()
                .frame(minWidth: 240)
        } detail: {
            systemList
        }
        .navigationTitle("Platypus")
        .onAppear {
            // Demo/dev convenience: auto-open a file named by $PLATYPUS_OPEN.
            if hpdb == nil, let p = ProcessInfo.processInfo.environment["PLATYPUS_OPEN"] {
                load(path: p)
            }
        }
    }

    // MARK: - Controls

    private var controls: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button("Open HPDB…", action: openFile)
            HStack {
                Button("Back Up Card", action: backUpCard)
                    .disabled(loadedPath == nil)
                    .help("Copy the whole card to a folder — always do this before any write.")
                Button("Eject Card", action: ejectCard)
                    .disabled(loadedPath == nil)
                    .help("Flush and eject the card — always do this before reconnecting to the scanner.")
            }

            Picker("Filter", selection: $filter) {
                ForEach(FilterMode.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .disabled(hpdb == nil)

            switch filter {
            case .all:
                EmptyView()
            case .county:
                TextField("County id", text: $countyText)
                    .textFieldStyle(.roundedBorder)
            case .radius:
                TextField("Latitude", text: $latText).textFieldStyle(.roundedBorder)
                TextField("Longitude", text: $lonText).textFieldStyle(.roundedBorder)
                TextField("Miles", text: $radiusText).textFieldStyle(.roundedBorder)
            }

            Button("Apply", action: applyFilter)
                .disabled(hpdb == nil)
                .keyboardShortcut(.return)

            Spacer()
            Text(status)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    // MARK: - System list

    private var systemList: some View {
        List(systems) { system in
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(system.name).font(.headline)
                    Spacer()
                    Text(system.kind)
                        .font(.caption).foregroundStyle(.secondary)
                }
                if !system.counties.isEmpty {
                    Text("Counties: " + system.counties.map(String.init).joined(separator: ", "))
                        .font(.caption).foregroundStyle(.secondary)
                }
                ForEach(system.locations.prefix(4)) { loc in
                    Text("• \(loc.name)  (\(loc.lat, specifier: "%.4f"), \(loc.lon, specifier: "%.4f")) ~\(loc.range, specifier: "%.1f") mi")
                        .font(.caption2).foregroundStyle(.secondary)
                }
            }
            .padding(.vertical, 2)
        }
        .overlay {
            if systems.isEmpty {
                Text(status).foregroundStyle(.secondary)
            }
        }
    }

    // MARK: - Actions

    private func openFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.message = "Choose a per-state .hpd file (or any HPDB .hpd/.cfg)."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        load(path: url.path)
    }

    private func load(path: String) {
        let name = (path as NSString).lastPathComponent
        guard let db = Hpdb(path: path) else {
            status = "Could not parse \(name)."
            hpdb = nil
            systems = []
            return
        }
        hpdb = db
        loadedPath = path
        filter = .all
        applyFilter()
        status = "Loaded \(name)."
    }

    /// Full backup of the card before any write — read-only of the card. Lets the
    /// user pick where the backup goes (which also grants write access there),
    /// then copies on a background queue so the UI stays responsive.
    private func backUpCard() {
        guard let path = loadedPath else { return }
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.prompt = "Back Up Here"
        panel.message = "Choose where to save a full backup of the card."
        guard panel.runModal() == .OK, let parent = panel.url else { return }

        let stamp = Self.timestamp()
        status = "Backing up the card…"
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                let r = try CardBackup.backUp(fileOnCard: path, into: parent, timestamp: stamp)
                DispatchQueue.main.async {
                    status = "Backed up to \(r.folder.path) — \(r.filesVerified) files verified."
                    NSWorkspace.shared.activateFileViewerSelecting([r.folder])
                }
            } catch {
                DispatchQueue.main.async {
                    status = "Backup failed: \(error.localizedDescription)"
                }
            }
        }
    }

    private static func timestamp() -> String {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HHmm"
        return f.string(from: Date())
    }

    /// Safely eject the card the loaded file lives on. We drop our in-memory
    /// handle first (no open fd, but tidy), then eject — and only report "safe"
    /// if the eject actually succeeds. This is the mandatory step before
    /// reconnecting the card to the scanner.
    private func ejectCard() {
        guard let path = loadedPath else { return }
        do {
            let volume = try CardVolume.eject(fileOnCard: path)
            hpdb = nil
            systems = []
            loadedPath = nil
            status = "Ejected “\(volume)”. Safe to reconnect / exit USB Mass Storage on the scanner."
        } catch {
            status =
                "Eject failed: \(error.localizedDescription) — close anything using the card and retry. Do NOT reconnect yet."
        }
    }

    private func applyFilter() {
        guard let hpdb else { return }
        switch filter {
        case .all:
            systems = hpdb.allSystems()
        case .county:
            guard let id = UInt64(countyText.trimmingCharacters(in: .whitespaces)) else {
                status = "Enter a numeric county id."
                return
            }
            systems = hpdb.systems(inCounty: id)
        case .radius:
            guard let lat = Double(latText), let lon = Double(lonText),
                let mi = Double(radiusText)
            else {
                status = "Enter numeric latitude, longitude, and miles."
                return
            }
            systems = hpdb.systems(near: lat, lon, milesRadius: mi)
        }
        status = "\(systems.count) system\(systems.count == 1 ? "" : "s")."
    }
}
