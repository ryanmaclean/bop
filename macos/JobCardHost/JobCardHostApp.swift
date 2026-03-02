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

    private func handleBopURL(_ url: URL) {
        // bop://card/<id>/session  → zellij attach bop-<id>
        guard url.scheme == "bop",
              url.host == "card",
              let id = url.pathComponents.dropFirst().first
        else { return }

        let action = url.pathComponents.last ?? "session"

        switch action {
        case "session":
            let session = "bop-\(id)"
            // Resumable: attach existing or create new
            runInTerminal("zellij attach '\(session)' 2>/dev/null || zellij -s '\(session)'")
        case "spec":
            // Open spec.md from the card in the default editor
            let candidates = [
                FileManager.default.homeDirectoryForCurrentUser
                    .appendingPathComponent("bop/.cards"),
            ]
            for base in candidates {
                for state in ["running", "pending", "done", "merged", "failed"] {
                    let spec = base.appendingPathComponent("\(state)/\(id).jobcard/spec.md")
                    if FileManager.default.fileExists(atPath: spec.path) {
                        NSWorkspace.shared.open(spec)
                        return
                    }
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
