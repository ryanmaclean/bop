#!/usr/bin/env swift
// set_card_icon.swift <path>
//
// Sets a custom Finder icon on a .bop bundle OR a state directory.
//
// .bop bundle  → stage-coloured rounded square with playing-card glyph (fan crop)
// state directory  → same colour + bold state label (DONE / FAILED / etc.)
//
// State colours (shared between cards and their parent dirs):
//   pending → midnight blue   running → dark amber   done  → forest green
//   merged  → deep violet     failed  → dark crimson  (other) → charcoal

import AppKit
import Foundation

// MARK: - Args

guard CommandLine.arguments.count == 2 else {
    fputs("usage: set_card_icon.swift <card.bop | state-dir>\n", stderr)
    exit(2)
}

let targetPath = (CommandLine.arguments[1] as NSString).expandingTildeInPath
let targetURL  = URL(fileURLWithPath: targetPath)
let isCard     = targetURL.lastPathComponent.hasSuffix(".bop")

// MARK: - Colours (keyed by state name)

func stateColor(_ state: String) -> NSColor {
    switch state {
    case "running":  return NSColor(calibratedRed: 0.58, green: 0.32, blue: 0.06, alpha: 1)
    case "done":     return NSColor(calibratedRed: 0.07, green: 0.36, blue: 0.17, alpha: 1)
    case "merged":   return NSColor(calibratedRed: 0.30, green: 0.13, blue: 0.58, alpha: 1)
    case "failed":   return NSColor(calibratedRed: 0.52, green: 0.09, blue: 0.09, alpha: 1)
    case "pending":  return NSColor(calibratedRed: 0.14, green: 0.22, blue: 0.38, alpha: 1)
    case "drafts":   return NSColor(calibratedRed: 0.35, green: 0.35, blue: 0.38, alpha: 1)
    default:         return NSColor(calibratedRed: 0.18, green: 0.18, blue: 0.22, alpha: 1)
    }
}

// MARK: - State

let state: String
if isCard {
    state = targetURL.deletingLastPathComponent().lastPathComponent
} else {
    state = targetURL.lastPathComponent   // the dir itself is the state
}

let bgColor = stateColor(state)
let bgLight = bgColor.highlight(withLevel: 0.22) ?? bgColor

// MARK: - Render

let size   = CGFloat(512)
let image  = NSImage(size: NSSize(width: size, height: size))
let rect   = NSRect(x: 0, y: 0, width: size, height: size)
let radius = size * 0.22

image.lockFocus()

guard NSGraphicsContext.current != nil else {
    fputs("no graphics context\n", stderr); exit(1)
}
NSGraphicsContext.current?.imageInterpolation = .high

// Background gradient
let path = NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius)
path.addClip()
let gradient = NSGradient(colors: [bgLight, bgColor], atLocations: [0, 1],
                          colorSpace: .deviceRGB)!
gradient.draw(in: rect, angle: -60)

// Inner highlight ring
let ring = NSBezierPath(roundedRect: rect.insetBy(dx: 2, dy: 2),
                        xRadius: radius - 2, yRadius: radius - 2)
NSColor(white: 1, alpha: 0.15).setStroke()
ring.lineWidth = 4
ring.stroke()

let cream = NSColor(calibratedRed: 1.0, green: 0.97, blue: 0.88, alpha: 0.96)

if isCard {
    // ── Card icon: playing-card glyph, fan-crop from top ──────────────────────
    let metaURL = targetURL.appendingPathComponent("meta.json")
    struct CardMeta: Decodable { let glyph: String? }
    let glyph: String
    if let data = try? Data(contentsOf: metaURL),
       let meta = try? JSONDecoder().decode(CardMeta.self, from: data),
       let g = meta.glyph, !g.isEmpty {
        glyph = g
    } else {
        glyph = "🂠"
    }

    let fontSize = size * 1.46
    let attrs: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: fontSize),
        .foregroundColor: cream,
    ]
    let gs = glyph.size(withAttributes: attrs)
    glyph.draw(at: NSPoint(x: (size - gs.width) / 2,
                           y: size - gs.height + size * 0.23),
               withAttributes: attrs)

} else {
    // ── State dir icon: symbol top + bold label bottom ────────────────────────
    let symbol: String
    switch state {
    case "pending":   symbol = "◔"   // partial circle = waiting
    case "running":   symbol = "▶"   // play = active
    case "done":      symbol = "✓"   // check = complete
    case "merged":    symbol = "⤴"   // arrow up-right = landed
    case "failed":    symbol = "✗"   // cross = error
    case "drafts":    symbol = "✎"   // pencil = draft
    case "templates": symbol = "⬡"   // hexagon = blueprint
    default:          symbol = "·"
    }

    // Large symbol in upper 60%
    let symSize  = size * 0.48
    let symAttrs: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: symSize, weight: .thin),
        .foregroundColor: cream,
    ]
    let ss = symbol.size(withAttributes: symAttrs)
    symbol.draw(at: NSPoint(x: (size - ss.width) / 2,
                            y: size * 0.38 - ss.height / 2),
                withAttributes: symAttrs)

    // State name in lower third
    let label     = state.uppercased()
    let labelSize = size * 0.13
    let labelAttrs: [NSAttributedString.Key: Any] = [
        .font: NSFont.monospacedSystemFont(ofSize: labelSize, weight: .bold),
        .foregroundColor: cream.withAlphaComponent(0.72),
    ]
    let ls = label.size(withAttributes: labelAttrs)
    label.draw(at: NSPoint(x: (size - ls.width) / 2, y: size * 0.10),
               withAttributes: labelAttrs)
}

image.unlockFocus()

// MARK: - Set icon

let ok = NSWorkspace.shared.setIcon(image, forFile: targetPath, options: [])

// Set Finder tags for Smart Folder queries and automation tools
let tags: [String] = isCard ? [state, "bop"] : [state, "bop", "dir"]
try? (targetURL as NSURL).setResourceValue(tags as NSArray, forKey: .tagNamesKey)

if ok {
    let label = isCard
        ? "\(state)/\(targetURL.lastPathComponent)"
        : "[\(state)/]"
    print("✓ \(label)")
} else {
    fputs("✗ failed: \(targetPath)\n", stderr)
    exit(1)
}
