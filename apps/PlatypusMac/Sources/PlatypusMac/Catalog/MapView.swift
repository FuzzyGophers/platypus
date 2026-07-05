// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import CoreLocation
import MapKit
import SwiftUI

/// Direction 1A — the map lens. Pan to your area, set a radius, and **Load here** to
/// plot the systems whose coverage reaches the center: a pin per system (colored by
/// dominant service type) with its coverage circle. Tap a pin to add that system to
/// the cart, or **Add area** to add every system in range. Shares the catalog filter
/// + cart, so it's just another way to select.
struct MapLensView: View {
    let library: ScannerLibrary?
    let filter: FilterState
    /// Whether a favorites list is open to add into.
    let canAdd: Bool
    /// When true (a conventional-only radio is active), drop trunked systems — a clone-image
    /// HT can't use them.
    var conventionalOnly: Bool = false
    /// Name of the open favorites list, for the confirm dialog.
    let listName: String?
    /// Add a system (by `d<i>s<j>` id) to the open list, scoped to the current map
    /// center + radius (only the departments near here).
    let onAdd: (_ systemID: String, _ lat: Double, _ lon: Double, _ miles: Double) -> Void

    @State private var camera: MapCameraPosition = .region(
        MKCoordinateRegion(
            center: CLLocationCoordinate2D(latitude: 39.5, longitude: -98.35),
            span: MKCoordinateSpan(latitudeDelta: 9, longitudeDelta: 9)))
    @State private var center = CLLocationCoordinate2D(latitude: 39.5, longitude: -98.35)
    @State private var radiusMi: Double = 25
    @State private var systems: [GeoSystem] = []
    @State private var status = "Pan to your area, set a radius, then Load here."
    @State private var searchText = ""
    @State private var geocoding = false
    /// True once the user has done an initial "Load here"; afterward the lens re-queries
    /// live as the radius or the map viewport changes, so the pins track the ring.
    @State private var live = false

    /// Count of in-range systems staged for the "Add area" confirmation (nil = no dialog).
    @State private var pendingArea: Int?
    /// The hovered system — its coverage circle shows only while the cursor is over its pin.
    @State private var hoveredID: String?
    /// The system whose info popover is open (nil = none). Single-click a pin to inspect a net.
    @State private var infoID: String?
    /// The open net's radius-scoped channels — fetched once when its popover opens.
    @State private var infoChannels: [CatalogChannel] = []

    private func color(_ s: GeoSystem) -> Color { ServiceType.info(s.serviceType).color }
    private func meters(_ mi: Double) -> Double { mi * 1609.34 }

    var body: some View {
        ZStack(alignment: .bottom) {
            Map(position: $camera) {
                // The radius ring at the map center.
                MapCircle(center: center, radius: meters(radiusMi))
                    .foregroundStyle(Theme.accent.opacity(0.06))
                    .stroke(Theme.accent.opacity(0.6), lineWidth: 1)
                ForEach(systems) { sys in
                    let coord = CLLocationCoordinate2D(latitude: sys.lat, longitude: sys.lon)
                    // Coverage circle only for the hovered system, to avoid concentric clutter.
                    if hoveredID == sys.id {
                        MapCircle(center: coord, radius: meters(max(sys.rangeMi, 1)))
                            .foregroundStyle(color(sys).opacity(0.10))
                            .stroke(color(sys).opacity(0.4), lineWidth: 0.5)
                    }
                    Annotation(sys.name, coordinate: coord) {
                        pin(sys)
                    }
                }
            }
            .mapStyle(.standard(elevation: .flat, emphasis: .muted,
                                pointsOfInterest: .excludingAll, showsTraffic: false))
            .onMapCameraChange(frequency: .onEnd) { ctx in
                center = ctx.region.center
                if live { load() }
            }
            // The catalog filter (service type / tech / text) is shared — re-query the lens when
            // it changes so the pins reflect the active filter.
            .onChange(of: filter) { if live { load() } }

            searchBar
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)

            controls
        }
        .confirmationDialog(confirmTitle, isPresented: pendingBinding, titleVisibility: .visible) {
            Button("Add", action: commit)
            Button("Cancel", role: .cancel) { pendingArea = nil }
        }
    }

    /// A map pin: a colored service-type badge. Hover reveals its coverage circle; a single
    /// click opens the net's info popover.
    private func pin(_ sys: GeoSystem) -> some View {
        Image(systemName: ServiceType.info(sys.serviceType).symbol)
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 28, height: 28)
            .background(Circle().fill(color(sys)))
            .overlay(Circle().stroke(.white, lineWidth: 1.5))
            .shadow(color: .black.opacity(0.3), radius: 2, y: 1)
            .contentShape(Circle())
            .onHover { hoveredID = $0 ? sys.id : nil }
            .onTapGesture { openInfo(sys) }
            .popover(isPresented: infoBinding(sys.id), arrowEdge: .top) { infoPopover(sys) }
    }

    /// The friendly name of the open favorites list (for labels), or a generic fallback.
    private var listLabel: String { listName?.isEmpty == false ? listName! : "favorites" }

    private var pendingBinding: Binding<Bool> {
        Binding(get: { pendingArea != nil }, set: { if !$0 { pendingArea = nil } })
    }

    private var confirmTitle: String {
        pendingArea.map { "Add \($0) systems to “\(listLabel)”?" } ?? ""
    }

    // MARK: - Net info popover

    private func infoBinding(_ id: String) -> Binding<Bool> {
        Binding(get: { infoID == id }, set: { if !$0 { infoID = nil } })
    }

    /// Open the info popover for a net, loading the same radius-scoped channels that "Add" appends.
    private func openInfo(_ sys: GeoSystem) {
        infoID = sys.id
        infoChannels = library?.radiusChannels(
            systemID: sys.id, lat: center.latitude, lon: center.longitude, miles: radiusMi, filter) ?? []
    }

    @ViewBuilder private func infoPopover(_ sys: GeoSystem) -> some View {
        let info = ServiceType.info(sys.serviceType)
        let allTalkgroups = !infoChannels.isEmpty && infoChannels.allSatisfy { $0.isTalkgroup }
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: info.symbol).font(.system(size: 15)).foregroundStyle(info.color)
                    .frame(width: 20)
                VStack(alignment: .leading, spacing: 1) {
                    Text(sys.name).font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.fg)
                        .lineLimit(2)
                    Text(info.name).font(.system(size: 10.5)).foregroundStyle(Theme.fg3)
                }
            }
            Divider().overlay(Theme.border)
            HStack(spacing: 6) {
                metaBadge(sys.kind == "Trunk" ? "Trunk" : "Conventional")
                if let tech = sys.tech, !tech.isEmpty { metaBadge(tech) }
                Spacer()
                Text("~\(Int(sys.rangeMi)) mi").font(.system(size: 10.5)).foregroundStyle(Theme.fg3)
            }
            Text("\(infoChannels.count) \(allTalkgroups ? "talkgroups" : "channels")")
                .font(.system(size: 10.5, weight: .semibold)).foregroundStyle(Theme.fg2)
            if infoChannels.isEmpty {
                Text("No channels in range.").font(.system(size: 11)).foregroundStyle(Theme.fg3)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(infoChannels) { ch in
                            HStack(spacing: 6) {
                                Image(systemName: ServiceType.info(ch.serviceType).symbol)
                                    .font(.system(size: 10)).foregroundStyle(ServiceType.info(ch.serviceType).color)
                                    .frame(width: 13)
                                VStack(alignment: .leading, spacing: 0) {
                                    Text(ch.name).font(.system(size: 12)).foregroundStyle(Theme.fg).lineLimit(1)
                                    if let tone = ch.toneInline {
                                        Text(tone).font(.system(size: 9)).foregroundStyle(Theme.fg3).lineLimit(1)
                                    }
                                }
                                Spacer(minLength: 8)
                                Text(ch.detail).font(.system(size: 10.5).monospacedDigit())
                                    .foregroundStyle(Theme.fg2)
                            }
                        }
                    }
                }
                .frame(maxHeight: 200)
            }
            Divider().overlay(Theme.border)
            Button { addFromInfo(sys) } label: {
                Label("Add to “\(listLabel)”", systemImage: "plus.circle")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent).controlSize(.small).disabled(!canAdd)
        }
        .padding(14)
        .frame(width: 280)
    }

    private func metaBadge(_ text: String) -> some View {
        Text(text).font(.system(size: 10, weight: .bold))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Theme.chip).foregroundStyle(Theme.fg2)
            .clipShape(RoundedRectangle(cornerRadius: Theme.rChip))
    }

    /// Append this net (radius-scoped) to the open list, then close the popover.
    private func addFromInfo(_ sys: GeoSystem) {
        guard canAdd else { return }
        infoID = nil
        onAdd(sys.id, center.latitude, center.longitude, radiusMi)
    }

    /// A ZIP / place search that recenters the map (Apple geocoding — no API key).
    private var searchBar: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").font(.system(size: 11)).foregroundStyle(Theme.fg3)
            TextField("ZIP or place", text: $searchText)
                .textFieldStyle(.plain).font(.system(size: 12)).frame(width: 148)
                .onSubmit { jumpTo(searchText) }
            if geocoding {
                ProgressView().controlSize(.small).scaleEffect(0.7)
            } else if !searchText.isEmpty {
                Button { searchText = "" } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.fg3)
                }
                .buttonStyle(.plain).help("Clear")
            }
        }
        .padding(.horizontal, 11).padding(.vertical, 7)
        .background(.regularMaterial)
        .clipShape(Capsule())
        .overlay(Capsule().stroke(Theme.border))
        .padding(14)
    }

    private var controls: some View {
        HStack(spacing: 12) {
            Image(systemName: "smallcircle.filled.circle").foregroundStyle(Theme.accent)
            HStack(spacing: 8) {
                Text("Radius").font(.system(size: 11)).foregroundStyle(Theme.fg2)
                Slider(value: $radiusMi, in: 5...80, step: 1) { editing in
                    if !editing, live { load() }
                }
                .frame(width: 130)
                Text("\(Int(radiusMi)) mi").font(.system(size: 12, weight: .semibold)).monospacedDigit()
                    .frame(width: 46, alignment: .leading)
            }
            Button("Load here", action: load).buttonStyle(.borderedProminent).controlSize(.small)
            if !systems.isEmpty, canAdd {
                Button("Add area (\(systems.count))", action: addArea).controlSize(.small)
            }
            Text(systems.isEmpty ? status : "\(systems.count) systems in range")
                .font(.system(size: 11)).foregroundStyle(Theme.fg3).lineLimit(1)
        }
        .padding(.horizontal, 14).padding(.vertical, 9)
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 11))
        .overlay(RoundedRectangle(cornerRadius: 11).stroke(Theme.border))
        .padding(14)
    }

    // MARK: - Actions

    private func load() {
        live = true
        var found = library?.geo(lat: center.latitude, lon: center.longitude, miles: radiusMi, filter) ?? []
        if conventionalOnly { found = found.filter { $0.kind != "Trunk" } }
        systems = found
        status = systems.isEmpty ? "No systems with coverage here." : ""
    }

    /// Geocode a ZIP / city / address (Apple's `CLGeocoder`, no API key), recenter the
    /// map + radius ring there, and load the systems in range.
    private func jumpTo(_ query: String) {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return }
        geocoding = true
        CLGeocoder().geocodeAddressString(q) { placemarks, _ in
            DispatchQueue.main.async {
                geocoding = false
                guard let coord = placemarks?.first?.location?.coordinate else {
                    systems = []
                    status = "Couldn't find “\(q)”. Try a ZIP, city, or address."
                    return
                }
                center = coord
                // Frame the radius ring comfortably (~1° lat ≈ 69 mi).
                let deg = max(0.15, radiusMi * 2.6 / 69.0)
                camera = .region(
                    MKCoordinateRegion(
                        center: coord,
                        span: MKCoordinateSpan(latitudeDelta: deg, longitudeDelta: deg)))
                load()
            }
        }
    }

    /// Stage the whole in-range area for confirmation.
    private func addArea() {
        guard canAdd, !systems.isEmpty else { return }
        pendingArea = systems.count
    }

    /// Add every in-range system, then clear the pending state.
    private func commit() {
        for sys in systems { onAdd(sys.id, center.latitude, center.longitude, radiusMi) }
        pendingArea = nil
    }
}
