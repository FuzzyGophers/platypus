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
    /// Geo-located systems within `miles` of a point — supplied by the active browse source (an
    /// HPDB library or a RadioReference session), so the map isn't tied to one source type.
    let geo: (_ lat: Double, _ lon: Double, _ miles: Double, _ filter: FilterState) -> [GeoSystem]
    /// A system's radius-scoped channels for the info popover / add. Empty for browse-only sources
    /// (e.g. RadioReference) that don't support "add near here" yet.
    let radiusChannels:
        (_ systemID: String, _ lat: Double, _ lon: Double, _ miles: Double, _ filter: FilterState)
            -> [CatalogChannel]
    let filter: FilterState
    /// Whether a radio is active (show the Add affordances at all — gated by `addReadyReason`).
    let showAdd: Bool
    /// The reason adds aren't possible yet (nil ⇒ ready). Non-nil disables Add + is the hint /
    /// popover message (e.g. "Read your FT-60R first to add channels.").
    let addReadyReason: String?
    /// Name of the open favorites list (or radio), for the confirm dialog.
    let listName: String?
    /// When set (a location-queryable source like RadioReference has a fetched location), the map
    /// auto-centers there and loads on open — no "pan and Load here" step. Nil for HPDB (nationwide).
    var initialCenter: CLLocationCoordinate2D? = nil
    /// For a location-queryable source: searching a place / "locate me" *fetches* that location's
    /// systems (rather than just panning). When set, the map's search + locate route here; the
    /// resulting `initialCenter` change recenters + loads. Nil for HPDB (in-memory, pan-to-load).
    var onLocationSearch: ((CLLocationCoordinate2D) -> Void)? = nil
    /// Changes whenever the active source set changes — the lens re-queries so a removed source's
    /// pins drop (and an added source's appear) without needing a pan/filter change.
    var reloadToken: String = ""
    /// Per-pin add gate by source × active target — a source with no write path to the current
    /// target radio can be browsed on the map but not programmed, so its pins' Add is disabled.
    /// Default: all pins addable.
    var addAllowed: ((GeoSystem) -> Bool)? = nil
    /// Per-pin add gate — why the active target can't take this pin (nil = it can): the radio can't
    /// program it (capability) or there's no path from its source yet. Blocked pins are dimmed and
    /// their Add is disabled with the reason. Default: all pins fit.
    var blocked: ((GeoSystem) -> AddBlock?)? = nil
    /// Add a system (by `d<i>s<j>` id) to the open list, scoped to the current map
    /// center + radius (only the departments near here). Declared last so it takes the trailing closure.
    let onAdd: (_ systemID: String, _ lat: Double, _ lon: Double, _ miles: Double) -> Void

    @State private var camera: MapCameraPosition = .region(
        MKCoordinateRegion(
            center: CLLocationCoordinate2D(latitude: 39.5, longitude: -98.35),
            span: MKCoordinateSpan(latitudeDelta: 9, longitudeDelta: 9)))
    @State private var center = CLLocationCoordinate2D(latitude: 39.5, longitude: -98.35)
    @State private var radiusMi: Double = 25
    @State private var systems: [GeoSystem] = []
    @State private var status = "Pan or zoom to your area — it loads automatically."
    @State private var searchText = ""
    @State private var geocoding = false
    /// True while a current-location fix is pending (drives the spinner + disables the button).
    @State private var locating = false
    @StateObject private var locator = LocationFinder()
    /// True once the user has done an initial "Load here"; afterward the lens re-queries
    /// live as the radius or the map viewport changes, so the pins track the ring.
    @State private var live = false
    /// The center the shared location is currently anchored to. When navigation moves the center far
    /// enough from here, we re-anchor to the new center (so location-scoped sources follow the pan/zoom).
    /// Set on every (re-)anchor to dedup and to reset the movement baseline.
    @State private var lastAnchoredCenter: CLLocationCoordinate2D?
    /// Debounces the auto-load reverse-geocode so a continuous pan/zoom coalesces into one anchor
    /// (and stays gentle on Apple's rate-limited geocoder). Cancelled + rescheduled on each move.
    @State private var reanchorWork: DispatchWorkItem?

    /// Count of in-range systems staged for the "Add area" confirmation (nil = no dialog).
    /// The addable systems staged for the "Add area" confirmation (nil = no dialog).
    @State private var pendingArea: [GeoSystem]?
    /// The hovered system — its coverage circle shows only while the cursor is over its pin.
    @State private var hoveredID: String?
    /// The system whose info popover is open (nil = none). Single-click a pin to inspect a net.
    @State private var infoID: String?
    /// The open net's radius-scoped channels — fetched once when its popover opens.
    @State private var infoChannels: [CatalogChannel] = []
    /// True while the info popover's channels are loading (a networked source).
    @State private var infoLoading = false

    /// Pin + coverage-ring color. Falls back to the accent (not the muted "Other" gray) when the
    /// service type is unknown, so pins and their hover rings stay visible — e.g. RadioReference
    /// systems, which don't carry a dominant service type until a system is drilled.
    private func color(_ s: GeoSystem) -> Color {
        s.serviceType == nil ? Theme.accent : ServiceType.info(s.serviceType).color
    }
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
                if live { load() }  // instant re-filter for coordinate sources between anchors
                maybeAutoLoad()     // re-derive the shared location when navigation moves far enough
            }
            // The catalog filter (service type / tech / text) is shared — re-query the lens when
            // it changes so the pins reflect the active filter.
            .onChange(of: filter) { if live { load() } }
            // A location-queryable source arrives with a known location — center + load immediately
            // (no "pan and Load here"), so switching to the map just shows what you searched. The
            // onChange picks up a new location fetched from *either* lens (chip, map search, locate).
            .onAppear {
                if let c = initialCenter { recenter(on: c) }
                // No shared location yet → seed the movement baseline at the default center so the
                // initial settle is a no-move; the first deliberate navigation past the threshold
                // auto-loads (rather than anchoring the meaningless nationwide default).
                else { lastAnchoredCenter = center }
            }
            .onChange(of: initialCenter?.latitude) { if let c = initialCenter { recenter(on: c) } }
            .onChange(of: initialCenter?.longitude) { if let c = initialCenter { recenter(on: c) } }
            // The active source set changed (a source was toggled off/on) — re-query so pins track it.
            .onChange(of: reloadToken) { if live { load() } }

            searchBar
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)

            controls
        }
        .confirmationDialog(confirmTitle, isPresented: pendingBinding, titleVisibility: .visible) {
            Button("Add", action: commit)
            Button("Cancel", role: .cancel) { pendingArea = nil }
        } message: {
            if pendingSkipped > 0 {
                Text("\(pendingSkipped) more in view can’t be programmed to this radio and will be skipped.")
            }
        }
    }

    /// A map pin: a colored service-type badge. Hover reveals its coverage circle; a single
    /// click opens the net's info popover.
    private func pin(_ sys: GeoSystem) -> some View {
        // The active target can't take this system → grey + dim it (still visible + tappable for
        // awareness), rather than hide it.
        let isBlocked = blocked?(sys) != nil
        return Image(systemName: ServiceType.info(sys.serviceType).symbol)
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 28, height: 28)
            .background(Circle().fill(isBlocked ? Color.gray : color(sys)))
            .overlay(Circle().stroke(.white, lineWidth: 1.5))
            .shadow(color: .black.opacity(0.3), radius: 2, y: 1)
            .opacity(isBlocked ? 0.5 : 1)
            .contentShape(Circle())
            .modifier(AppearIn())  // gently fade + scale in as its site warms (no teleport)
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
        guard let n = pendingArea?.count else { return "" }
        return "Add \(n) system\(n == 1 ? "" : "s") to “\(listLabel)”?"
    }

    /// How many in-range systems the confirm dialog is leaving out (can't be programmed here).
    private var pendingSkipped: Int { (pendingArea != nil) ? systems.count - (pendingArea?.count ?? 0) : 0 }

    // MARK: - Net info popover

    private func infoBinding(_ id: String) -> Binding<Bool> {
        Binding(get: { infoID == id }, set: { if !$0 { infoID = nil } })
    }

    /// Open the info popover for a net, loading its local channels. The provider may be networked
    /// (RadioReference), so run it off-main and show a loading state.
    private func openInfo(_ sys: GeoSystem) {
        infoID = sys.id
        infoChannels = []
        infoLoading = true
        let (id, clat, clon, r, f) = (sys.id, center.latitude, center.longitude, radiusMi, filter)
        DispatchQueue.global(qos: .userInitiated).async {
            let chans = radiusChannels(id, clat, clon, r, f)
            DispatchQueue.main.async {
                guard infoID == id else { return }  // popover still open for this pin
                infoChannels = chans
                infoLoading = false
            }
        }
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
                sourceChip(sys.source)
                Spacer()
                Text("~\(Int(sys.rangeMi)) mi").font(.system(size: 10.5)).foregroundStyle(Theme.fg3)
            }
            if infoLoading {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Loading channels…").font(.system(size: 11)).foregroundStyle(Theme.fg3)
                }
            } else {
                Text("\(infoChannels.count) \(allTalkgroups ? "talkgroups" : "channels")")
                    .font(.system(size: 10.5, weight: .semibold)).foregroundStyle(Theme.fg2)
            }
            if infoLoading {
                EmptyView()
            } else if infoChannels.isEmpty {
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
                // A definite height so the ScrollView actually scrolls (its ideal height is ~0, so a
                // bare maxHeight collapses it to one row inside a self-sizing popover).
                .frame(height: min(200, CGFloat(infoChannels.count) * 26 + 4))
            }
            Divider().overlay(Theme.border)
            // Three states: the radio can't take this system (block reason) · no target loaded yet
            // (readiness reason) · addable (the Add button). The reasons wrap in a subtle chip.
            if let reason = blocked?(sys) {
                reasonChip(reason.detail)
            } else if let notReady = addReadyReason {
                reasonChip(notReady)
            } else {
                Button { addFromInfo(sys) } label: {
                    Label("Add to “\(listLabel)”", systemImage: "plus.circle")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent).controlSize(.small).disabled(!canAddPin(sys))
            }
        }
        .padding(14)
        .frame(width: 280)
    }

    /// A wrapping "why you can't add this" chip for the info popover (no truncation, noticeable).
    private func reasonChip(_ text: String) -> some View {
        Label(text, systemImage: "slash.circle")
            .font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.fg2)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
            .background(RoundedRectangle(cornerRadius: 8).fill(Theme.chip))
    }

    private func metaBadge(_ text: String) -> some View {
        Text(text).font(.system(size: 10, weight: .bold))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Theme.chip).foregroundStyle(Theme.fg2)
            .clipShape(RoundedRectangle(cornerRadius: Theme.rChip))
    }

    /// Provenance chip (tinted dot + short label) so a pin says which source it came from.
    private func sourceChip(_ kind: DataSourceKind) -> some View {
        HStack(spacing: 3) {
            Circle().fill(kind.tint).frame(width: 6, height: 6)
            Text(kind.badge).font(.system(size: 9, weight: .bold)).foregroundStyle(Theme.fg2)
        }
        .padding(.horizontal, 5).padding(.vertical, 2)
        .background(Theme.chip).clipShape(Capsule())
    }

    /// Whether this pin's source permits programming its channels (else browse-only).
    private func canAddPin(_ sys: GeoSystem) -> Bool {
        (addAllowed?(sys) ?? true) && blocked?(sys) == nil
    }

    /// Append this net (radius-scoped) to the open list, then close the popover.
    private func addFromInfo(_ sys: GeoSystem) {
        guard addReadyReason == nil, canAddPin(sys) else { return }
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
            if geocoding || locating {
                ProgressView().controlSize(.small).scaleEffect(0.7)
            } else if !searchText.isEmpty {
                Button { searchText = "" } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.fg3)
                }
                .buttonStyle(.plain).help("Clear")
            }
            Divider().frame(height: 14)
            Button(action: locateMe) {
                Image(systemName: "location.fill").font(.system(size: 11)).foregroundStyle(Theme.accent)
            }
            .buttonStyle(.plain).help("Center on my location").disabled(locating)
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
            Button("Load here", action: reloadHere).buttonStyle(.borderedProminent).controlSize(.small)
            // "Add area" shows for any active radio (parity), counts only the systems this radio can
            // program, and is disabled with a hint until a target is loaded.
            if showAdd, !systems.isEmpty {
                let addable = systems.filter { blocked?($0) == nil }
                Button("Add area (\(addable.count))") { addArea(addable) }
                    .controlSize(.small)
                    .disabled(addReadyReason != nil || addable.isEmpty)
                    .help(addReadyReason ?? "Add the \(addable.count) programmable system\(addable.count == 1 ? "" : "s") in range")
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
        // The geo provider can be slow the first time (a network source fetches site locations), so
        // run it off-main and show a locating state; instant thereafter (cached).
        let (clat, clon, r, f) = (center.latitude, center.longitude, radiusMi, filter)
        if systems.isEmpty { status = "Locating systems…" }
        DispatchQueue.global(qos: .userInitiated).async {
            // Every system the active sources cover is plotted; ones the target radio can't program
            // are dimmed by `pin(_:)` (not filtered out), so the map stays a full picture of the area.
            let found = geo(clat, clon, r, f)
            DispatchQueue.main.async {
                systems = found
                status = systems.isEmpty ? "No systems with coverage here." : ""
            }
        }
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
                if let onLocationSearch {
                    status = "Fetching systems…"
                    onLocationSearch(coord)  // the resulting location change recenters + loads
                } else {
                    recenter(on: coord)
                }
            }
        }
    }

    /// Center the map on my current location (`CLLocationManager`, one-shot fix), then recenter
    /// the ring + load — the same path as the ZIP/place jump.
    private func locateMe() {
        locating = true
        status = "Finding your location…"
        locator.locate { result in
            locating = false
            switch result {
            case .success(let coord):
                if let onLocationSearch {
                    status = "Fetching systems…"
                    onLocationSearch(coord)
                } else {
                    recenter(on: coord)
                }
            case .failure(let err):
                systems = []
                status = err.localizedDescription
            }
        }
    }

    /// Recenter the map + radius ring on `coord` and load the systems in range. Shared by the
    /// ZIP/place search and the "center on me" button.
    private func recenter(on coord: CLLocationCoordinate2D) {
        // A "jump" (search / locate / switch) moves somewhere new → reframe to the ring and show the
        // loading state. A pan-anchor lands ~where the user already is → keep their pan/zoom untouched
        // and just reload the data there, so re-anchoring after a drag doesn't yank the camera.
        let jumped = milesBetween(coord, center) > 3
        lastAnchoredCenter = coord
        center = coord
        if jumped {
            // Drop the previous location's pins so the switch shows the "Locating…" state during the
            // fetch rather than a blank recentered map with stale, off-screen pins. (Warm reloads go
            // through `load()` via `reloadToken`, not here, so incremental pin bursts don't flicker.)
            systems = []
            status = "Locating systems…"
            let deg = Self.frameSpanDegrees(radiusMi: radiusMi)
            camera = .region(
                MKCoordinateRegion(
                    center: coord,
                    span: MKCoordinateSpan(latitudeDelta: deg, longitudeDelta: deg)))
        }
        load()
    }

    /// Movement (miles) beyond which navigating the map re-derives the shared location — county-scale,
    /// scaled to the zoom (radius) with a floor so a small radius doesn't re-anchor on every nudge.
    private var reanchorThresholdMi: Double { max(8, radiusMi * 0.5) }

    /// Seconds the map must sit **still** on a new area before it auto-loads. Any movement restarts it,
    /// so a load fires only once the user has settled — one fetch per settled area, never a flood of
    /// upstream requests (the networked SOAP source + Apple's rate-limited reverse-geocoder) while
    /// they're still scrubbing around.
    private static let autoLoadDwell: TimeInterval = 2.0

    /// When map navigation (pan/zoom) settles far enough from the last **loaded** area, re-derive the
    /// **shared location** from the new center — reverse-geocoded upstream via `onLocationSearch` — so
    /// every active location-scoped source loads it. No button, no typed ZIP, source-agnostic. Gated by
    /// two things: a distance threshold (ordinary panning within an area doesn't refetch) and a still-
    /// dwell (`autoLoadDwell`). **Every** settle cancels the pending timer first, so any movement
    /// restarts the dwell; the load only runs when the map has rested `autoLoadDwell` seconds.
    private func maybeAutoLoad() {
        guard let onLocationSearch else { return }
        reanchorWork?.cancel()  // any movement cancels/restarts the pending load
        let baseline = lastAnchoredCenter ?? initialCenter
        if let baseline, milesBetween(center, baseline) <= reanchorThresholdMi { return }
        // `lastAnchoredCenter` is intentionally *not* stamped here — it updates only when a load lands
        // (via `recenter`), so the distance gate keeps measuring against the last loaded area and a
        // sub-threshold nudge that cancels the timer still reschedules rather than dropping the load.
        let target = center
        let work = DispatchWorkItem {
            live = true
            status = "Locating systems…"
            onLocationSearch(target)
        }
        reanchorWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.autoLoadDwell, execute: work)
    }

    /// "Load here" — force a reload of the current center immediately, bypassing both the distance gate
    /// and the still-dwell (the manual counterpart to the automatic re-anchor). Falls back to a plain
    /// in-place load when no upstream location resolver is wired.
    private func reloadHere() {
        reanchorWork?.cancel()
        live = true
        status = "Locating systems…"
        if let onLocationSearch { onLocationSearch(center) } else { load() }
    }

    /// Great-circle distance between two coordinates, in miles.
    private func milesBetween(_ a: CLLocationCoordinate2D, _ b: CLLocationCoordinate2D) -> Double {
        CLLocation(latitude: a.latitude, longitude: a.longitude)
            .distance(from: CLLocation(latitude: b.latitude, longitude: b.longitude)) / 1609.34
    }

    /// Latitude/longitude span (degrees) that frames a radius ring comfortably (~1° lat ≈ 69 mi).
    /// Floored so a tiny radius doesn't zoom in too far. Pure, so it's unit-testable.
    static func frameSpanDegrees(radiusMi: Double) -> Double {
        max(0.15, radiusMi * 2.6 / 69.0)
    }

    /// Stage the whole in-range area for confirmation.
    /// Stage the programmable in-range systems for confirmation (the incompatible ones are excluded).
    private func addArea(_ addable: [GeoSystem]) {
        guard addReadyReason == nil, !addable.isEmpty else { return }
        pendingArea = addable
    }

    /// Add every staged (programmable) system, then clear the pending state.
    private func commit() {
        for sys in pendingArea ?? [] { onAdd(sys.id, center.latitude, center.longitude, radiusMi) }
        pendingArea = nil
    }
}

/// One-shot current-location helper behind the map's "center on me" button. `CLLocationManager`
/// needs an `NSObject` delegate, so this wraps it and hands the resolved coordinate (or an error)
/// to a single completion. Requests when-in-use authorization on first use; the app is
/// unsandboxed, so no entitlement is needed — only `NSLocationWhenInUseUsageDescription` in
/// `Info.plist`.
final class LocationFinder: NSObject, ObservableObject, CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private var onFix: ((Result<CLLocationCoordinate2D, Error>) -> Void)?

    enum LocateError: LocalizedError {
        case denied
        var errorDescription: String? {
            "Location access is off. Enable it in System Settings › Privacy & Security › Location Services."
        }
    }

    /// Request authorization (if needed) and a single location fix. `completion` runs on the main
    /// thread with the coordinate or an error. A second call while one is pending replaces the
    /// completion.
    func locate(completion: @escaping (Result<CLLocationCoordinate2D, Error>) -> Void) {
        onFix = completion
        manager.delegate = self
        if manager.authorizationStatus == .notDetermined {
            manager.requestWhenInUseAuthorization()  // fix is requested on the auth callback
        } else {
            proceed(with: manager.authorizationStatus)
        }
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        guard onFix != nil else { return }  // ignore the callback from just setting the delegate
        proceed(with: manager.authorizationStatus)
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let coord = locations.last?.coordinate else { return }
        finish(.success(coord))
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        finish(.failure(error))
    }

    private func proceed(with status: CLAuthorizationStatus) {
        switch status {
        case .authorizedWhenInUse, .authorizedAlways: manager.requestLocation()
        case .denied, .restricted: finish(.failure(LocateError.denied))
        default: break  // notDetermined — still waiting on the prompt
        }
    }

    private func finish(_ result: Result<CLLocationCoordinate2D, Error>) {
        guard let cb = onFix else { return }
        onFix = nil
        DispatchQueue.main.async { cb(result) }
    }
}

/// Fade + scale a map pin in the first time it renders, so a warmed-in pin arrives gently rather than
/// popping/teleporting. The annotation `ForEach` is keyed by system id, so only *new* pins run
/// `onAppear` — existing ones are reused and hold their place.
private struct AppearIn: ViewModifier {
    @State private var shown = false
    func body(content: Content) -> some View {
        content
            .scaleEffect(shown ? 1 : 0.4)
            .opacity(shown ? 1 : 0)
            .onAppear {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.7)) { shown = true }
            }
    }
}
