import Cocoa
import Quartz
import SwiftUI

// MARK: - Codable model matching meta.json

struct JobCardMeta: Codable {
    let id: String
    let title: String?
    let description: String?
    let stage: String
    let priority: Int?
    let agentType: String?
    let created: String?
    let labels: [MetaLabel]?
    let progress: Int?
    let subtasks: [MetaSubtask]?
    let stages: [String: MetaStageRecord]?
    let acceptanceCriteria: [String]?

    enum CodingKeys: String, CodingKey {
        case id, title, description, stage, priority, created, labels, progress, subtasks, stages
        case agentType = "agent_type"
        case acceptanceCriteria = "acceptance_criteria"
    }
}

struct MetaLabel: Codable {
    let name: String
    let kind: String?
}

struct MetaSubtask: Codable {
    let id: String
    let title: String
    let done: Bool
}

struct MetaStageRecord: Codable {
    let status: String
    let agent: String?
}

// MARK: - Colours

private extension Color {
    static let cardBg      = Color(red: 0.129, green: 0.102, blue: 0.200)  // #211A33
    static let cardBorder  = Color(red: 0.220, green: 0.180, blue: 0.340)
    static let labelPurple = Color(red: 0.400, green: 0.200, blue: 0.800)
    static let labelGold   = Color(red: 0.800, green: 0.600, blue: 0.100)
    static let labelTeal   = Color(red: 0.100, green: 0.600, blue: 0.700)
    static let progressFill = Color(red: 0.500, green: 0.250, blue: 0.900)
    static let dotDone     = Color(red: 0.500, green: 0.250, blue: 0.900)
    static let dotPending  = Color.white.opacity(0.20)
    static let stageActive = Color(red: 0.500, green: 0.250, blue: 0.900)
    static let stageDone   = Color.white.opacity(0.40)
    static let stopOrange  = Color(red: 0.95, green: 0.45, blue: 0.10)
    static let textPrimary = Color.white
    static let textSecondary = Color.white.opacity(0.55)
    static let textAccent  = Color(red: 0.70, green: 0.55, blue: 1.00)
}

// MARK: - Label pill

private struct LabelPill: View {
    let label: MetaLabel

    private var pillColor: Color {
        switch label.kind {
        case "effort":  return .labelGold
        case "scope":   return .labelTeal
        default:        return .labelPurple
        }
    }

    private var icon: String {
        switch label.kind {
        case "effort":  return "bolt.fill"
        case "scope":   return "checkmark.shield"
        default:        return "arrow.2.circlepath"
        }
    }

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: icon).font(.system(size: 9, weight: .bold))
            Text(label.name).font(.system(size: 11, weight: .semibold))
        }
        .foregroundColor(pillColor)
        .padding(.horizontal, 8).padding(.vertical, 4)
        .background(pillColor.opacity(0.15))
        .clipShape(Capsule())
        .overlay(Capsule().stroke(pillColor.opacity(0.35), lineWidth: 1))
    }
}

// MARK: - Subtask dots

private struct SubtaskDots: View {
    let subtasks: [MetaSubtask]
    private let maxVisible = 10

    var body: some View {
        HStack(spacing: 5) {
            ForEach(Array(subtasks.prefix(maxVisible).enumerated()), id: \.offset) { _, st in
                Circle()
                    .fill(st.done ? Color.dotDone : Color.dotPending)
                    .frame(width: 8, height: 8)
            }
            if subtasks.count > maxVisible {
                Text("+\(subtasks.count - maxVisible)")
                    .font(.system(size: 10))
                    .foregroundColor(.textSecondary)
            }
        }
    }
}

// MARK: - Stage pipeline

private let stageOrder = ["spec", "plan", "implement", "qa"]

private struct StagePipeline: View {
    let currentStage: String
    let stages: [String: MetaStageRecord]?

    private func statusFor(_ s: String) -> String {
        stages?[s]?.status ?? (s == currentStage ? "running" : "pending")
    }

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(stageOrder.enumerated()), id: \.offset) { idx, s in
                StageChip(name: s, status: statusFor(s), isCurrent: s == currentStage)
                if idx < stageOrder.count - 1 {
                    Rectangle()
                        .fill(Color.white.opacity(0.15))
                        .frame(width: 16, height: 1)
                }
            }
        }
    }
}

private struct StageChip: View {
    let name: String
    let status: String
    let isCurrent: Bool

    private var isDone: Bool { status == "done" }

    var body: some View {
        HStack(spacing: 4) {
            if isDone {
                Image(systemName: "checkmark").font(.system(size: 9, weight: .bold))
                    .foregroundColor(.stageDone)
            }
            Text(name.capitalized)
                .font(.system(size: 11, weight: isCurrent ? .bold : .regular))
                .foregroundColor(isCurrent ? .textPrimary : isDone ? .stageDone : .textSecondary)
        }
        .padding(.horizontal, 8).padding(.vertical, 4)
        .background(isCurrent ? Color.stageActive.opacity(0.25) : Color.clear)
        .clipShape(RoundedRectangle(cornerRadius: 5))
        .overlay(
            RoundedRectangle(cornerRadius: 5)
                .stroke(isCurrent ? Color.stageActive.opacity(0.6) : Color.clear, lineWidth: 1)
        )
    }
}

// MARK: - Progress bar

private struct ProgressBar: View {
    let value: Int  // 0-100

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("Progress").font(.system(size: 11)).foregroundColor(.textSecondary)
                Spacer()
                Text("\(value)%").font(.system(size: 11, weight: .semibold)).foregroundColor(.textAccent)
            }
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 3).fill(Color.white.opacity(0.10))
                    RoundedRectangle(cornerRadius: 3)
                        .fill(Color.progressFill)
                        .frame(width: geo.size.width * CGFloat(value) / 100)
                }
            }
            .frame(height: 6)
        }
    }
}

// MARK: - Relative time

private func relativeTime(from isoString: String?) -> String {
    guard let str = isoString else { return "" }
    let f1 = ISO8601DateFormatter()
    f1.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    let f2 = ISO8601DateFormatter()
    guard let date = f1.date(from: str) ?? f2.date(from: str) else {
        return String(str.prefix(10))
    }
    let diff = Int(-date.timeIntervalSinceNow)
    if diff < 60   { return "just now" }
    if diff < 3600 { return "\(diff / 60)m ago" }
    if diff < 86400 { return "\(diff / 3600)h ago" }
    return "\(diff / 86400)d ago"
}

// MARK: - Main card view

struct JobCardPreview: View {
    var url: URL?
    var meta: JobCardMeta?
    var logs: String = ""
    var bundleFiles: [String] = []

    private var displayTitle: String {
        meta?.title ?? meta?.id ?? url?.lastPathComponent ?? "JobCard"
    }

    private var isActive: Bool {
        guard let m = meta else { return false }
        let s = m.stages?[m.stage]?.status ?? ""
        return s == "running" || s == "pending"
    }

    var body: some View {
        ZStack {
            Color.cardBg.ignoresSafeArea()
            if meta != nil {
                cardContent
            } else {
                Text("Loading…").foregroundColor(.textSecondary)
            }
        }
    }

    private var cardContent: some View {
        VStack(alignment: .leading, spacing: 0) {

            // Header
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 8) {
                        Image(systemName: "square.and.pencil")
                            .font(.system(size: 14))
                            .foregroundColor(.textAccent)
                        Text(displayTitle)
                            .font(.system(size: 16, weight: .bold))
                            .foregroundColor(.textPrimary)
                            .lineLimit(2)
                    }
                    if let desc = meta?.description {
                        Text(desc)
                            .font(.system(size: 12))
                            .foregroundColor(.textAccent)
                            .lineLimit(3)
                    }
                }
                Spacer()
                // Priority badge
                if let p = meta?.priority {
                    Text("P\(p)")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundColor(.labelGold)
                        .padding(.horizontal, 7).padding(.vertical, 3)
                        .background(Color.labelGold.opacity(0.15))
                        .clipShape(Capsule())
                }
            }
            .padding(.bottom, 12)

            // Labels
            if let labels = meta?.labels, !labels.isEmpty {
                wrappingLabels(labels)
                    .padding(.bottom, 12)
            }

            // Progress
            if let progress = meta?.progress {
                ProgressBar(value: progress)
                    .padding(.bottom, 12)
            }

            // Subtask dots
            if let subtasks = meta?.subtasks, !subtasks.isEmpty {
                HStack(spacing: 8) {
                    let done = subtasks.filter(\.done).count
                    Text("\(done)/\(subtasks.count)")
                        .font(.system(size: 11))
                        .foregroundColor(.textSecondary)
                    SubtaskDots(subtasks: subtasks)
                }
                .padding(.bottom, 12)
            }

            Divider().background(Color.cardBorder).padding(.bottom, 10)

            // Stage pipeline
            StagePipeline(currentStage: meta?.stage ?? "", stages: meta?.stages)
                .padding(.bottom, 12)

            // Acceptance criteria (compact)
            if let criteria = meta?.acceptanceCriteria, !criteria.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Acceptance Criteria")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(.textSecondary)
                    ForEach(criteria, id: \.self) { c in
                        HStack(alignment: .top, spacing: 6) {
                            Text("›").foregroundColor(.textAccent)
                            Text(c)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(.textPrimary.opacity(0.80))
                        }
                    }
                }
                .padding(.bottom, 12)
            }

            // Bundle files
            if !bundleFiles.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 4) {
                        Text("FILES")
                            .font(.system(size: 10, weight: .bold))
                            .foregroundColor(.textSecondary)
                        Spacer()
                        Text("\(bundleFiles.count)")
                            .font(.system(size: 10))
                            .foregroundColor(.textSecondary)
                    }
                    ForEach(bundleFiles, id: \.self) { file in
                        HStack(spacing: 6) {
                            Image(systemName: fileIcon(for: file))
                                .font(.system(size: 10))
                                .foregroundColor(.textAccent)
                                .frame(width: 14)
                            Text(file)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(.textPrimary.opacity(0.85))
                        }
                    }
                }
                .padding(.bottom, 12)
            }

            // Recent logs
            if !logs.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Recent Output")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(.textSecondary)
                    ScrollView {
                        Text(logs)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.textPrimary.opacity(0.75))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 100)
                    .padding(8)
                    .background(Color.black.opacity(0.30))
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                }
                .padding(.bottom, 12)
            }

            Spacer(minLength: 0)

            // Footer
            HStack(spacing: 6) {
                Image(systemName: "clock").font(.system(size: 10)).foregroundColor(.textSecondary)
                Text(relativeTime(from: meta?.created))
                    .font(.system(size: 11))
                    .foregroundColor(.textSecondary)
                Spacer()
                if isActive {
                    HStack(spacing: 4) {
                        Circle().fill(Color.stopOrange).frame(width: 6, height: 6)
                        Text("Stop")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundColor(.stopOrange)
                    }
                    .padding(.horizontal, 10).padding(.vertical, 5)
                    .background(Color.stopOrange.opacity(0.15))
                    .clipShape(Capsule())
                    .overlay(Capsule().stroke(Color.stopOrange.opacity(0.4), lineWidth: 1))
                }
            }
        }
        .padding(20)
        .background(
            RoundedRectangle(cornerRadius: 12)
                .fill(Color.cardBg)
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.cardBorder, lineWidth: 1))
        )
        .padding(16)
    }

    private func fileIcon(for name: String) -> String {
        if name.hasSuffix(".md") { return "doc.text" }
        if name.hasSuffix(".json") { return "curlybraces" }
        if name.hasSuffix(".log") { return "terminal" }
        if name.hasSuffix(".toml") || name.hasSuffix(".yaml") { return "gearshape" }
        return "doc"
    }

    private func wrappingLabels(_ labels: [MetaLabel]) -> some View {
        // Simple wrapping using a flexible HStack — adequate for QL panels
        var rows: [[MetaLabel]] = [[]]
        for label in labels {
            rows[rows.count - 1].append(label)
        }
        return VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                HStack(spacing: 6) {
                    ForEach(row, id: \.name) { l in LabelPill(label: l) }
                }
            }
        }
    }
}

// MARK: - QL controller

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
        var meta: JobCardMeta?

        func decode(_ data: Data) {
            meta = try? JSONDecoder().decode(JobCardMeta.self, from: data)
        }

        let fm = FileManager.default
        if fm.fileExists(atPath: metaUrl.path), let data = try? Data(contentsOf: metaUrl) {
            decode(data)
        } else {
            var coordError: NSError?
            NSFileCoordinator().coordinate(readingItemAt: metaUrl, options: .withoutChanges, error: &coordError) { u in
                if let data = try? Data(contentsOf: u) { decode(data) }
            }
        }

        var logs = ""
        let logUrl = url.appendingPathComponent("logs/stdout.log")
        func readLog(_ data: Data) {
            if let text = String(data: data, encoding: .utf8) {
                logs = text.components(separatedBy: .newlines).suffix(25)
                    .joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            }
        }

        if fm.fileExists(atPath: logUrl.path), let data = try? Data(contentsOf: logUrl) {
            readLog(data)
        } else {
            var coordError: NSError?
            NSFileCoordinator().coordinate(readingItemAt: logUrl, options: .withoutChanges, error: &coordError) { u in
                if let data = try? Data(contentsOf: u) { readLog(data) }
            }
        }

        // Enumerate bundle files (top-level only, skip logs/ and output/ dirs)
        var bundleFiles: [String] = []
        if let items = try? fm.contentsOfDirectory(atPath: url.path) {
            bundleFiles = items
                .filter { !$0.hasPrefix(".") && $0 != "logs" && $0 != "output" && $0 != "worktree" }
                .sorted()
        }

        DispatchQueue.main.async {
            self.hostingView.rootView = JobCardPreview(url: url, meta: meta, logs: logs, bundleFiles: bundleFiles)
            self.preferredContentSize = NSSize(width: 520, height: 680)
            handler(nil)
        }
    }
}
