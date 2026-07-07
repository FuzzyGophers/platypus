// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

// A pixel-accurate render of the SDS150's 320×240 QVGA screen for the Customize Display editor.
// The layout is drawn into the true 320×240 grid with the bundled pixel fonts, rasterized at 1×,
// and upscaled nearest-neighbor so it reads as a crisp LCD. Each on-screen field records its rect
// so clicks map back to a field (two-way selection with the inspector).

/// The SDS150 display mode families — named modes show System/Dept/Channel; scan modes (Search /
/// Weather / Tone-Out) show Primary Area 1–3 (confirmed from the hardware screens).
enum LCDModeFamily { case named, scan }

/// One on-screen field, its content text, and the model coordinates its colors bind to. `colorPair`
/// points at a real `DispColors` pair (dispColorId, index) when the field is directly colorable;
/// nil means it inherits a region color (rendered, not individually editable).
struct LCDField: Identifiable {
    enum Kind { case xl, meta, svc, srow, grid }
    let id: String            // stable field key (system, dept, channel, pa1, freq, …)
    let kind: Kind
    var text: String
    var color: Color          // text color used to render
    var back: Color? = nil    // background fill (nil = transparent)
    var avoid = false         // trailing "AVOID" tag on this row
    var avoidColor: Color = LCDTier.red   // the AVOID tag's own color (its DispColors pair)
    var hold = false          // inverse "HOLD" box (Search/WX/Tone)
    var dim = false           // dimmed placeholder (the `---` dept line)
    var right: String? = nil  // right-aligned secondary (e.g. Service Type's CTCSS/DCS/NAC)
    var rightColor: Color = .white
    var gridRows: [(String, String)] = []
    /// The `DispColors` pair this field's Text/Background edit writes to, if any.
    var colorPair: (dispColorId: Int, index: Int)? = nil
    /// The `DispColors` pair the trailing AVOID tag edits (group 1 odd indices), if any.
    var avoidPair: (dispColorId: Int, index: Int)? = nil
    /// The `DispColors` pair the right-aligned secondary edits (e.g. Large-area Option B_1).
    var rightPair: (dispColorId: Int, index: Int)? = nil
}

/// The full laid-out screen for one mode: the status band, the body fields, the indicator tokens,
/// and the softkey labels — everything the renderer needs.
struct LCDScreen {
    var family: LCDModeFamily
    var detailStatus: Bool          // 4-line status band (detail + scan) vs 2-line (simple)
    var dense: Bool                 // detail modes pack tighter
    var blankMeta: Bool             // Weather/Tone-Out blank the Date/Time
    var numberTag: Bool
    var body: [LCDField]
    var statusTokens: [LCDField]    // small-area items (DispColorId=3), colored + clickable
    var statusFlags: [LCDField]     // F/SIG/BAT/SP0/KEY (DispColorId=6)
    var icons: [LCDField]           // ICON1-5 (DispColorId=5)
    var softkeys: [LCDField]        // Soft Key 1/2/3 (DispColorId=7)
    var indicator: [(text: String, box: Bool, dim: Bool)]
    var hold: Bool

    /// Every selectable/colorable element across the screen (body + chrome) — for looking up a
    /// clicked field's color pair in the inspector.
    var allFields: [LCDField] { body + statusTokens + statusFlags + icons + softkeys }
}

/// Tier defaults from the reference photos — the representative colors used for chrome and for any
/// field not bound to a real color pair. Name tiers themselves are colored from the live model.
enum LCDTier {
    static let white = Color(hex: 0xeef2ff)
    static let red = Color(hex: 0xff3f27)
    static let orange = Color(hex: 0xff6a2a)
    static let amber = Color(hex: 0xffab2e)
    static let yellow = Color(hex: 0xffe500)
    static let dim = Color(hex: 0x7d8391)
    static let bg = Color(hex: 0x06070d)
}

/// The pixel LCD view: renders `screen` into a 320×240 bitmap and upscales nearest-neighbor. Taps
/// are hit-tested against the recorded field rects and reported via `onPick`. `selected` outlines
/// the current field.
struct LCDPixelView: View {
    let screen: LCDScreen
    let scale: CGFloat
    var selected: String? = nil
    var onPick: (String) -> Void = { _ in }

    // Native panel size.
    static let w: CGFloat = 320
    static let h: CGFloat = 240

    var body: some View {
        // Lay the screen out once (native 320×240 coords); the rects drive drawing, the click map,
        // and the selection outline. Draw scaled up with normal anti-aliasing — crisp at full res.
        let laid = LCDLayout.layout(screen)
        ZStack(alignment: .topLeading) {
            Canvas { ctx, _ in
                ctx.scaleBy(x: scale, y: scale)
                ctx.fill(Path(CGRect(x: 0, y: 0, width: Self.w, height: Self.h)), with: .color(LCDTier.bg))
                LCDLayout.draw(laid, in: &ctx, selected: nil)
            }
            .frame(width: Self.w * scale, height: Self.h * scale)
            if let sel = selected, let r = laid.rect(sel) {
                Rectangle()
                    .strokeBorder(Theme.accent, style: StrokeStyle(lineWidth: 1.5, dash: [3, 2]))
                    .frame(width: r.width * scale, height: r.height * scale)
                    .offset(x: r.minX * scale, y: r.minY * scale)
            }
        }
        .frame(width: Self.w * scale, height: Self.h * scale, alignment: .topLeading)
        .contentShape(Rectangle())
        .onTapGesture { pt in
            if let f = laid.hit(CGPoint(x: pt.x / scale, y: pt.y / scale)) { onPick(f) }
        }
    }
}
