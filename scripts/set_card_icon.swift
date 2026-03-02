#!/usr/bin/env swift
// set_card_icon.swift <card.jobcard dir>
//
// Sets a custom Finder icon on a .jobcard bundle.
// Icon = rounded-square with stage colour + glyph centred.
//
// State is derived from the parent directory name:
//   pending → blue-gray  running → amber  done → green
//   merged  → violet     failed  → red    (other) → neutral
//
// Usage:
//   swift scripts/set_card_icon.swift .cards/running/my-task.jobcard
//   # or batch via macos_cards_maintenance.zsh

import AppKit
import Foundation

// MARK: - Args

guard CommandLine.arguments.count == 2 else {
    fputs("usage: set_card_icon.swift <card.jobcard>\n", stderr)
    exit(2)
}

let cardPath = (CommandLine.arguments[1] as NSString).expandingTildeInPath
let cardURL  = URL(fileURLWithPath: cardPath)
let metaURL  = cardURL.appendingPathComponent("meta.json")

// MARK: - Meta

struct CardMeta: Decodable {
    let glyph: String?
}

let glyph: String
if let data = try? Data(contentsOf: metaURL),
   let meta = try? JSONDecoder().decode(CardMeta.self, from: data),
   let g = meta.glyph, !g.isEmpty {
    glyph = g
} else {
    glyph = "🃟"   // joker fallback
}

// MARK: - Stage colour from parent dir name

let state = cardURL.deletingLastPathComponent().lastPathComponent

let bgColor: NSColor
switch state {
case "running":
    bgColor = NSColor(calibratedRed: 0.58, green: 0.32, blue: 0.06, alpha: 1) // dark amber
case "done":
    bgColor = NSColor(calibratedRed: 0.07, green: 0.36, blue: 0.17, alpha: 1) // forest green
case "merged":
    bgColor = NSColor(calibratedRed: 0.30, green: 0.13, blue: 0.58, alpha: 1) // deep violet
case "failed":
    bgColor = NSColor(calibratedRed: 0.52, green: 0.09, blue: 0.09, alpha: 1) // dark crimson
case "pending":
    bgColor = NSColor(calibratedRed: 0.14, green: 0.22, blue: 0.38, alpha: 1) // midnight blue
default:
    bgColor = NSColor(calibratedRed: 0.18, green: 0.18, blue: 0.22, alpha: 1) // charcoal
}

// Lighter top edge for subtle gradient depth
let bgLight = bgColor.highlight(withLevel: 0.22) ?? bgColor

// MARK: - Render

let size = CGFloat(512)
let image = NSImage(size: NSSize(width: size, height: size))

image.lockFocus()

guard let ctx = NSGraphicsContext.current else {
    fputs("no graphics context\n", stderr)
    exit(1)
}
ctx.imageInterpolation = .high

let rect   = NSRect(x: 0, y: 0, width: size, height: size)
let radius = size * 0.22   // rounded square, iOS-app-icon style

// Background gradient
let path = NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius)
path.addClip()

let gradient = NSGradient(colors: [bgLight, bgColor], atLocations: [0, 1],
                          colorSpace: .deviceRGB)!
gradient.draw(in: rect, angle: -60)

// Subtle inner shadow ring for depth
let ring = NSBezierPath(roundedRect: rect.insetBy(dx: 2, dy: 2), xRadius: radius - 2, yRadius: radius - 2)
NSColor(white: 1, alpha: 0.15).setStroke()
ring.lineWidth = 4
ring.stroke()

// Glyph — wider than the icon so only the top face shows (fan-of-cards effect).
// The rounded-rect clip cuts off the bottom naturally.
let fontSize = size * 1.46
let glyphColor = NSColor(calibratedRed: 1.0, green: 0.97, blue: 0.88, alpha: 0.96)
let attrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: fontSize),
    .foregroundColor: glyphColor,
]
let glyphSize = glyph.size(withAttributes: attrs)
let glyphOrigin = NSPoint(
    x: (size - glyphSize.width) / 2,           // centred horizontally
    y: size - glyphSize.height + size * 0.23    // card sits just proud of centre
)
glyph.draw(at: glyphOrigin, withAttributes: attrs)

image.unlockFocus()

// MARK: - Set icon on bundle

let ok = NSWorkspace.shared.setIcon(image, forFile: cardPath, options: [])
if ok {
    print("✓ icon set: \(state)/\(cardURL.lastPathComponent)  \(glyph)")
} else {
    fputs("✗ failed to set icon (check path exists): \(cardPath)\n", stderr)
    exit(1)
}
