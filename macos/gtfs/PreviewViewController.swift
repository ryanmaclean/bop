import Cocoa
import Quartz
import SwiftUI

struct JobCardPreview: View {
    var url: URL?
    var metaDict: [String: Any] = [:]
    var logs: String = ""
    
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let url = url {
                HStack {
                    Image(systemName: "terminal.fill").font(.system(size: 24)).foregroundColor(.blue)
                    Text(metaDict["id"] as? String ?? url.lastPathComponent)
                        .font(.title).bold()
                    Spacer()
                    Text((metaDict["stage"] as? String ?? "UNKNOWN").uppercased())
                        .font(.caption).bold()
                        .padding(.horizontal, 10).padding(.vertical, 4)
                        .background(Color.blue.opacity(0.15))
                        .foregroundColor(.blue)
                        .cornerRadius(6)
                }
                Divider()
                
                HStack(spacing: 40) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Agent").font(.caption).foregroundColor(.secondary)
                        Text(metaDict["agent_type"] as? String ?? "N/A").font(.body)
                    }
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Priority").font(.caption).foregroundColor(.secondary)
                        Text("\(metaDict["priority"] ?? "N/A")").font(.body)
                    }
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Created").font(.caption).foregroundColor(.secondary)
                        Text(formatDate(metaDict["created"] as? String)).font(.body)
                    }
                }
                
                if let criteria = metaDict["acceptance_criteria"] as? [String], !criteria.isEmpty {
                    Text("Acceptance Criteria").font(.headline).padding(.top, 8)
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(criteria, id: \.self) { c in
                            HStack(alignment: .top) {
                                Text("•").foregroundColor(.secondary)
                                Text(c)
                            }
                        }
                    }
                }
                
                if !logs.isEmpty {
                    Text("Recent Output").font(.headline).padding(.top, 8)
                    ScrollView {
                        Text(logs)
                            .font(.system(size: 11, design: .monospaced))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(10)
                    .background(Color(NSColor.textBackgroundColor))
                    .cornerRadius(6)
                }
                Spacer(minLength: 0)
            } else {
                Text("Loading...").foregroundColor(.secondary)
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(NSColor.windowBackgroundColor))
    }
    
    func formatDate(_ isoString: String?) -> String {
        guard let str = isoString else { return "Unknown" }
        let formatter = ISO8601DateFormatter()
        // Provide fractional seconds options since Python/Rust often adds them
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        var date = formatter.date(from: str)
        if date == nil {
            let backupFormatter = ISO8601DateFormatter()
            date = backupFormatter.date(from: str)
        }
        if let d = date {
            let display = DateFormatter()
            display.dateStyle = .medium
            display.timeStyle = .short
            return display.string(from: d)
        }
        return String(str.prefix(10)) // fallback to just the YYYY-MM-DD
    }
}

@objc(PreviewViewController)
class PreviewViewController: NSViewController, QLPreviewingController {
    
    var hostingView: NSHostingView<JobCardPreview>!
    
    override var nibName: NSNib.Name? { nil }

    override func loadView() {
        hostingView = NSHostingView(rootView: JobCardPreview())
        self.view = hostingView
    }

    func preparePreviewOfFile(at url: URL, completionHandler handler: @escaping (Error?) -> Void) {
        let metaUrl = url.appendingPathComponent("meta.json")
        var metaDict: [String: Any] = [:]
        
        if let data = try? Data(contentsOf: metaUrl),
           let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            metaDict = json
        }
        
        var logs = ""
        let logUrl = url.appendingPathComponent("logs/stdout.log")
        if let logData = try? Data(contentsOf: logUrl),
           let logText = String(data: logData, encoding: .utf8) {
            logs = logText.components(separatedBy: .newlines).suffix(25).joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        }

        DispatchQueue.main.async {
            self.hostingView.rootView = JobCardPreview(url: url, metaDict: metaDict, logs: logs)
            self.preferredContentSize = NSSize(width: 550, height: 650)
            handler(nil)
        }
    }
}
