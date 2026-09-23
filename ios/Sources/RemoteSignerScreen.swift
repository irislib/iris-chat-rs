import SwiftUI

struct RemoteSignerScreen: View {
    @ObservedObject var manager: AppManager
    @Environment(\.openURL) private var openURL
    @State private var showingLinkInput = false
    @State private var signerLink = ""

    private var login: RemoteSignerLoginSnapshot? { manager.state.remoteSignerLogin }
    private var awaitingApproval: Bool {
        login?.phase == .waitingForApproval || login?.phase == .finishing
    }

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard {
                VStack(spacing: 20) {
                    CardHeader(title: "Signer app/device")
                        .accessibilityIdentifier("remoteSignerScreen")
                    if let uri = login?.connectionUri, !awaitingApproval {
                        Text("Scan with your signer app.")
                            .foregroundStyle(.secondary)
                        QrCodeImage(text: uri, size: 240)
                            .padding(16)
                            .background(Color.white)
                            .clipShape(RoundedRectangle(cornerRadius: 12))
                            .accessibilityIdentifier("remoteSignerCode")
                        Button("Copy code") { manager.copyToClipboard(uri) }
                            .buttonStyle(IrisSecondaryButtonStyle())
                    } else if let login {
                        ProgressView()
                        Text(statusText(login.phase)).foregroundStyle(.secondary)
                    } else {
                        Button("Try again") { manager.dispatch(.startRemoteSignerLogin) }
                            .buttonStyle(IrisSecondaryButtonStyle())
                    }
                    if let value = login?.authUrl, let url = safeApprovalURL(value) {
                        Button("Open approval") { openURL(url) }
                            .buttonStyle(IrisPrimaryButtonStyle())
                            .accessibilityIdentifier("remoteSignerApprovalAction")
                    }
                    if showingLinkInput {
                        TextField("Signer link", text: $signerLink)
                            .irisInputField()
                            .accessibilityIdentifier("remoteSignerLinkInput")
                        Button("Connect") {
                            manager.dispatch(.connectRemoteSigner(connectionUri: signerLink))
                            showingLinkInput = false
                            signerLink = ""
                        }
                        .buttonStyle(IrisPrimaryButtonStyle())
                        .disabled(signerLink.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("remoteSignerConnectAction")
                    } else if !awaitingApproval {
                        Button("Paste signer link") {
                            signerLink = PlatformClipboard.string() ?? ""
                            showingLinkInput = true
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("remoteSignerPasteLink")
                    }
                }
                .frame(maxWidth: .infinity)
                .multilineTextAlignment(.center)
            }
            .frame(maxWidth: 480)
            .frame(maxWidth: .infinity)
        }
    }

    private func statusText(_ phase: RemoteSignerPhase) -> String {
        switch phase {
        case .connecting: return "Connecting…"
        case .waitingForSigner: return "Waiting for your signer…"
        case .waitingForApproval: return "Approve in your signer app."
        case .finishing: return "Signing in…"
        }
    }

    private func safeApprovalURL(_ value: String) -> URL? {
        guard let url = URL(string: value),
              let scheme = url.scheme?.lowercased(),
              ["https", "http"].contains(scheme), url.host != nil else { return nil }
        return url
    }
}
