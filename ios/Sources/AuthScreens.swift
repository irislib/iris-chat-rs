import Foundation
import Combine
import SwiftUI
import UniformTypeIdentifiers
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif
#if canImport(PhotosUI)
import PhotosUI
#endif

struct WelcomeScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @AppStorage(irisTermsAcceptedDefaultsKey) private var termsAccepted = false

    var body: some View {
        IrisScrollScreen {
            VStack(spacing: 20) {
                VStack(spacing: 18) {
                    Color.clear
                        .frame(height: 0)
                        .accessibilityIdentifier("welcomeChooserCard")

                    Image("IrisLogo")
                        .resizable()
                        .scaledToFit()
                        .frame(width: 132, height: 132)
                        .accessibilityHidden(true)

                    HStack(spacing: 0) {
                        Text("iris")
                            .foregroundStyle(palette.accent)
                        Text(" chat")
                            .foregroundStyle(palette.textPrimary)
                    }
                    .font(.system(.largeTitle, design: .rounded, weight: .bold))

                    VStack(spacing: 10) {
                        Button {
                            manager.dispatch(.pushScreen(screen: .createAccount))
                        } label: {
                            Label("Create profile", systemImage: "plus")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(IrisPrimaryButtonStyle())
                        .accessibilityIdentifier("welcomeCreateAction")

                        Button {
                            manager.dispatch(.pushScreen(screen: .restoreAccount))
                        } label: {
                            Label("Restore profile", systemImage: "key.fill")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("welcomeRestoreAction")

                        Button {
                            manager.dispatch(.pushScreen(screen: .addDevice))
                        } label: {
                            Label("Link this device", systemImage: "iphone")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("welcomeAddDeviceAction")
                    }
                    .frame(maxWidth: 320)
                    .disabled(!termsAccepted)
                    .opacity(termsAccepted ? 1 : 0.46)

                    OnboardingTermsAgreement(accepted: $termsAccepted)
                        .frame(maxWidth: 360)
                }
                .frame(maxWidth: .infinity)

                if manager.trustedTestBuildEnabled() {
                    Text("Test build")
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.accentAlt)
                        .accessibilityIdentifier("welcomeSecondaryCard")
                }
            }
            .frame(maxWidth: 480)
            .frame(maxWidth: .infinity)
            .padding(.top, IrisLayout.usesDesktopChrome ? 96 : 56)
        }
    }
}

private struct OnboardingTermsAgreement: View {
    @Environment(\.irisPalette) private var palette
    @Binding var accepted: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Iris has no tolerance for objectionable content or abusive users.")
                .font(.system(.caption, design: .rounded))
                .foregroundStyle(palette.muted)
                .accessibilityIdentifier("onboardingTermsNotice")

            Button {
                accepted.toggle()
            } label: {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: accepted ? "checkmark.square.fill" : "square")
                        .foregroundStyle(accepted ? palette.accent : palette.muted)
                    Text("I agree to the Terms of Use")
                        .font(.system(.subheadline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .accessibilityIdentifier("onboardingTermsAgreementToggle")

            HStack(spacing: 14) {
                Link("Terms", destination: irisTermsURL)
                    .accessibilityIdentifier("onboardingTermsLink")
                Link("Privacy", destination: irisPrivacyURL)
                    .accessibilityIdentifier("onboardingPrivacyLink")
            }
            .font(.system(.caption, design: .rounded, weight: .semibold))
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(palette.panel.opacity(0.72))
        )
    }
}

struct CreateAccountScreen: View {
    @ObservedObject var manager: AppManager
    @State private var displayName = ""
    @FocusState private var isNameFocused: Bool

    private var trimmedDisplayName: String {
        displayName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canCreateAccount: Bool {
        !trimmedDisplayName.isEmpty && !manager.state.busy.creatingAccount
    }

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("createAccountScreen")

                CardHeader(
                    title: "Create profile"
                )

                TextField("Name", text: $displayName)
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .focused($isNameFocused)
                    .submitLabel(.done)
                    .onSubmit(submitCreateAccount)
                    .accessibilityIdentifier("signupNameField")

                Button(manager.state.busy.creatingAccount ? "Creating…" : "Create profile") {
                    submitCreateAccount()
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(!canCreateAccount)
                .accessibilityIdentifier("generateKeyButton")
            }
        }
        .onAppear {
            DispatchQueue.main.async {
                isNameFocused = true
            }
        }
    }

    private func submitCreateAccount() {
        guard canCreateAccount else { return }
        manager.createAccount(name: trimmedDisplayName)
    }
}

struct RestoreAccountScreen: View {
    @ObservedObject var manager: AppManager
    @StateObject private var restoreSecret = SecretKeyDraft()
    @State private var lastSubmittedSecret: String?

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("restoreAccountScreen")

                CardHeader(
                    title: "Restore profile",
                    subtitle: "Paste your secret key."
                )

                SecretKeyField(text: Binding(
                    get: { restoreSecret.text },
                    set: { updateSecret($0) }
                ))
                    .irisInputField()

                Text("Secret key = nostr nsec")
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Button(manager.state.busy.restoringSession ? "Restoring…" : "Restore profile") {
                    submitRestore(restoreSecret.text, force: true)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(manager.state.busy.restoringSession)
                .accessibilityIdentifier("importKeyButton")
            }
        }
    }

    private func updateSecret(_ value: String) {
        let previous = restoreSecret.text.trimmingCharacters(in: .whitespacesAndNewlines)
        restoreSecret.text = value
        let current = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard shouldAutoSubmitSecret(previous: previous, current: current) else {
            return
        }
        submitRestore(current)
    }

    private func submitRestore(_ value: String, force: Bool = false) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            manager.restoreSession(ownerNsec: trimmed)
            return
        }
        guard !manager.state.busy.restoringSession else { return }
        guard force || lastSubmittedSecret != trimmed else {
            return
        }
        lastSubmittedSecret = trimmed
        manager.restoreSession(ownerNsec: trimmed)
    }
}

struct AddDeviceScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let awaitingApproval: Bool

    @State private var showingLogoutConfirmation = false

    var body: some View {
        IrisScrollScreen {
            linkDeviceCard
                .frame(maxWidth: 480)
                .frame(maxWidth: .infinity)
        }
        .onAppear {
            if !awaitingApproval,
               manager.state.linkDevice == nil,
               !manager.state.busy.linkingDevice {
                manager.startLinkedDevice(ownerInput: "")
            }
        }
        .alert("Delete all local data?", isPresented: $showingLogoutConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                manager.logout()
            }
            .accessibilityIdentifier("awaitingApprovalConfirmLogoutButton")
        } message: {
            Text("This removes your secret keys, messages, and cached files from this device.")
        }
    }

    private var linkDeviceCard: some View {
        IrisSectionCard {
            Color.clear
                .frame(height: 0)
                .accessibilityIdentifier("addDeviceScreen")

            CardHeader(
                title: awaitingApproval ? "Finish linking" : "Link this device",
                subtitle: awaitingApproval
                    ? "Waiting for approval from your signed-in device."
                    : "Scan this code with your signed-in device."
            )

            if awaitingApproval {
                Button("Sign out") {
                    showingLogoutConfirmation = true
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .accessibilityIdentifier("awaitingApprovalLogoutButton")
            } else if let linkDevice = manager.state.linkDevice {
                ZStack {
                    QrCodeImage(text: linkDevice.url)
                        .frame(width: 240, height: 240)
                    Color.clear
                        .accessibilityIdentifier("linkDeviceQrCode")
                }
                .frame(maxWidth: .infinity)

                VStack(spacing: 10) {
                    Button("Copy link code") {
                        manager.copyToClipboard(linkDevice.url)
                    }
                    .buttonStyle(IrisPrimaryButtonStyle())
                    .accessibilityIdentifier("linkDeviceCopyButton")

                    Button(manager.state.busy.linkingDevice ? "Creating…" : "New code") {
                        manager.startLinkedDevice(ownerInput: "")
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .disabled(manager.state.busy.linkingDevice)
                    .accessibilityIdentifier("linkDeviceRefreshButton")
                }
            } else {
                ProgressView()
                    .accessibilityIdentifier("linkDeviceCreating")
            }
        }
    }
}
