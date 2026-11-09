// Sync details: which Mac, how fresh, the last check, and unpairing.
import SwiftUI

struct SyncSheet: View {
    var data: DashboardData
    var actions: DashboardActions

    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var scheme
    @State private var confirmUnpair = false

    var body: some View {
        let t = Theme(scheme)
        NavigationStack {
            TimelineView(.periodic(from: .now, by: 15)) { timeline in
                form(t, timeline.date)
            }
            .navigationTitle("Sync")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    private func form(_ t: Theme, _ now: Date) -> some View {
        let snap = data.snapshot
        let gen = ISODate.parse(snap?.generatedAt)
        let stale = gen.map { now.timeIntervalSince($0) > Dashboard.staleSnapshot } ?? false
        return Form {
            Section {
                HStack(spacing: 12) {
                    Image(systemName: "desktopcomputer")
                        .font(.system(size: 20))
                        .foregroundStyle(t.inkSoft)
                        .frame(width: 44, height: 44)
                        .background(t.field, in: .rect(cornerRadius: 12))
                    VStack(alignment: .leading, spacing: 2) {
                        Text(snap?.deviceName ?? "Main Mac").font(.headline)
                        Text(gen.map { "Updated \(Fmt.relativeAge($0, now: now)) · \(Dashboard.clockTime($0, now: now))" } ?? "No upload yet")
                            .font(.subheadline)
                            .foregroundStyle(stale ? t.warn : t.inkSoft)
                    }
                }
                .accessibilityElement(children: .combine)
            }
            Section {
                if let host = data.serverHost { LabeledContent("Server", value: host) }
                LabeledContent("Checked", value: data.fetchedAt.map { Fmt.relativeAge($0, now: now) } ?? "Not yet")
                if let v = data.version { LabeledContent("Version", value: String(v)) }
                if let e = data.error {
                    let d = e.describe(deviceName: snap?.deviceName, hasSnapshot: snap != nil)
                    Label {
                        Text(d.long)
                    } icon: {
                        Image(systemName: e.isReal ? "exclamationmark.triangle" : "hourglass")
                    }
                    .foregroundStyle(e.isReal ? t.warn : t.inkSoft)
                }
                Button {
                    Task { await actions.refresh(true) }
                } label: {
                    HStack(spacing: 8) {
                        Text(data.refreshing ? "Checking…" : "Check now")
                        if data.refreshing { ProgressView() }
                    }
                }
                .disabled(data.refreshing)
            } footer: {
                Text("This iPhone checks every 30 seconds while the app is open. The Mac uploads when its numbers change.")
            }
            Section {
                Button("Unpair this iPhone", role: .destructive) { confirmUnpair = true }
                    .confirmationDialog("Unpair this iPhone?", isPresented: $confirmUnpair, titleVisibility: .visible) {
                        Button("Unpair", role: .destructive) {
                            dismiss()
                            actions.unpair()
                        }
                    } message: {
                        Text("The pairing code and the saved numbers are removed from this iPhone. The Mac keeps uploading.")
                    }
            } footer: {
                Label(
                    "End-to-end encrypted with the pairing code: the server only ever stores ciphertext. Anyone with the code can read these numbers.",
                    systemImage: "lock"
                )
            }
        }
    }
}
