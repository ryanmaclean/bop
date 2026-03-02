//
//  JobCardHostApp.swift
//  JobCardHost
//

import SwiftUI
import AppKit

@main
struct JobCardHostApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate

    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

// MARK: - bop:// URL scheme handler

class AppDelegate: NSObject, NSApplicationDelegate {
    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls { handleBopURL(url) }
    }

    private struct SessionMeta: Decodable {
        let zellijSession: String?
        enum CodingKeys: String, CodingKey {
            case zellijSession = "zellij_session"
        }
    }

    private func cardRootCandidates() -> [URL] {
        var roots: [URL] = []
        let env = ProcessInfo.processInfo.environment
        if let cards = env["BOP_CARDS_DIR"], !cards.isEmpty {
            roots.append(URL(fileURLWithPath: cards))
        }
        roots.append(
            FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent("bop/.cards")
        )
        return roots.filter { FileManager.default.fileExists(atPath: $0.path) }
    }

    private func findCardDirectory(id: String) -> URL? {
        let suffix = "-\(id).jobcard"
        let exact = "\(id).jobcard"
        let fm = FileManager.default

        for base in cardRootCandidates() {
            for state in ["running", "pending", "done", "merged", "failed"] {
                let stateDir = base.appendingPathComponent(state)
                let exactURL = stateDir.appendingPathComponent(exact)
                if fm.fileExists(atPath: exactURL.path) {
                    return exactURL
                }
                guard let entries = try? fm.contentsOfDirectory(
                    at: stateDir,
                    includingPropertiesForKeys: nil
                ) else { continue }
                if let match = entries.first(where: { $0.lastPathComponent.hasSuffix(suffix) }) {
                    return match
                }
            }
        }
        return nil
    }

    private func resolveZellijSession(id: String) -> String? {
        guard let cardDir = findCardDirectory(id: id) else { return nil }
        let metaURL = cardDir.appendingPathComponent("meta.json")
        guard
            let data = try? Data(contentsOf: metaURL),
            let meta = try? JSONDecoder().decode(SessionMeta.self, from: data),
            let session = meta.zellijSession?.trimmingCharacters(in: .whitespacesAndNewlines),
            !session.isEmpty
        else { return nil }
        return session
    }

    private func shQuote(_ text: String) -> String {
        let escaped = text.replacingOccurrences(of: "'", with: "'\"'\"'")
        return "'\(escaped)'"
    }

    private func handleBopURL(_ url: URL) {
        // bop://card/<id>/session  → zellij attach bop-<id>
        guard url.scheme == "bop",
              url.host == "card"
        else { return }

        let parts = url.pathComponents.filter { $0 != "/" }
        guard let rawID = parts.first else { return }
        let id = rawID.removingPercentEncoding ?? rawID
        let action = (parts.count >= 2 ? parts[1] : "session").lowercased()
        let qid = shQuote(id)

        switch action {
        case "session":
            let session = resolveZellijSession(id: id) ?? "bop-\(id)"
            let qsession = shQuote(session)
            // Resumable: attach existing or create new
            runInTerminal("zellij attach \(qsession) 2>/dev/null || zellij -s \(qsession)")
        case "tail":
            runInTerminal("bop logs \(qid) --follow")
        case "logs":
            runInTerminal("bop logs \(qid)")
        case "stop":
            runInTerminal("bop kill \(qid)")
        case "spec":
            // Open spec.md from the card in the default editor
            if let cardDir = findCardDirectory(id: id) {
                let spec = cardDir.appendingPathComponent("spec.md")
                if FileManager.default.fileExists(atPath: spec.path) {
                    NSWorkspace.shared.open(spec)
                }
            }
        default:
            break
        }
    }

    private func runInTerminal(_ script: String) {
        // Try Ghostty first, then Terminal.app
        let ghostty = URL(fileURLWithPath: "/Applications/Ghostty.app")
        if FileManager.default.fileExists(atPath: ghostty.path) {
            let cfg = NSWorkspace.OpenConfiguration()
            cfg.arguments = ["--command=zsh -c \"\(script)\""]
            NSWorkspace.shared.openApplication(at: ghostty, configuration: cfg)
            return
        }
        // Fallback: Terminal.app via open
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        p.arguments = ["-a", "Terminal", "--args", "zsh", "-c", script]
        try? p.run()
    }
}
