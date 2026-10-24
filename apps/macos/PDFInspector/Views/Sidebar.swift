import PdfInspectorKit
import SwiftUI

/// The lists of the sidebar (the web side panel's tabs).
enum SidebarTab: String, CaseIterable, Identifiable {
    case pages, outline, fonts, images, objects, xref, validate, diagnostics

    var id: Self { self }

    var title: String {
        switch self {
        case .pages: "Pages"
        case .outline: "Outline"
        case .fonts: "Fonts"
        case .images: "Images"
        case .objects: "Objects"
        case .xref: "XRef"
        case .validate: "Validate"
        case .diagnostics: "Diagnostics"
        }
    }

    var symbol: String {
        switch self {
        case .pages: "doc.on.doc"
        case .outline: "list.bullet.indent"
        case .fonts: "textformat"
        case .images: "photo"
        case .objects: "cube"
        case .xref: "tablecells"
        case .validate: "checkmark.seal"
        case .diagnostics: "exclamationmark.triangle"
        }
    }
}

struct Sidebar: View {
    let model: DocumentModel
    @Bindable var layout: WindowLayout

    var body: some View {
        VStack(spacing: 0) {
            SummaryCard(model: model)
            Divider()
            Landmarks(model: model)
            Divider()
            Picker("List", selection: $layout.tab) {
                ForEach(SidebarTab.allCases) { tab in
                    Image(systemName: tab.symbol)
                        .help(tab.title)
                        .tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(6)
            Divider()
            panel
                .frame(maxHeight: .infinity)
        }
    }

    @ViewBuilder private var panel: some View {
        switch layout.tab {
        case .pages: PagesPanel(model: model)
        case .outline: NotPorted("Outline", web: "OutlinePanel.tsx")
        case .fonts: FontsPanel(model: model)
        case .images: ImagesPanel(model: model)
        case .objects: ObjectsPanel(model: model)
        case .xref: NotPorted("Cross-reference", web: "XrefPanel.tsx")
        case .validate: NotPorted("Validation", web: "ValidatePanel.tsx")
        case .diagnostics: DiagnosticsPanel(model: model)
        }
    }
}

/// The file at a glance, and checking its objects (the web `SummaryCard`).
struct SummaryCard: View {
    let model: DocumentModel

    var body: some View {
        let s = model.summary
        VStack(alignment: .leading, spacing: 6) {
            Text(s.info.name)
                .font(.headline)
                .help(s.info.path ?? "")
            FlowBadges(items: badges(s))
            if let e = s.encryption, !e.authenticated {
                Label("Encrypted; reopen with the password to see strings and streams.",
                      systemImage: "lock")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
            CheckObjects(model: model)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func badges(_ s: DocSummary) -> [String] {
        var b = ["PDF \(s.version ?? "?")" + (s.pdfVersion.map { $0 != s.version ? " → \($0)" : "" } ?? "")]
        b.append(bytesText(s.info.size))
        b.append("\(s.pages.formatted()) pages")
        b.append("\(s.liveObjects.formatted()) objects")
        b.append("\(s.revisions.count) rev")
        if s.recovered { b.append("recovered") }
        if let e = s.encryption { b.append("\(e.streamMethod) \(e.keyBits)-bit") }
        return b
    }
}

struct FlowBadges: View {
    let items: [String]

    var body: some View {
        HStack(spacing: 4) {
            ForEach(items, id: \.self) { t in
                Text(t)
                    .font(.caption)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .background(.quaternary, in: .rect(cornerRadius: 4))
            }
        }
    }
}

/// The `checkObjects` job: parse every object (and decode the streams).
struct CheckObjects: View {
    let model: DocumentModel
    @State private var error: String?

    private var job: JobInfo? {
        model.latestJob { if case .checkObjects = $0 { true } else { false } }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Button("Check objects") { start(decode: false) }
                Button("+ decode streams") { start(decode: true) }
                if let job, job.state == .running {
                    Button("Cancel", role: .cancel) { Task { await model.cancelJob(job.job) } }
                }
            }
            .controlSize(.small)
            .disabled(job?.state == .running)
            if let job {
                JobProgress(job: job)
                if case .checkObjects(let ok, let failed, let missing, _, _, _, let diagnostics)? = job.result {
                    Text("ok \(ok.formatted()) · failed \(failed) · missing \(missing) · +\(diagnostics) diagnostics")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            if let error {
                Text(error).font(.caption).foregroundStyle(.red)
            }
        }
    }

    private func start(decode: Bool) {
        error = nil
        Task {
            do {
                try await model.startJob(.checkObjects(decodeStreams: decode))
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

struct JobProgress: View {
    let job: JobInfo

    var body: some View {
        if job.state == .running {
            ProgressView(value: Double(job.done), total: Double(max(job.total, 1)))
                .controlSize(.small)
        } else if job.state != .completed {
            Text(job.error ?? "\(job.state)")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}

/// Starting points of the document (the web `Landmarks`).
struct Landmarks: View {
    let model: DocumentModel

    var body: some View {
        let s = model.summary
        HStack(spacing: 4) {
            landmark("Overview", .document)
            landmark("Trailer", .trailer)
            if let root = s.root { landmark("Catalog", .object(root)) }
            if let info = s.infoDict { landmark("Info", .object(info)) }
            Spacer()
        }
        .controlSize(.small)
        .padding(6)
    }

    private func landmark(_ title: String, _ t: Target) -> some View {
        Button(title) { model.nav.open(t) }
            .buttonStyle(.accessoryBar)
            .background(model.nav.target.isSame(as: t) ? Color.accentColor.opacity(0.2) : .clear,
                        in: .rect(cornerRadius: 5))
    }
}
