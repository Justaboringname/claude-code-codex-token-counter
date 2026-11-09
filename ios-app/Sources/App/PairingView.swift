// Not paired yet: scan the Mac's QR code, or paste the pairing code.
import SwiftUI

struct PairingView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var scheme
    @State private var code = ""
    @State private var busy = false
    @State private var failure: SyncError?
    @State private var scanning = false
    @FocusState private var focused: Bool

    var body: some View {
        let t = Theme(scheme)
        ZStack {
            Wallpaper(dark: t.dark)
            GeometryReader { geo in
            ScrollView {
                VStack(spacing: 0) {
                    AppMark(size: 84, dark: t.dark)
                        .shadow(color: .black.opacity(0.25), radius: 15, y: 10)
                    Text("Token Counter")
                        .font(.system(size: 30, weight: .bold))
                        .tracking(-0.6)
                        .foregroundStyle(t.wallInk)
                        .padding(.top, 18)
                        .accessibilityAddTraits(.isHeader)
                    Text("Your Mac's Claude Code and Codex usage, on this iPhone.")
                        .font(.system(size: 16))
                        .multilineTextAlignment(.center)
                        .foregroundStyle(t.wallInkSoft)
                        .frame(maxWidth: 300)
                        .padding(.top, 6)
                    card(t).padding(.top, 26)
                    Label("End-to-end encrypted. The server only stores ciphertext; the pairing code is the key.", systemImage: "lock")
                        .font(.system(size: 13))
                        .foregroundStyle(t.wallInkSoft)
                        .padding(.horizontal, 6)
                        .padding(.top, 16)
                }
                .frame(maxWidth: 420)
                .padding(.horizontal, 16)
                .padding(.vertical, 24)
                // Centred in the screen; scrolls only when it doesn't fit.
                .frame(maxWidth: .infinity, minHeight: geo.size.height)
            }
            .scrollBounceBehavior(.basedOnSize)
            .scrollDismissesKeyboard(.interactively)
            }
        }
        .sheet(isPresented: $scanning) {
            QRScanner { scanned in
                scanning = false
                code = scanned
                submit()
            }
            .ignoresSafeArea()
            .presentationDragIndicator(.visible)
        }
    }

    private func card(_ t: Theme) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            if busy {
                HStack(spacing: 10) {
                    ProgressView()
                    Text("Pairing…").font(.system(size: 16))
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 22)
            } else {
                step(t, "1", Text("On the Mac running Token Counter, click the \(Image(systemName: "icloud")) cloud button."))
                step(
                    t, "2",
                    QRScanner.isAvailable
                        ? Text("Scan its QR code — or click **Copy** next to the pairing code and paste it here.")
                        : Text("Click **Copy** next to the pairing code, then paste it here.")
                )
                .padding(.top, 6)

                TextField("tc1.…", text: $code, axis: .vertical)
                    .lineLimit(1...3)
                    .font(.system(size: 15, design: .monospaced))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.asciiCapable)
                    .submitLabel(.go)
                    .focused($focused)
                    .onSubmit(submit)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 11)
                    .background(t.field, in: .rect(cornerRadius: 12))
                    .padding(.top, 16)
                    .accessibilityLabel("Pairing code")

                if let failure {
                    Label(failure.describe(hasSnapshot: false).long, systemImage: "exclamationmark.triangle")
                        .font(.system(size: 14))
                        .foregroundStyle(t.warn)
                        .padding(.top, 10)
                }

                HStack(spacing: 8) {
                    if QRScanner.isAvailable {
                        Button {
                            focused = false
                            scanning = true
                        } label: {
                            Label("Scan QR code", systemImage: "qrcode.viewfinder").frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.glass)
                    }
                    PasteButton(payloadType: String.self) { strings in
                        let pasted = strings.first ?? ""
                        Task { @MainActor in
                            code = SyncProtocol.extractCode(pasted)
                            submit()
                        }
                    }
                    .buttonBorderShape(.capsule)
                    .tint(t.accent)
                }
                .controlSize(.large)
                .padding(.top, 14)

                Button(action: submit) {
                    Text("Pair").fontWeight(.semibold).frame(maxWidth: .infinity)
                }
                .buttonStyle(.glassProminent)
                .tint(t.accent)
                .controlSize(.large)
                .disabled(code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .padding(.top, 8)
            }
        }
        .foregroundStyle(t.ink)
        .padding(.horizontal, 18)
        .padding(.top, 20)
        .padding(.bottom, 18)
        .frame(maxWidth: .infinity)
        .glassEffect(.regular.tint(t.cardTint), in: .rect(cornerRadius: 26))
    }

    private func step(_ t: Theme, _ n: String, _ text: Text) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text("\(n).").monospacedDigit().foregroundStyle(t.inkSoft)
            text
        }
        .font(.system(size: 15))
        .foregroundStyle(t.inkStrong)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func submit() {
        let raw = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty, !busy else { return }
        focused = false
        busy = true
        failure = nil
        Task {
            do {
                try await model.pair(raw)
            } catch let e as SyncError {
                failure = e
            } catch {
                failure = SyncError(kind: .network, message: "Can't reach the sync server.")
            }
            busy = false
        }
    }
}
