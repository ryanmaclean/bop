import Foundation
import QuickLook

struct JobCardMeta: Decodable {
    let id: String?
    let stage: String?
    let agent_type: String?
    let provider_chain: [String]?
    let acceptance_criteria: [String]?
}

final class PreviewProvider: QLPreviewProvider {
    func providePreview(for request: QLFilePreviewRequest) async throws -> QLPreviewReply {
        let metaUrl = request.fileURL.appendingPathComponent("meta.json")
        let metaData = (try? Data(contentsOf: metaUrl)) ?? Data()
        let meta = (try? JSONDecoder().decode(JobCardMeta.self, from: metaData)) ?? JobCardMeta(id: nil, stage: nil, agent_type: nil, provider_chain: nil, acceptance_criteria: nil)

        let title = meta.id ?? request.fileURL.deletingPathExtension().lastPathComponent
        let stage = meta.stage ?? "unknown"
        let agent = meta.agent_type ?? ""
        let providers = (meta.provider_chain ?? []).joined(separator: ", ")
        let criteria = (meta.acceptance_criteria ?? []).joined(separator: "<br>")

        let html = """
        <!doctype html>
        <html>
          <head>
            <meta charset=\"utf-8\" />
            <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />
            <style>
              body { font-family: -apple-system, BlinkMacSystemFont, Helvetica, Arial, sans-serif; padding: 18px; }
              .card { border: 1px solid #ddd; border-radius: 10px; padding: 16px; }
              .row { margin-top: 10px; }
              .k { color: #666; font-size: 12px; text-transform: uppercase; letter-spacing: 0.04em; }
              .v { font-size: 14px; margin-top: 2px; }
              .title { font-size: 18px; font-weight: 600; }
              .stage { display: inline-block; padding: 4px 10px; border-radius: 999px; background: #f2f2f7; font-size: 12px; margin-top: 8px; }
            </style>
          </head>
          <body>
            <div class=\"card\">
              <div class=\"title\">\(escapeHtml(title))</div>
              <div class=\"stage\">stage: \(escapeHtml(stage))</div>

              <div class=\"row\">
                <div class=\"k\">agent</div>
                <div class=\"v\">\(escapeHtml(agent))</div>
              </div>

              <div class=\"row\">
                <div class=\"k\">providers</div>
                <div class=\"v\">\(escapeHtml(providers))</div>
              </div>

              <div class=\"row\">
                <div class=\"k\">acceptance criteria</div>
                <div class=\"v\">\(criteria.isEmpty ? "" : criteria)</div>
              </div>
            </div>
          </body>
        </html>
        """

        return QLPreviewReply(dataOfContentType: UTType.html.identifier) {
            html.data(using: .utf8) ?? Data()
        }
    }

    private func escapeHtml(_ s: String) -> String {
        s
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
            .replacingOccurrences(of: "\"", with: "&quot;")
    }
}
