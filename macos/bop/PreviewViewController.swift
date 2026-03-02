import Cocoa
import Quartz
import SwiftUI

// MARK: - Codable model

fileprivate struct JobCardMeta: Codable {
    let id: String
    let title: String?
    let description: String?
    let stage: String
    let priority: Int?
    let created: String?
    let labels: [MetaLabel]?
    let progress: Int?
    let subtasks: [MetaSubtask]?
    let stages: [String: MetaStageRecord]?
    let glyph: String?
    let acceptanceCriteria: [String]?
    let zellijSession: String?
    let zellijPane: String?

    enum CodingKeys: String, CodingKey {
        case id, title, description, stage, priority, created, labels, progress, subtasks, stages
        case glyph
        case acceptanceCriteria = "acceptance_criteria"
        case zellijSession = "zellij_session"
        case zellijPane = "zellij_pane"
    }
}

private struct MetaLabel: Codable {
    let name: String
    let kind: String?
}

private struct MetaSubtask: Codable {
    let id: String
    let title: String
    let done: Bool
}

private struct MetaStageRecord: Codable {
    let status: String
    let agent: String?
}

// MARK: - Palette

private extension Color {
    static let appBg         = Color(red: 0.09, green: 0.07, blue: 0.14) // Darker background outside card
    static let cardBg        = Color(red: 0.16, green: 0.13, blue: 0.27)
    static let cardBorder    = Color(red: 0.30, green: 0.22, blue: 0.45)
    static let textPrimary   = Color.white
    static let textSecondary = Color(red: 0.82, green: 0.73, blue: 0.94)
    static let textMuted     = Color(red: 0.60, green: 0.50, blue: 0.75)
    
    static let pillPurple    = Color(red: 0.65, green: 0.45, blue: 0.95)
    static let pillPurpleBg  = Color(red: 0.25, green: 0.18, blue: 0.45)
    
    static let pillGold      = Color(red: 0.95, green: 0.80, blue: 0.25)
    static let pillGoldBg    = Color(red: 0.35, green: 0.25, blue: 0.15)
    
    static let stageActive   = Color(red: 0.10, green: 0.80, blue: 0.90) // Cyan checkmark
    static let stageActiveBg = Color(red: 0.15, green: 0.35, blue: 0.45)
    static let stagePending  = Color(red: 0.60, green: 0.45, blue: 0.85) // Light purple text
    static let stagePendingBg = Color(red: 0.22, green: 0.16, blue: 0.36) // Dark purple bg
    
    static let stopBg        = Color(red: 0.98, green: 0.45, blue: 0.50) // Coral
    static let stopText      = Color.black
    static let attachBg      = Color(red: 0.10, green: 0.72, blue: 0.42) // Green
    static let attachText    = Color.black
    static let tailBg        = Color(red: 0.18, green: 0.42, blue: 0.82) // Blue
    static let tailText      = Color.white
    
    static let barEmpty      = Color(red: 0.25, green: 0.16, blue: 0.42)
    static let barFill       = Color(red: 0.85, green: 0.45, blue: 0.95) // Pinkish purple
}

// MARK: - Stage display names

private let stageOrder: [(key: String, label: String)] = [
    ("spec",      "Spec"),
    ("plan",      "Plan"),
    ("implement", "Code"),
    ("qa",        "QA"),
]

// MARK: - Label pill

private struct LabelPill: View {
    let label: MetaLabel

    var color: Color { .pillPurple }
    
    var icon: String {
        switch label.name.lowercased() {
        case "coding": return "arrow.triangle.2.circlepath"
        case "performance": return "gauge.medium"
        case "bug": return "ladybug.fill"
        default: return "tag"
        }
    }

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: icon).font(.system(size: 11, weight: .medium))
            Text(label.name).font(.system(size: 13, weight: .medium))
        }
        .foregroundColor(color)
        .padding(.horizontal, 12).padding(.vertical, 6)
        .background(Color.pillPurpleBg)
        .clipShape(Capsule())
        .overlay(Capsule().stroke(color.opacity(0.6), lineWidth: 1))
    }
}

// MARK: - Stage pipeline

private struct StagePipeline: View {
    let currentStage: String
    let stages: [String: MetaStageRecord]?

    private func status(_ key: String) -> String {
        stages?[key]?.status ?? "pending"
    }

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(stageOrder.enumerated()), id: \.offset) { idx, pair in
                let st = status(pair.key)
                let isCurrent = pair.key == currentStage
                let isDone    = st == "done" || (idx < (stageOrder.firstIndex(where: { $0.key == currentStage }) ?? 0))

                HStack(spacing: 6) {
                    if isDone {
                        Image(systemName: "checkmark")
                            .font(.system(size: 11, weight: .bold))
                            .foregroundColor(.stageActive)
                    }
                    Text(pair.label)
                        .font(.system(size: 13, weight: .medium))
                        .foregroundColor(isDone ? .stageActive : (isCurrent ? .stagePending : .textSecondary))
                }
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(isDone ? Color.stageActiveBg.opacity(0.5) : Color.stagePendingBg)
                .clipShape(RoundedRectangle(cornerRadius: 6))

                if idx < stageOrder.count - 1 {
                    Text("—")
                        .font(.system(size: 14, weight: .bold))
                        .foregroundColor(.cardBorder)
                        .padding(.horizontal, 8)
                }
            }
        }
    }
}

// MARK: - Relative time

private func relativeTime(_ iso: String?) -> String {
    guard let s = iso else { return "" }
    let f1 = ISO8601DateFormatter()
    f1.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    let f2 = ISO8601DateFormatter()
    guard let d = f1.date(from: s) ?? f2.date(from: s) else { return String(s.prefix(10)) }
    let dt = Int(-d.timeIntervalSinceNow)
    if dt < 60    { return "just now" }
    if dt < 3600  { return "\(dt / 60)m ago" }
    if dt < 86400 { return "\(dt / 3600)h ago" }
    return "\(dt / 86400)d ago"
}

// MARK: - Card view

fileprivate enum CardTab: String, CaseIterable {
    case overview = "Overview"
    case subtasks = "Subtasks"
    case logs = "Logs"
    case files = "Files"
}

fileprivate struct JobCardPreview: View {
    var url: URL?
    var meta: JobCardMeta?
    var logs: String = ""
    var bundleFiles: [String] = []

    @State private var selectedTab: CardTab = .overview

    private var displayTitle: String {
        meta?.title ?? meta?.id ?? url?.lastPathComponent ?? "JobCard"
    }

    private var isRunning: Bool {
        guard let m = meta else { return false }
        return m.stages?[m.stage]?.status == "running"
    }
    
    private func priorityText(_ p: Int) -> String {
        switch p {
        case 1: return "Critical"
        case 2: return "High Impact"
        case 3: return "Medium Impact"
        default: return "Low Priority"
        }
    }

    private func tabName(for tab: CardTab) -> String {
        if tab == .subtasks, let count = meta?.subtasks?.count {
            return "Subtasks (\(count))"
        }
        return tab.rawValue
    }

    var body: some View {
        ZStack {
            Color.appBg.ignoresSafeArea()
            if let m = meta {
                cardBody(m)
            } else {
                Text("Loading…").foregroundColor(.textSecondary)
            }
        }
    }

    @ViewBuilder
    private func cardBody(_ m: JobCardMeta) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            
            // Header: Checkbox + Title
            HStack(alignment: .top, spacing: 16) {
                if let g = m.glyph {
                    Text(g)
                        .font(.system(size: 32))
                        .frame(width: 36, height: 36)
                        .padding(.top, 2)
                } else {
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Color.cardBorder, lineWidth: 2)
                        .frame(width: 24, height: 24)
                        .padding(.top, 2)
                }
                
                VStack(alignment: .leading, spacing: 10) {
                    Text(displayTitle)
                        .font(.system(size: 22, weight: .bold))
                        .foregroundColor(.textPrimary)
                        .lineLimit(2)
                    
                    HStack(spacing: 12) {
                        Text(m.id)
                            .font(.system(size: 13, design: .monospaced))
                            .foregroundColor(.textPrimary)
                            .padding(.horizontal, 8).padding(.vertical, 4)
                            .background(Color.black.opacity(0.3))
                            .clipShape(RoundedRectangle(cornerRadius: 4))
                        
                        Text(m.stage.capitalized)
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundColor(.stageActive)
                        
                        if let prog = m.progress {
                            Text("\(prog)%")
                                .font(.system(size: 13, weight: .bold))
                                .foregroundColor(.textPrimary)
                        }
                    }
                }
            }
            .padding(.horizontal, 24)
            .padding(.top, 24)
            .padding(.bottom, 20)
            
            // Progress Bar
            let prog = m.progress ?? 0
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Rectangle().fill(Color.barEmpty)
                    Rectangle()
                        .fill(Color.barFill)
                        .frame(width: max(0, geo.size.width * CGFloat(prog) / 100))
                }
            }
            .frame(height: 4)
            .padding(.horizontal, 24)
            .padding(.bottom, 20)
            
            // Tabs
            HStack(spacing: 24) {
                ForEach(CardTab.allCases, id: \.self) { tab in
                    VStack(spacing: 6) {
                        Text(tabName(for: tab))
                            .font(.system(size: 14, weight: selectedTab == tab ? .bold : .medium))
                            .foregroundColor(selectedTab == tab ? .textPrimary : .textMuted)
                        Rectangle()
                            .fill(selectedTab == tab ? Color.barFill : Color.clear)
                            .frame(height: 2)
                    }
                    .onTapGesture {
                        selectedTab = tab
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
            }
            .padding(.horizontal, 24)
            
            Divider().background(Color.cardBorder)
            
            // Tab Content
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    switch selectedTab {
                    case .overview:
                        overviewTab(m)
                    case .subtasks:
                        subtasksTab(m)
                    case .logs:
                        logsTab()
                    case .files:
                        filesTab()
                    }
                }
                .padding(24)
            }
            
            Divider().background(Color.cardBorder)
            
            // Footer
            HStack(spacing: 8) {
                Image(systemName: "clock")
                    .font(.system(size: 13))
                    .foregroundColor(.textMuted)
                Text("Created \(relativeTime(m.created))")
                    .font(.system(size: 13))
                    .foregroundColor(.textMuted)
                
                Spacer()
                
                if !logs.isEmpty,
                   let tailURL = URL(string: "bop://card/\(m.id)/logs") {
                    Link(destination: tailURL) {
                        HStack(spacing: 6) {
                            Image(systemName: "scroll")
                                .font(.system(size: 11))
                            Text("Tail")
                                .font(.system(size: 13, weight: .bold))
                        }
                        .foregroundColor(.tailText)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(Color.tailBg)
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                    }
                    .help("Live tail: bop logs \(m.id) --follow")
                }
                if isRunning, let session = m.zellijSession,
                   let url = URL(string: "bop://card/\(m.id)/session") {
                    Link(destination: url) {
                        HStack(spacing: 6) {
                            Image(systemName: "terminal")
                                .font(.system(size: 11))
                            Text("Attach")
                                .font(.system(size: 13, weight: .bold))
                        }
                        .foregroundColor(.attachText)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(Color.attachBg)
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                    }
                    .help("Attach to zellij session: \(session)")
                }
                if isRunning {
                    HStack(spacing: 6) {
                        Image(systemName: "square.fill")
                            .font(.system(size: 11))
                        Text("Stop")
                            .font(.system(size: 13, weight: .bold))
                    }
                    .foregroundColor(.stopText)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(Color.stopBg)
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                }
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 16)
        }
        .background(
            RoundedRectangle(cornerRadius: 16)
                .fill(Color.cardBg)
                .overlay(RoundedRectangle(cornerRadius: 16).stroke(Color.cardBorder, lineWidth: 1))
        )
        .padding(16)
    }
    
    @ViewBuilder
    private func overviewTab(_ m: JobCardMeta) -> some View {
        VStack(alignment: .leading, spacing: 24) {
            // Labels & Priority
            HStack(spacing: 12) {
                if let priority = m.priority {
                    Text(priorityText(priority))
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundColor(.pillGold)
                        .padding(.horizontal, 12).padding(.vertical, 6)
                        .background(Color.pillGoldBg)
                        .clipShape(Capsule())
                        .overlay(Capsule().stroke(Color.pillGold.opacity(0.4), lineWidth: 1))
                }
                
                if let labels = m.labels?.filter({ $0.kind != "priority" }), !labels.isEmpty {
                    ForEach(labels, id: \.name) { LabelPill(label: $0) }
                }
            }
            
            if let desc = m.description {
                Text(desc)
                    .font(.system(size: 15))
                    .foregroundColor(.textSecondary)
                    .lineSpacing(4)
            }
            
            StagePipeline(currentStage: m.stage, stages: m.stages)
            
            if let criteria = m.acceptanceCriteria, !criteria.isEmpty {
                VStack(alignment: .leading, spacing: 12) {
                    Text("ACCEPTANCE CRITERIA")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundColor(.textMuted)
                    
                    ForEach(criteria, id: \.self) { c in
                        HStack(alignment: .top, spacing: 10) {
                            Image(systemName: "checkmark.circle")
                                .foregroundColor(.stageActive)
                                .font(.system(size: 14))
                                .padding(.top, 2)
                            Text(c)
                                .font(.system(size: 14, design: .monospaced))
                                .foregroundColor(.textSecondary)
                        }
                    }
                }
                .padding()
                .background(Color.black.opacity(0.2))
                .clipShape(RoundedRectangle(cornerRadius: 8))
            }
        }
    }
    
    @ViewBuilder
    private func subtasksTab(_ m: JobCardMeta) -> some View {
        if let subs = m.subtasks, !subs.isEmpty {
            VStack(alignment: .leading, spacing: 16) {
                let doneCount = subs.filter(\.done).count
                HStack {
                    Text("\(doneCount) of \(subs.count) completed")
                        .font(.system(size: 14, weight: .medium))
                        .foregroundColor(.textSecondary)
                    Spacer()
                    Text("\(Int(Double(doneCount) / Double(subs.count) * 100))%")
                        .font(.system(size: 14, weight: .medium))
                        .foregroundColor(.textSecondary)
                }
                
                ForEach(Array(subs.enumerated()), id: \.offset) { idx, st in
                    HStack(alignment: .top, spacing: 16) {
                        Image(systemName: st.done ? "checkmark.circle.fill" : "circle")
                            .foregroundColor(st.done ? .stageActive : .textMuted)
                            .font(.system(size: 18))
                        
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Text("#\(idx + 1)")
                                    .font(.system(size: 12, weight: .bold))
                                    .foregroundColor(.pillPurple)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2)
                                    .background(Color.pillPurpleBg)
                                    .clipShape(Capsule())
                                
                                Text(st.title)
                                    .font(.system(size: 15, weight: .medium))
                                    .foregroundColor(st.done ? .textSecondary : .textPrimary)
                            }
                            
                            Text(st.title)
                                .font(.system(size: 13))
                                .foregroundColor(.textMuted)
                        }
                    }
                    .padding()
                    .background(Color.black.opacity(0.15))
                    .clipShape(RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.white.opacity(0.05), lineWidth: 1))
                }
            }
        } else {
            Text("No subtasks defined.")
                .foregroundColor(.textMuted)
        }
    }
    
    @ViewBuilder
    private func logsTab() -> some View {
        if !logs.isEmpty {
            Text(logs)
                .font(.system(size: 12, design: .monospaced))
                .foregroundColor(Color.white.opacity(0.8))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(16)
                .background(Color.black.opacity(0.4))
                .clipShape(RoundedRectangle(cornerRadius: 8))
        } else {
            Text("No logs available.")
                .foregroundColor(.textMuted)
        }
    }
    
    @ViewBuilder
    private func filesTab() -> some View {
        if !bundleFiles.isEmpty {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(bundleFiles, id: \.self) { file in
                    HStack(spacing: 12) {
                        Image(systemName: fileIcon(for: file))
                            .foregroundColor(.pillPurple)
                            .font(.system(size: 16))
                            .frame(width: 20)
                        
                        Text(file)
                            .font(.system(size: 14, design: .monospaced))
                            .foregroundColor(.textPrimary)
                        
                        Spacer()
                    }
                    .padding(.vertical, 8)
                    .padding(.horizontal, 16)
                    .background(Color.black.opacity(0.2))
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                }
            }
        } else {
            Text("No files in bundle.")
                .foregroundColor(.textMuted)
        }
    }
    
    private func fileIcon(for name: String) -> String {
        if name.hasSuffix(".md") { return "doc.text" }
        if name.hasSuffix(".json") { return "curlybraces" }
        if name.hasSuffix(".log") { return "terminal" }
        if name.hasSuffix(".toml") || name.hasSuffix(".yaml") { return "gearshape" }
        if name.hasSuffix(".rs") || name.hasSuffix(".swift") { return "chevron.left.forwardslash.chevron.right" }
        return "doc"
    }
}

// MARK: - QL controller

@objc(PreviewViewController)
class PreviewViewController: NSViewController, QLPreviewingController {

    private var hostingView: NSHostingView<JobCardPreview>!

    override var nibName: NSNib.Name? { nil }

    override func loadView() {
        hostingView = NSHostingView(rootView: JobCardPreview())
        self.view = hostingView
    }

    func preparePreviewOfFile(at url: URL, completionHandler handler: @escaping (Error?) -> Void) {
        let metaUrl = url.appendingPathComponent("meta.json")
        var meta: JobCardMeta?

        if let data = try? Data(contentsOf: metaUrl) {
            meta = try? JSONDecoder().decode(JobCardMeta.self, from: data)
        }
        if meta == nil {
            var coordErr: NSError?
            NSFileCoordinator().coordinate(readingItemAt: metaUrl, options: .withoutChanges, error: &coordErr) { u in
                if let data = try? Data(contentsOf: u) {
                    meta = try? JSONDecoder().decode(JobCardMeta.self, from: data)
                }
            }
        }

        var logs = ""
        let logUrl = url.appendingPathComponent("logs/stdout.log")
        if let data = try? Data(contentsOf: logUrl),
           let text = String(data: data, encoding: .utf8) {
            logs = text.components(separatedBy: .newlines).suffix(100)
                .joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }

        // Enumerate bundle files (top-level only, skip logs/ and output/ dirs)
        var bundleFiles: [String] = []
        if let items = try? FileManager.default.contentsOfDirectory(atPath: url.path) {
            bundleFiles = items
                .filter { !$0.hasPrefix(".") && $0 != "logs" && $0 != "output" && $0 != "worktree" }
                .sorted()
        }

        DispatchQueue.main.async {
            self.hostingView.rootView = JobCardPreview(url: url, meta: meta, logs: logs, bundleFiles: bundleFiles)
            self.preferredContentSize = NSSize(width: 800, height: 750)
            handler(nil)
        }
    }
}
