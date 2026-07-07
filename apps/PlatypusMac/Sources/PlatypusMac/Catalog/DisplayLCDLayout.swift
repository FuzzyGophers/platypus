// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import SwiftUI

// Layout + draw for the SDS150 pixel screen. `layout` places every field/label in the native
// 320×240 grid (recording field rects for hit-testing); `draw` paints the placed ops into a
// GraphicsContext with the bundled pixel fonts. Kept apart from the view so the same placement
// drives both the bitmap and the click map.

/// A single paint op in native LCD pixels.
enum LCDOp {
    case fill(CGRect, Color)
    case hline(CGFloat, CGFloat, CGFloat, Color)  // x1, x2, y, color
    case text(String, CGFloat, CGFloat, String, CGFloat, Color, LCDAlign)  // str,x,y,font,size,color,align
}

enum LCDAlign { case left, right, center }

/// The result of laying out a screen: the paint ops plus each field's rect (for hit-testing and
/// the selection outline).
struct LCDLaid {
    var ops: [LCDOp] = []
    var fields: [(key: String, rect: CGRect)] = []

    func hit(_ p: CGPoint) -> String? { fields.last { $0.rect.contains(p) }?.key }
    func rect(_ key: String) -> CGRect? { fields.first { $0.key == key }?.rect }
}

enum LCDLayout {
    static let w: CGFloat = 320
    static let margin: CGFloat = 6

    // MARK: - Layout

    static func layout(_ s: LCDScreen) -> LCDLaid {
        var l = LCDLaid()
        var y = statusBand(s, into: &l)
        y += 3
        bodyFields(s, startY: y, into: &l)
        indicatorAndSoftkeys(s, into: &l)
        return l
    }

    /// The top status band — data-driven + colorable: the status flags (F/SIG/BAT/SP0/KEY, group 6),
    /// the icons (group 5), and the small-area item tokens (group 3), each colored from its pair and
    /// recorded for hit-testing. Placement is representative (not pixel-photo-exact). Returns the y
    /// just under the band's rule.
    private static func statusBand(_ s: LCDScreen, into l: inout LCDLaid) -> CGFloat {
        let y0: CGFloat = 2
        var x = margin
        // Left flags: F, SP0, KEY (group-6 indices 0, 3, 4).
        for i in [0, 3, 4] where i < s.statusFlags.count {
            let f = s.statusFlags[i]
            let wd = est(f.text, 8)
            l.ops.append(.text(f.text, x, y0, LCDFonts.small, 8, f.color, .left))
            l.fields.append((f.id, CGRect(x: x - 1, y: y0 - 1, width: wd + 2, height: 11)))
            x += wd + 6
        }
        // Icons: small colored squares (group 5).
        for ic in s.icons {
            let sq = CGRect(x: x, y: y0 + 1, width: 7, height: 7)
            l.ops.append(.fill(sq, ic.color))
            l.fields.append((ic.id, CGRect(x: x - 1, y: y0 - 1, width: 9, height: 11)))
            x += 10
        }
        // Right flags: BAT then SIG (group-6 indices 2, 1).
        var rx = w - margin
        for i in [2, 1] where i < s.statusFlags.count {
            let f = s.statusFlags[i]
            let wd = est(f.text, 8)
            l.ops.append(.text(f.text, rx, y0, LCDFonts.small, 8, f.color, .right))
            l.fields.append((f.id, CGRect(x: rx - wd - 1, y: y0 - 1, width: wd + 2, height: 11)))
            rx -= wd + 6
        }
        // Small-area item tokens row (group 3).
        var tx = margin
        let ty = y0 + 11
        for tok in s.statusTokens {
            let wd = est(tok.text, 8)
            if tx + wd > w - margin { break }
            l.ops.append(.text(tok.text, tx, ty, LCDFonts.small, 8, tok.dim ? LCDTier.dim : tok.color, .left))
            l.fields.append((tok.id, CGRect(x: tx - 1, y: ty - 1, width: wd + 2, height: 11)))
            tx += wd + 8
        }
        let bottom = ty + 12
        l.ops.append(.hline(0, w, bottom, .white.opacity(0.28)))
        return bottom
    }

    /// The programmable body — the tier names, meta rows, service line, and data grid.
    private static func bodyFields(_ s: LCDScreen, startY: CGFloat, into l: inout LCDLaid) {
        var y = startY
        let xl: CGFloat = s.dense ? 15 : 21
        let gap: CGFloat = s.dense ? 2 : 7
        for fld in s.body {
            switch fld.kind {
            case .xl:
                let h = xl + 3
                let r = CGRect(x: margin, y: y, width: w - margin * 2, height: h)
                if let b = fld.back { l.ops.append(.fill(r, b)) }
                l.ops.append(.text(fld.text, margin, y + 1, LCDFonts.big, xl, fld.color, .left))
                l.fields.append((fld.id, r))
                y += h + gap
            case .meta:
                let h: CGFloat = 13
                let r = CGRect(x: margin, y: y, width: w - margin * 2, height: h)
                if let b = fld.back { l.ops.append(.fill(r, b)) }
                let col = fld.dim ? LCDTier.dim : fld.color
                l.ops.append(.text(fld.text, margin, y + 1, LCDFonts.small, 9, col, .left))
                if fld.avoid { l.ops.append(.text("AVOID", w - margin, y + 2, LCDFonts.smallBold, 7, fld.avoidColor, .right)) }
                l.ops.append(.hline(margin, w - margin, y + h - 1, col.opacity(0.55)))
                l.fields.append((fld.id, r))
                y += h + (s.dense ? 1 : 3)
            case .svc:
                let h: CGFloat = 14
                let r = CGRect(x: margin, y: y, width: w - margin * 2, height: h)
                l.ops.append(.text(fld.text, margin, y + 1, LCDFonts.small, 10, fld.color, .left))
                if let rt = fld.right { l.ops.append(.text(rt, w - margin, y + 1, LCDFonts.small, 10, fld.rightColor, .right)) }
                l.fields.append((fld.id, r))
                y += h + gap
            case .srow:
                let h: CGFloat = 13
                let r = CGRect(x: margin, y: y, width: w - margin * 2, height: h)
                l.ops.append(.text(fld.text, margin, y + 1, LCDFonts.small, 9, fld.color, .left))
                if fld.hold {
                    let hb = CGRect(x: w - margin - 30, y: y, width: 30, height: 11)
                    l.ops.append(.fill(hb, LCDTier.yellow))
                    l.ops.append(.text("HOLD", w - margin - 15, y + 1, LCDFonts.smallBold, 7, LCDTier.bg, .center))
                    l.ops.append(.text("AVOID", w - margin - 36, y + 2, LCDFonts.smallBold, 7, fld.color, .right))
                } else if fld.avoid {
                    l.ops.append(.text("AVOID", w - margin, y + 2, LCDFonts.smallBold, 7, fld.avoidColor, .right))
                }
                l.fields.append((fld.id, r))
                y += h + gap
            case .grid:
                let rowH: CGFloat = 12
                let top = y
                for (i, pair) in fld.gridRows.enumerated() {
                    let ry = y + CGFloat(i) * rowH
                    l.ops.append(.text(pair.0, margin, ry, LCDFonts.small, 9, fld.color, .left))
                    l.ops.append(.text(pair.1, margin + (w - margin * 2) / 2, ry, LCDFonts.small, 9, fld.color, .left))
                }
                let h = CGFloat(fld.gridRows.count) * rowH
                l.fields.append((fld.id, CGRect(x: margin, y: top, width: w - margin * 2, height: h)))
                y += h + gap
            }
        }
    }

    /// The indicator token row + the three softkeys ([---] [T-COLOR] [B-COLOR]).
    private static func indicatorAndSoftkeys(_ s: LCDScreen, into l: inout LCDLaid) {
        let indY: CGFloat = 214
        l.ops.append(.hline(0, w, indY - 2, .white.opacity(0.28)))
        var x = margin
        for cell in s.indicator {
            let col: Color = cell.dim ? LCDTier.dim : LCDTier.white
            if cell.box { l.ops.append(.fill(CGRect(x: x - 1, y: indY - 1, width: est(cell.text, 8) + 2, height: 11), Color.white.opacity(0.14))) }
            l.ops.append(.text(cell.text, x, indY, LCDFonts.small, 8, col, .left))
            x += est(cell.text, 8) + 5
        }
        // Softkeys — the three group-7 keys, colored from their pairs + clickable.
        let ky: CGFloat = 226
        let kw = (w - margin * 2 - 12) / 3
        for i in 0..<3 {
            let kx = margin + CGFloat(i) * (kw + 6)
            let r = CGRect(x: kx, y: ky, width: kw, height: 12)
            l.ops.append(.fill(r, .white.opacity(0.06)))
            let key = i < s.softkeys.count ? s.softkeys[i] : nil
            let label = key?.text ?? "\u{2014}\u{2014}\u{2014}"
            l.ops.append(.text(label, kx + kw / 2, ky + 2, LCDFonts.small, 8, key?.color ?? LCDTier.white, .center))
            if let key { l.fields.append((key.id, r)) }
        }
    }

    /// Crude monospace width estimate for the pixel font (Silkscreen ≈ 0.62em per glyph).
    private static func est(_ s: String, _ size: CGFloat) -> CGFloat { CGFloat(s.count) * size * 0.62 }

    // MARK: - Draw

    static func draw(_ l: LCDLaid, in ctx: inout GraphicsContext, selected: String?) {
        for op in l.ops {
            switch op {
            case let .fill(r, c):
                ctx.fill(Path(r), with: .color(c))
            case let .hline(x1, x2, y, c):
                var p = Path()
                p.move(to: CGPoint(x: x1, y: y)); p.addLine(to: CGPoint(x: x2, y: y))
                ctx.stroke(p, with: .color(c), lineWidth: 1)
            case let .text(str, x, y, font, size, color, align):
                var t = ctx.resolve(Text(str).font(.custom(font, size: size)).foregroundColor(color))
                let m = t.measure(in: CGSize(width: w, height: 200))
                let px: CGFloat
                switch align {
                case .left: px = x
                case .right: px = x - m.width
                case .center: px = x - m.width / 2
                }
                t.shading = .color(color)
                ctx.draw(t, at: CGPoint(x: px, y: y), anchor: .topLeading)
            }
        }
    }
}
