import Cocoa
import Quartz
import SwiftUI

// MARK: - Codable model

private struct JobCardMeta: Codable {
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
    let acceptanceCriteria: [String]?

    enum CodingKeys: String, CodingKey {
        case id, title, description, stage, priority, created, labels, progress, subtasks, stages
        case acceptanceCriteria = "acceptance_criteria"
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
    static let cardBg        = Color(red: 0.100, green: 0.075, blue: 0.175)
    static let cardBorder    = Color(red: 0.220, green: 0.175, blue: 0.340)
    static let pillPurple    = Color(red: 0.420, green: 0.220, blue: 0.820)
    static let pillGold      = Color(red: 0.820, green: 0.600, blue: 0.050)
    static let pillTeal      = Color(red: 0.100, green: 0.580, blue: 0.680)
    static let barFill       = Color(red: 0.480, green: 0.230, blue: 0.880)
    static let dotOn         = Color(red: 0.480, green: 0.230, blue: 0.880)
    static let dotOff        = Color.white.opacity(0.18)
    static let stageHl       = Color(red: 0.480, green: 0.230, blue: 0.880)
    static let stageDone     = Color.white.opacity(0.38)
    static let stageInactive = Color.white.opacity(0.28)
    static let stopOrange    = Color(red: 0.95, green: 0.42, blue: 0.08)
    static let textPrimary   = Color.white
    static let textMuted     = Color.white.opacity(0.52)
    static let textSub       = Color.white.opacity(0.72)
    static let textHl        = Color(red: 0.68, green: 0.52, blue: 1.00)
}

// MARK: - Stage display names
// "implement" → "Code", "qa" → "QA" so pipeline reads cleanly

private let stageOrder: [(key: String, label: String)] = [
    ("spec",      "Spec"),
    ("plan",      "Plan"),
    ("implement", "Code"),
    ("qa",        "QA"),
]

// MARK: - Label pill

private struct LabelPill: View {
    let label: MetaLabel

    var color: Color {
        switch label.kind {
        case "effort": return .pillGold
        case "scope":  return .pillTeal
        default:       return .pillPurple
        }
    }
    var icon: String {
        switch label.kind {
        case "effort": return "bolt.fill"
        case "scope":  return "checkmark.shield"
        default:       return "arrow.2.circlepath"
        }
    }

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: icon).font(.system(size: 9, weight: .bold))
            Text(label.name).font(.system(size: 11, weight: .semibold))
        }
        .foregroundColor(color)
        .padding(.horizontal, 9).padding(.vertical, 4)
        .background(color.opacity(0.14))
        .clipShape(Capsule())
        .overlay(Capsule().stroke(color.opacity(0.38), lineWidth: 1))
    }
}

// MARK: - Subtask dots

private struct SubtaskDots: View {
    let subtasks: [MetaSubtask]
    private let cap = 10

    var body: some View {
        HStack(spacing: 5) {
            ForEach(Array(subtasks.prefix(cap).enumerated()), id: \.offset) { _, st in
                Circle()
                    .fill(st.done ? Color.dotOn : Color.dotOff)
                    .frame(width: 8, height: 8)
            }
            if subtasks.count > cap {
                Text("+\(subtasks.count - cap)")
                    .font(.system(size: 10))
                    .foregroundColor(.textMuted)
            }
        }
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
                let isDone    = st == "done"

                HStack(spacing: 4) {
                    if isDone {
                        Image(systemName: "checkmark")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundColor(.stageDone)
                    }
                    Text(pair.label)
                        .font(.system(size: 11, weight: isCurrent ? .bold : .regular))
                        .foregroundColor(isCurrent ? .textPrimary : isDone ? .stageDone : .stageInactive)
                }
                .padding(.horizontal, 8).padding(.vertical, 4)
                .background(isCurrent ? Color.stageHl.opacity(0.22) : .clear)
                .clipShape(RoundedRectangle(cornerRadius: 5))
                .overlay(
                    RoundedRectangle(cornerRadius: 5)
                        .stroke(isCurrent ? Color.stageHl.opacity(0.55) : .clear, lineWidth: 1)
                )

                if idx < stageOrder.count - 1 {
                    Rectangle()
                        .fill(Color.white.opacity(0.14))
                        .frame(width: 14, height: 1)
                }
            }
        }
    }
}

// MARK: - Progress bar

private struct ProgressBar: View {
    let value: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack {
                Text("Progress").font(.system(size: 11)).foregroundColor(.textMuted)
                Spacer()
                Text("\(value)%").font(.system(size: 11, weight: .semibold)).foregroundColor(.textHl)
            }
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 3).fill(Color.white.opacity(0.09))
                    RoundedRectangle(cornerRadius: 3)
                        .fill(Color.barFill)
                        .frame(width: max(0, geo.size.width * CGFloat(value) / 100))
                }
            }
            .frame(height: 6)
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

struct JobCardPreview: View {
    var url: URL?
    var meta: JobCardMeta?
    var logs: String = ""

    private var displayTitle: String {
        meta?.title ?? meta?.id ?? url?.lastPathComponent ?? "JobCard"
    }

    // Stop only appears when the card stage is actively running
    private var isRunning: Bool {
        guard let m = meta else { return false }
        return m.stages?[m.stage]?.status == "running"
    }

    var body: some View {
        ZStack {
            Color.cardBg.ignoresSafeArea()
            if let m = meta {
                cardBody(m)
            } else {
                Text("Loading…").foregroundColor(.textMuted)
            }
        }
    }

    @ViewBuilder
    private func cardBody(_ m: JobCardMeta) -> some View {
        VStack(alignment: .leading, spacing: 0) {

            // Header
            HStack(alignment: .top, spacing: 10) {
                VStack(alignment: .leading, spacing: 5) {
                    Text(displayTitle)
                        .font(.system(size: 15, weight: .bold))
                        .foregroundColor(.textPrimary)
                        .lineLimit(2)
                    if let desc = m.description {
                        Text(desc)
                            .font(.system(size: 12))
                            .foregroundColor(.textSub)
                            .lineLimit(3)
                    }
                }
                Spacer(minLength: 8)
                if let p = m.priority {
                    Text("P\(p)")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundColor(.pillGold)
                        .padding(.horizontal, 7).padding(.vertical, 3)
                        .background(Color.pillGold.opacity(0.14))
                        .clipShape(Capsule())
                }
            }
            .padding(.bottom, 12)

            // Labels
            if let labels = m.labels, !labels.isEmpty {
                HStack(spacing: 6) {
                    ForEach(labels, id: \.name) { LabelPill(label: $0) }
                    Spacer()
                }
                .padding(.bottom, 11)
            }

            // Progress bar
            if let prog = m.progress {
                ProgressBar(value: prog).padding(.bottom, 11)
            }

            // Subtask dots
            if let subs = m.subtasks, !subs.isEmpty {
                HStack(spacing: 8) {
                    Text("\(subs.filter(\.done).count)/\(subs.count)")
                        .font(.system(size: 11))
                        .foregroundColor(.textMuted)
                    SubtaskDots(subtasks: subs)
                    Spacer()
                }
                .padding(.bottom, 11)
            }

            Divider().background(Color.cardBorder).padding(.bottom, 10)

            // Stage pipeline
            StagePipeline(currentStage: m.stage, stages: m.stages)
                .padding(.bottom, 12)

            // Acceptance criteria
            if let criteria = m.acceptanceCriteria, !criteria.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Acceptance Criteria")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(.textMuted)
                    ForEach(criteria, id: \.self) { c in
                        HStack(alignment: .top, spacing: 5) {
                            Text("›").foregroundColor(.textHl)
                            Text(c)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(.textSub)
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
                        .foregroundColor(.textMuted)
                    ScrollView {
                        Text(logs)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(Color.white.opacity(0.70))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 90)
                    .padding(8)
                    .background(Color.black.opacity(0.28))
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                }
                .padding(.bottom, 12)
            }

            Spacer(minLength: 0)

            // Footer
            HStack(spacing: 5) {
                Image(systemName: "clock").font(.system(size: 10)).foregroundColor(.textMuted)
                Text(relativeTime(m.created)).font(.system(size: 11)).foregroundColor(.textMuted)
                Spacer()
                if isRunning {
                    HStack(spacing: 4) {
                        Circle().fill(Color.stopOrange).frame(width: 6, height: 6)
                        Text("Stop")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundColor(.stopOrange)
                    }
                    .padding(.horizontal, 10).padding(.vertical, 5)
                    .background(Color.stopOrange.opacity(0.13))
                    .clipShape(Capsule())
                    .overlay(Capsule().stroke(Color.stopOrange.opacity(0.38), lineWidth: 1))
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
            logs = text.components(separatedBy: .newlines).suffix(25)
                .joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }

        DispatchQueue.main.async {
            self.hostingView.rootView = JobCardPreview(url: url, meta: meta, logs: logs)
            self.preferredContentSize = NSSize(width: 500, height: 660)
            handler(nil)
        }
    }
}
