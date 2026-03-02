#!/usr/bin/env swift
import AppKit
import Foundation

struct CardMeta: Decodable {
    let id: String
    let title: String?
    let stage: String?
    let glyph: String?
    let priority: Int?
}

func usage() {
    fputs("usage: render_card_thumbnail.swift <meta.json> <out.png>\n", stderr)
}

guard CommandLine.arguments.count == 3 else {
    usage()
    exit(2)
}

let metaPath = URL(fileURLWithPath: CommandLine.arguments[1])
let outPath = URL(fileURLWithPath: CommandLine.arguments[2])

let data: Data
do {
    data = try Data(contentsOf: metaPath)
} catch {
    fputs("failed to read meta: \(error)\n", stderr)
    exit(1)
}

let meta: CardMeta
do {
    meta = try JSONDecoder().decode(CardMeta.self, from: data)
} catch {
    fputs("failed to decode meta: \(error)\n", stderr)
    exit(1)
}

let width = 512
let height = 512
guard let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: width,
    pixelsHigh: height,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
) else {
    fputs("failed to create bitmap image rep\n", stderr)
    exit(1)
}

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let rect = NSRect(x: 0, y: 0, width: CGFloat(width), height: CGFloat(height))

let gradient = NSGradient(
    colors: [
        NSColor(calibratedRed: 0.11, green: 0.08, blue: 0.20, alpha: 1.0),
        NSColor(calibratedRed: 0.06, green: 0.06, blue: 0.11, alpha: 1.0),
    ]
)!
gradient.draw(in: rect, angle: 120)

let borderRect = rect.insetBy(dx: 22, dy: 22)
let border = NSBezierPath(roundedRect: borderRect, xRadius: 20, yRadius: 20)
NSColor(calibratedWhite: 1.0, alpha: 0.16).setStroke()
border.lineWidth = 2
border.stroke()

let glyph = (meta.glyph?.isEmpty == false) ? meta.glyph! : "CARD"
let glyphAttrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 120, weight: .bold),
    .foregroundColor: NSColor(calibratedWhite: 1.0, alpha: 0.95),
]
let glyphSize = glyph.size(withAttributes: glyphAttrs)
glyph.draw(
    at: NSPoint(
        x: rect.midX - glyphSize.width / 2,
        y: rect.midY + 90 - glyphSize.height / 2
    ),
    withAttributes: glyphAttrs
)

let title = (meta.title?.isEmpty == false) ? meta.title! : meta.id
let titleAttrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 34, weight: .semibold),
    .foregroundColor: NSColor(calibratedWhite: 1.0, alpha: 0.96),
]
let titleRect = NSRect(x: 40, y: 180, width: 432, height: 84)
title.draw(in: titleRect, withAttributes: titleAttrs)

let stage = (meta.stage ?? "unknown").uppercased()
let priority = meta.priority.map { "P\($0)" } ?? "P?"
let subtitle = "\(stage)  •  \(priority)"
let subtitleAttrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.monospacedSystemFont(ofSize: 20, weight: .medium),
    .foregroundColor: NSColor(calibratedRed: 0.77, green: 0.67, blue: 1.0, alpha: 0.94),
]
subtitle.draw(in: NSRect(x: 40, y: 120, width: 432, height: 30), withAttributes: subtitleAttrs)

NSGraphicsContext.restoreGraphicsState()

guard let pngData = rep.representation(using: .png, properties: [:]) else {
    fputs("failed to encode png\n", stderr)
    exit(1)
}

do {
    try FileManager.default.createDirectory(
        at: outPath.deletingLastPathComponent(),
        withIntermediateDirectories: true
    )
    try pngData.write(to: outPath)
} catch {
    fputs("failed to write output: \(error)\n", stderr)
    exit(1)
}
