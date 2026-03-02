import Cocoa
import Quartz
import SwiftUI

// MARK: - Codable model

fileprivate struct JobCardMeta: Codable {
    let id: String
    let title: String?
    let description: String?
    let stage: String
    let workflowMode: String?
    let stepIndex: Int?
    let stageChain: [String]?
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
        case workflowMode = "workflow_mode"
        case stepIndex = "step_index"
        case stageChain = "stage_chain"
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
    let provider: String?
    let durationS: Int?
    let started: String?

    enum CodingKeys: String, CodingKey {
        case status, agent, provider, started
        case durationS = "duration_s"
    }
}

private struct RoadmapSnapshot {
    let statusCounts: [String: Int]
    let priorityCounts: [String: Int]
    let phaseCount: Int
    let featureCount: Int
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

private let defaultStageOrder: [(key: String, label: String)] = [
    ("spec",      "Spec"),
    ("plan",      "Plan"),
    ("implement", "Code"),
    ("qa",        "QA"),
]

private let roadmapStageOrder: [(key: String, label: String)] = [
    ("analyze",  "Analyze"),
    ("discover", "Discover"),
    ("generate", "Generate"),
    ("qa",       "QA"),
]

private let featureLifecycleStageOrder: [(key: String, label: String)] = [
    ("under_review", "Under Review"),
    ("planned", "Planned"),
    ("in_progress", "In Progress"),
    ("done", "Done"),
]

private func stageDisplayName(_ key: String) -> String {
    switch key.lowercased() {
    case "spec": return "Spec"
    case "plan": return "Plan"
    case "implement": return "Code"
    case "qa": return "QA"
    case "analyze": return "Analyze"
    case "discover": return "Discover"
    case "generate": return "Generate"
    case "roadmap": return "Roadmap"
    case "under_review": return "Under Review"
    case "planned": return "Planned"
    case "in_progress": return "In Progress"
    case "done": return "Done"
    default:
        let lower = key.lowercased()
        guard let first = lower.first else { return key }
        return String(first).uppercased() + lower.dropFirst()
    }
}

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
    let displayStages: [(key: String, label: String)]

    private func status(_ key: String) -> String {
        stages?[key]?.status ?? "pending"
    }

    private var currentIndex: Int {
        displayStages.firstIndex(where: { $0.key == currentStage }) ?? 0
    }

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(displayStages.enumerated()), id: \.offset) { idx, pair in
                let st = status(pair.key)
                let isCurrent = pair.key == currentStage
                let isDone = st == "done" || (idx < currentIndex)

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

                if idx < displayStages.count - 1 {
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

private func parseISODate(_ iso: String?) -> Date? {
    guard let s = iso else { return nil }
    let f1 = ISO8601DateFormatter()
    f1.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    let f2 = ISO8601DateFormatter()
    return f1.date(from: s) ?? f2.date(from: s)
}

private func relativeTimeFromDate(_ date: Date?) -> String {
    guard let d = date else { return "unknown" }
    let dt = Int(-d.timeIntervalSinceNow)
    if dt < 60 { return "just now" }
    if dt < 3600 { return "\(dt / 60)m ago" }
    if dt < 86400 { return "\(dt / 3600)h ago" }
    return "\(dt / 86400)d ago"
}

private func formatElapsed(_ seconds: Int) -> String {
    let s = max(0, seconds)
    let h = s / 3600
    let m = (s % 3600) / 60
    let sec = s % 60
    if h > 0 {
        return String(format: "%d:%02d:%02d", h, m, sec)
    }
    return String(format: "%d:%02d", m, sec)
}

private func detectToolTag(in text: String) -> String? {
    let lines = text.split(separator: "\n").map(String.init).reversed()
    for line in lines {
        if let r = line.range(of: #"\[Tool:\s*([^\]]+)\]"#, options: .regularExpression) {
            let payload = String(line[r]).replacingOccurrences(of: "[Tool:", with: "").replacingOccurrences(of: "]", with: "").trimmingCharacters(in: .whitespacesAndNewlines)
            if !payload.isEmpty { return payload }
        }
        if let r = line.range(of: #"functions\.([a-zA-Z0-9_-]+)"#, options: .regularExpression) {
            let raw = String(line[r])
            return raw.replacingOccurrences(of: "functions.", with: "")
        }
    }
    return nil
}

private func normalizeRoadmapStatus(_ raw: String) -> String? {
    let key = raw
        .lowercased()
        .replacingOccurrences(of: "-", with: "_")
        .replacingOccurrences(of: " ", with: "_")
    switch key {
    case "under_review", "review", "underreview":
        return "under_review"
    case "planned", "plan":
        return "planned"
    case "in_progress", "inprogress", "active", "doing":
        return "in_progress"
    case "done", "completed", "complete":
        return "done"
    default:
        return nil
    }
}

private func normalizeRoadmapPriority(_ raw: String) -> String? {
    let key = raw
        .lowercased()
        .replacingOccurrences(of: "-", with: "_")
        .replacingOccurrences(of: " ", with: "_")
    switch key {
    case "must", "must_have", "critical":
        return "must"
    case "should", "should_have", "important":
        return "should"
    case "could", "could_have", "nice_to_have":
        return "could"
    default:
        return nil
    }
}

private func parseRoadmapSnapshot(from cardURL: URL) -> RoadmapSnapshot? {
    let roadmapURL = cardURL.appendingPathComponent("output/roadmap.json")
    guard
        let data = try? Data(contentsOf: roadmapURL),
        let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    else { return nil }

    let featureItems = (obj["features"] as? [Any]) ?? []
    var statusCounts: [String: Int] = [:]
    var priorityCounts: [String: Int] = [:]
    var phaseSet = Set<String>()
    var featureCount = 0

    for item in featureItems {
        guard let feature = item as? [String: Any] else { continue }
        featureCount += 1

        if let rawStatus = feature["status"] as? String,
           let status = normalizeRoadmapStatus(rawStatus) {
            statusCounts[status, default: 0] += 1
        }
        if let rawPriority = feature["priority"] as? String,
           let priority = normalizeRoadmapPriority(rawPriority) {
            priorityCounts[priority, default: 0] += 1
        }

        if let phase = feature["phase"] as? String, !phase.isEmpty {
            phaseSet.insert(phase)
        } else if let phase = feature["phase_id"] as? String, !phase.isEmpty {
            phaseSet.insert(phase)
        } else if let phaseNum = feature["phase"] as? NSNumber {
            phaseSet.insert(phaseNum.stringValue)
        }
    }

    let explicitPhases = (obj["phases"] as? [Any])?.count ?? 0
    let phaseCount = max(explicitPhases, phaseSet.count)

    return RoadmapSnapshot(
        statusCounts: statusCounts,
        priorityCounts: priorityCounts,
        phaseCount: phaseCount,
        featureCount: featureCount
    )
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
    var lastActivityAt: Date? = nil
    var activeTool: String? = nil
    var roadmapSnapshot: RoadmapSnapshot? = nil

    @State private var selectedTab: CardTab = .overview

    private var displayTitle: String {
        meta?.title ?? meta?.id ?? url?.lastPathComponent ?? "JobCard"
    }

    private var cardState: String {
        guard let url else { return "unknown" }
        return url.deletingLastPathComponent().lastPathComponent
    }

    private var isRunning: Bool {
        if cardState == "running" { return true }
        guard let m = meta else { return false }
        return m.stages?[m.stage]?.status == "running"
    }

    private var isDoneLike: Bool {
        cardState == "done" || cardState == "merged"
    }

    private var logsAction: String { isDoneLike ? "logs" : "tail" }

    private var logsURL: URL? {
        guard let id = meta?.id else { return nil }
        return URL(string: "bop://card/\(id)/\(logsAction)")
    }

    private var stopURL: URL? {
        guard isRunning, let id = meta?.id else { return nil }
        return URL(string: "bop://card/\(id)/stop")
    }

    private var logsButtonText: String { isDoneLike ? "Logs" : "Tail" }

    private var logsHelpText: String {
        guard let id = meta?.id else { return "" }
        if isDoneLike {
            return "Open logs: bop logs \(id)"
        }
        return "Live tail: bop logs \(id) --follow"
    }

    private var isRoadmapWorkflow: Bool {
        guard let m = meta else { return false }
        if m.workflowMode?.lowercased() == "roadmap" {
            return true
        }
        return [
            "analyze", "discover", "generate", "roadmap",
            "under_review", "planned", "in_progress", "done",
        ].contains(m.stage.lowercased())
    }

    private func displayStageOrder(for m: JobCardMeta) -> [(key: String, label: String)] {
        if let chain = m.stageChain, !chain.isEmpty {
            return chain.map { (key: $0, label: stageDisplayName($0)) }
        }
        if normalizeRoadmapStatus(m.stage) != nil {
            return featureLifecycleStageOrder
        }
        return isRoadmapWorkflow ? roadmapStageOrder : defaultStageOrder
    }

    private func displayStageName(_ key: String) -> String {
        stageDisplayName(key)
    }

    private func inferredRoadmapProgress(_ m: JobCardMeta) -> Int {
        if let progress = m.progress {
            return progress
        }
        switch m.stage.lowercased() {
        case "analyze": return 20
        case "discover": return 40
        case "generate": return 60
        case "qa": return 85
        case "under_review": return 15
        case "planned": return 35
        case "in_progress": return 65
        case "done": return 100
        default: return 0
        }
    }

    private func elapsedTimeText(_ m: JobCardMeta) -> String {
        let start = parseISODate(m.stages?[m.stage]?.started) ?? parseISODate(m.created)
        guard let start else { return "0:00" }
        return formatElapsed(Int(Date().timeIntervalSince(start)))
    }

    private func lastActivityText() -> String {
        relativeTimeFromDate(lastActivityAt)
    }

    private func statusCount(_ key: String) -> Int {
        roadmapSnapshot?.statusCounts[key] ?? 0
    }

    private func priorityCount(_ key: String) -> Int {
        roadmapSnapshot?.priorityCounts[key] ?? 0
    }

    @ViewBuilder
    private func metricPill(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 12, weight: .semibold))
            .foregroundColor(.textSecondary)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Color.pillPurpleBg.opacity(0.65))
            .clipShape(Capsule())
            .overlay(Capsule().stroke(Color.cardBorder.opacity(0.6), lineWidth: 1))
    }

    private func roadmapStatusTitle(_ m: JobCardMeta) -> String {
        switch m.stage.lowercased() {
        case "analyze": return "Analyzing"
        case "discover": return "Discovering"
        case "generate": return "Generating"
        case "qa": return "Reviewing"
        case "under_review": return "Under Review"
        case "planned": return "Planned"
        case "in_progress": return "In Progress"
        case "done": return "Done"
        default: return displayStageName(m.stage)
        }
    }

    private func roadmapStatusSubtitle(_ m: JobCardMeta) -> String {
        switch m.stage.lowercased() {
        case "analyze":
            return "Analyzing project structure and current state."
        case "discover":
            return "Discovering user needs and competitor signals."
        case "generate":
            return "Generating milestone roadmap and execution plan."
        case "qa":
            return "Verifying roadmap quality and handoff readiness."
        case "under_review":
            return "Reviewing generated features before planning."
        case "planned":
            return "Prioritized and phase-assigned, ready to execute."
        case "in_progress":
            return "Executing selected roadmap features."
        case "done":
            return "Roadmap execution complete for this scope."
        default:
            return "Roadmap workflow in progress."
        }
    }

    private func roadmapStatusIcon(_ m: JobCardMeta) -> String {
        switch m.stage.lowercased() {
        case "analyze": return "magnifyingglass.circle"
        case "discover": return "person.2.circle"
        case "generate": return "wand.and.stars"
        case "qa": return "checkmark.shield"
        case "under_review": return "eye.circle"
        case "planned": return "calendar.circle"
        case "in_progress": return "play.circle"
        case "done": return "checkmark.circle"
        default: return "map"
        }
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
                Text(m.glyph ?? "🂠")
                    .font(.system(size: 108))
                    .fixedSize()
                    .offset(y: -36)
                
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
                        
                        Text(displayStageName(m.stage))
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundColor(.stageActive)

                        let headerProgress: Int? = isRoadmapWorkflow ? inferredRoadmapProgress(m) : m.progress
                        if let prog = headerProgress {
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
            let prog = isRoadmapWorkflow ? inferredRoadmapProgress(m) : (m.progress ?? 0)
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
                
                if let logsURL {
                    Link(destination: logsURL) {
                        HStack(spacing: 6) {
                            Image(systemName: "scroll")
                                .font(.system(size: 11))
                            Text(logsButtonText)
                                .font(.system(size: 13, weight: .bold))
                        }
                        .foregroundColor(.tailText)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(Color.tailBg)
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                    }
                    .help(logsHelpText)
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
                    if let stopURL {
                        Link(destination: stopURL) {
                            HStack(spacing: 6) {
                                Image(systemName: "square")
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

            if isRoadmapWorkflow {
                roadmapStatusPanel(m)
            } else {
                StagePipeline(
                    currentStage: m.stage,
                    stages: m.stages,
                    displayStages: displayStageOrder(for: m)
                )
            }
            
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
    private func roadmapStatusPanel(_ m: JobCardMeta) -> some View {
        let progress = inferredRoadmapProgress(m)
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .center, spacing: 12) {
                Image(systemName: roadmapStatusIcon(m))
                    .font(.system(size: 30, weight: .medium))
                    .foregroundColor(.stagePending)
                    .frame(width: 52, height: 52)
                    .background(Color.pillPurpleBg.opacity(0.7))
                    .clipShape(Circle())

                VStack(alignment: .leading, spacing: 4) {
                    Text(roadmapStatusTitle(m))
                        .font(.system(size: 24, weight: .bold))
                        .foregroundColor(.textPrimary)
                    Text(roadmapStatusSubtitle(m))
                        .font(.system(size: 14))
                        .foregroundColor(.textSecondary)
                    if let activeTool {
                        Text("[Tool: \(activeTool)]")
                            .font(.system(size: 13, design: .monospaced))
                            .foregroundColor(.stagePending)
                    }
                }

                Spacer()
                if let stopURL {
                    Link(destination: stopURL) {
                        HStack(spacing: 6) {
                            Image(systemName: "square")
                                .font(.system(size: 11, weight: .semibold))
                            Text("Stop")
                                .font(.system(size: 13, weight: .bold))
                        }
                        .foregroundColor(.stopText)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(Color.stopBg)
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                    }
                }
            }

            HStack(spacing: 8) {
                Text("Progress")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundColor(.textSecondary)
                Image(systemName: "clock")
                    .font(.system(size: 12))
                    .foregroundColor(.stagePending)
                Text(elapsedTimeText(m))
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundColor(.textSecondary)
                Text("·")
                    .foregroundColor(.textMuted)
                Text("last activity \(lastActivityText())")
                    .font(.system(size: 13))
                    .foregroundColor(.stagePending)
                Spacer()
                Circle()
                    .fill(Color.stagePending)
                    .frame(width: 10, height: 10)
                Text("Processing")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundColor(.stagePending)
                Text("\(progress)%")
                    .font(.system(size: 18, weight: .bold))
                    .foregroundColor(.textPrimary)
            }

            if let snapshot = roadmapSnapshot, snapshot.featureCount > 0 {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        metricPill("Under Review \(statusCount("under_review"))")
                        metricPill("Planned \(statusCount("planned"))")
                        metricPill("In Progress \(statusCount("in_progress"))")
                        metricPill("Done \(statusCount("done"))")
                        metricPill("\(priorityCount("must")) must")
                        metricPill("\(priorityCount("should")) should")
                        metricPill("\(priorityCount("could")) could")
                        metricPill("\(snapshot.phaseCount) phases")
                    }
                }
            }

            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Rectangle().fill(Color.barEmpty)
                    Rectangle()
                        .fill(Color.barFill)
                        .frame(width: max(0, geo.size.width * CGFloat(progress) / 100))
                }
            }
            .frame(height: 6)

            StagePipeline(
                currentStage: m.stage,
                stages: m.stages,
                displayStages: displayStageOrder(for: m)
            )
        }
        .padding(16)
        .background(Color.black.opacity(0.18))
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(Color.cardBorder.opacity(0.6), lineWidth: 1)
        )
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
        VStack(alignment: .leading, spacing: 12) {
            if !logs.isEmpty {
                Text(logs)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundColor(Color.white.opacity(0.8))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
                    .background(Color.black.opacity(0.4))
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            } else {
                Text("No logs yet.")
                    .foregroundColor(.textMuted)
            }
            if let logsURL = logsURL {
                Link(destination: logsURL) {
                    HStack(spacing: 6) {
                        Image(systemName: "scroll").font(.system(size: 12))
                        Text(isDoneLike ? "Open logs →" : "Open live tail →")
                            .font(.system(size: 13, weight: .medium))
                    }
                    .foregroundColor(.tailText)
                    .padding(.horizontal, 14).padding(.vertical, 7)
                    .background(Color.tailBg)
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                }
                .help(logsHelpText)
            }
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
        var combinedLogs = ""
        var lastActivityAt: Date?
        let stdoutUrl = url.appendingPathComponent("logs/stdout.log")
        let stderrUrl = url.appendingPathComponent("logs/stderr.log")

        if let data = try? Data(contentsOf: stdoutUrl),
           let text = String(data: data, encoding: .utf8) {
            logs = text.components(separatedBy: .newlines).suffix(100)
                .joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            combinedLogs += text
        }
        if let data = try? Data(contentsOf: stderrUrl),
           let text = String(data: data, encoding: .utf8) {
            combinedLogs += "\n" + text
        }

        let fm = FileManager.default
        for logPath in [stdoutUrl.path, stderrUrl.path] {
            if let attrs = try? fm.attributesOfItem(atPath: logPath),
               let mod = attrs[.modificationDate] as? Date {
                if let curr = lastActivityAt {
                    if mod > curr { lastActivityAt = mod }
                } else {
                    lastActivityAt = mod
                }
            }
        }
        let activeTool = detectToolTag(in: combinedLogs)
        let roadmapSnapshot = parseRoadmapSnapshot(from: url)

        // Enumerate bundle files (top-level only, skip logs/ and output/ dirs)
        var bundleFiles: [String] = []
        if let items = try? FileManager.default.contentsOfDirectory(atPath: url.path) {
            bundleFiles = items
                .filter { !$0.hasPrefix(".") && $0 != "logs" && $0 != "output" && $0 != "worktree" }
                .sorted()
        }

        DispatchQueue.main.async {
            self.hostingView.rootView = JobCardPreview(
                url: url,
                meta: meta,
                logs: logs,
                bundleFiles: bundleFiles,
                lastActivityAt: lastActivityAt,
                activeTool: activeTool,
                roadmapSnapshot: roadmapSnapshot
            )
            self.preferredContentSize = NSSize(width: 820, height: 720)
            handler(nil)
        }
    }
}
