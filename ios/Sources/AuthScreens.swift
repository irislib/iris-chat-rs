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

    var body: some View {
        GeometryReader { proxy in
            ScrollView {
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
                                .foregroundStyle(palette.textPrimary)
                            Text(" chat")
                                .foregroundStyle(palette.textPrimary)
                        }
                        .font(.system(.largeTitle, design: .rounded, weight: .bold))

                        VStack(spacing: 12) {
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
                        }
                        .frame(maxWidth: 320)
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
                .padding(.horizontal, IrisLayout.contentHorizontalPadding)
                .padding(.vertical, IrisLayout.contentBottomPadding)
                .frame(minHeight: proxy.size.height, alignment: .center)
            }
            .scrollIndicators(.hidden)
        }
    }
}

func irisRequiresOnboardingTermsAcceptance() -> Bool {
#if os(iOS)
    return true
#else
    return false
#endif
}

private struct OnboardingTermsAgreement: View {
    @Environment(\.irisPalette) private var palette
    @Binding var accepted: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                accepted.toggle()
            } label: {
                Label(
                    accepted ? "Terms accepted" : "Accept Terms",
                    systemImage: accepted ? "checkmark.square.fill" : "square"
                )
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
            .font(.system(.callout, design: .rounded, weight: .semibold))
            .foregroundStyle(palette.textPrimary)
            .accessibilityIdentifier("onboardingTermsAgreementToggle")

            Text("No abusive or objectionable content.")
                .font(.system(.caption, design: .rounded))
                .foregroundStyle(palette.muted)
                .accessibilityIdentifier("onboardingTermsNotice")

            HStack(spacing: 14) {
                Link("Terms", destination: irisTermsURL)
                    .accessibilityIdentifier("onboardingTermsLink")
                Link("Privacy", destination: irisPrivacyURL)
                    .accessibilityIdentifier("onboardingPrivacyLink")
            }
            .font(.system(.caption, design: .rounded, weight: .semibold))
        }
    }
}

struct CreateAccountScreen: View {
    @ObservedObject var manager: AppManager
    @AppStorage(irisTermsAcceptedDefaultsKey) private var termsAccepted = false
    @State private var displayName = ""
    @FocusState private var isNameFocused: Bool

    private var trimmedDisplayName: String {
        displayName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var requiresTermsAcceptance: Bool {
        irisRequiresOnboardingTermsAcceptance()
    }

    private var canCreateAccount: Bool {
        (!requiresTermsAcceptance || termsAccepted) &&
            !trimmedDisplayName.isEmpty &&
            !manager.state.busy.creatingAccount
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

                if requiresTermsAcceptance {
                    OnboardingTermsAgreement(accepted: $termsAccepted)
                }

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
    @AppStorage(irisTermsAcceptedDefaultsKey) private var termsAccepted = false
    @StateObject private var restoreSecret = SecretKeyDraft()
    @State private var lastSubmittedSecret: String?

    private var requiresTermsAcceptance: Bool {
        irisRequiresOnboardingTermsAcceptance()
    }

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

                if requiresTermsAcceptance {
                    OnboardingTermsAgreement(accepted: $termsAccepted)
                }

                Button {
                    manager.dispatch(.pushScreen(screen: .addDevice))
                } label: {
                    Label("Link this device", systemImage: "iphone")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .disabled(requiresTermsAcceptance && !termsAccepted)
                .accessibilityIdentifier("restoreLinkDeviceAction")
            }
        }
    }

    private func updateSecret(_ value: String) {
        restoreSecret.text = value
        let current = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard shouldAutoSubmitSecret(current: current) else { return }
        guard !manager.state.busy.restoringSession else { return }
        guard lastSubmittedSecret != current else { return }
        lastSubmittedSecret = current
        manager.restoreSession(ownerNsec: current)
    }
}

struct AddDeviceScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager

    @AppStorage(irisTermsAcceptedDefaultsKey) private var termsAccepted = false

    private var requiresTermsAcceptance: Bool {
        irisRequiresOnboardingTermsAcceptance()
    }

    private var canUseLinkDevice: Bool {
        !requiresTermsAcceptance || termsAccepted
    }

    var body: some View {
        IrisScrollScreen {
            linkDeviceCard
                .frame(maxWidth: 480)
                .frame(maxWidth: .infinity)
        }
        .onAppear {
            startLinkedDeviceIfNeeded()
        }
        .irisOnChange(of: termsAccepted) { _ in
            startLinkedDeviceIfNeeded()
        }
    }

    private var linkDeviceCard: some View {
        IrisSectionCard {
            Color.clear
                .frame(height: 0)
                .accessibilityIdentifier("addDeviceScreen")

            CardHeader(
                title: "Link this device",
                subtitle: canUseLinkDevice
                    ? "Scan this code with your signed-in device."
                    : "Accept the terms before linking this device."
            )

            if !canUseLinkDevice {
                OnboardingTermsAgreement(accepted: $termsAccepted)
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
                    .disabled(!canUseLinkDevice || manager.state.busy.linkingDevice)
                    .accessibilityIdentifier("linkDeviceRefreshButton")
                }
            } else {
                ProgressView()
                    .accessibilityIdentifier("linkDeviceCreating")
            }
        }
    }

    private func startLinkedDeviceIfNeeded() {
        if canUseLinkDevice,
           manager.state.linkDevice == nil,
           !manager.state.busy.linkingDevice {
            manager.startLinkedDevice(ownerInput: "")
        }
    }
}
