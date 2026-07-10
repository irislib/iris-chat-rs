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

struct DeviceRosterScreen: View {
    @ObservedObject var manager: AppManager
    @State private var deviceInput = ""
    @State private var showingScanner = false

    var body: some View {
        IrisScrollScreen {
            DeviceRosterContent(
                manager: manager,
                deviceInput: $deviceInput,
                showingScanner: $showingScanner
            )
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                _ = submitDeviceAuthorizationScan(code, manager: manager)
                deviceInput = code
                showingScanner = false
            }
            .irisModalSurface()
            .irisDismissOnMacOutsideClick { showingScanner = false }
        }
    }
}

@discardableResult
@MainActor
func submitDeviceAuthorizationScan(
    _ rawInput: String,
    manager: AppManager
) -> ResolvedDeviceAuthorizationInput {
    guard let roster = manager.state.deviceRoster,
          roster.canManageDevices,
          !manager.state.busy.updatingRoster else {
        return ResolvedDeviceAuthorizationInput(
            deviceInput: "",
            errorMessage: nil,
            requiresConfirmation: false
        )
    }
    let resolved = resolveDeviceAuthorizationInput(rawInput: rawInput)
    guard resolved.errorMessage == nil,
          !resolved.deviceInput.isEmpty else {
        return resolved
    }
    return resolved
}

struct DeviceRosterContent: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @Binding var deviceInput: String
    @Binding var showingScanner: Bool
    @State private var pendingDeviceConfirmation: ResolvedDeviceAuthorizationInput?

    private var resolvedInput: ResolvedDeviceAuthorizationInput? {
        guard let roster = manager.state.deviceRoster else {
            return nil
        }
        return resolveDeviceAuthorizationInput(rawInput: deviceInput)
    }

    private var isCurrentDeviceRegistered: Bool {
        guard let roster = manager.state.deviceRoster else {
            return false
        }
        return roster.devices.contains { $0.devicePubkeyHex == roster.currentDevicePublicKeyHex }
    }

    private var deviceAccessSubtitle: String {
        guard let roster = manager.state.deviceRoster else {
            return ""
        }
        if roster.canManageDevices {
            return "Scan the code from the device you want to link, or paste it."
        }
        if isCurrentDeviceRegistered {
            return "This device can view the list but cannot change it."
        }
        return "Sign in with your secret key before changing devices."
    }

    var body: some View {
        Group {
            if let roster = manager.state.deviceRoster {
                IrisSectionCard(accent: true) {
                    CardHeader(
                        title: "Linked devices",
                        subtitle: "These devices can use your profile."
                    )

                    IrisCopyButton(
                        label: "Copy user ID",
                        value: roster.ownerNpub,
                        style: .menuRow
                    )
                    .accessibilityIdentifier("deviceRosterOwnerNpub")
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Link another device",
                        subtitle: deviceAccessSubtitle
                    )

                    TextField("Link code", text: $deviceInput)
                        .irisIdentifierInputModifiers()
                        .textFieldStyle(.plain)
                        .irisInputField()
                        .accessibilityIdentifier("deviceRosterAddInput")
                        .onChange(of: deviceInput) { _ in
                            submitDeviceInputIfReady()
                        }

                    if let error = resolvedInput?.errorMessage {
                        Text(error)
                            .font(.system(.footnote, design: .rounded))
                            .foregroundStyle(.red)
                    }

                    VStack(spacing: 10) {
                        if irisSupportsQrScanning {
                            Button("Scan code") { showingScanner = true }
                                .buttonStyle(IrisPrimaryButtonStyle())
                                .disabled(roster.canManageDevices == false || manager.state.busy.updatingRoster)
                                .accessibilityIdentifier("deviceRosterScanButton")
                        }
                    }
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Devices",
                        subtitle: "\(roster.devices.count) linked"
                    )

                    if roster.devices.isEmpty {
                        Text("No linked devices")
                            .font(.system(.headline, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.textPrimary)
                            .accessibilityIdentifier("deviceRosterEmptyState")
                        Text("Linked devices will appear here.")
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.muted)
                    } else {
                        ForEach(Array(roster.devices.enumerated()), id: \.element.devicePubkeyHex) { index, device in
                            DeviceRosterRow(manager: manager, device: device, canManageDevices: roster.canManageDevices)
                            if index < roster.devices.count - 1 {
                                Divider().overlay(palette.border)
                            }
                        }
                    }
                }
            } else {
                IrisSectionCard {
                    Text("Devices unavailable.")
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                }
            }
        }
        .alert(
            linkDeviceConfirmationTitle(pendingDeviceConfirmation),
            isPresented: Binding(
                get: { pendingDeviceConfirmation != nil },
                set: { visible in
                    if !visible {
                        pendingDeviceConfirmation = nil
                    }
                }
            )
        ) {
            Button("Cancel", role: .cancel) {
                pendingDeviceConfirmation = nil
            }
            Button("Link device") {
                if let pending = pendingDeviceConfirmation {
                    manager.addAuthorizedDevice(deviceInput: pending.deviceInput)
                }
                pendingDeviceConfirmation = nil
            }
            .accessibilityIdentifier("deviceRosterConfirmAdd")
        } message: {
            Text(linkDeviceConfirmationMessage(pendingDeviceConfirmation))
        }
    }

    private func submitDeviceInputIfReady() {
        guard let roster = manager.state.deviceRoster,
              roster.canManageDevices,
              !manager.state.busy.updatingRoster,
              let resolved = resolvedInput,
              resolved.errorMessage == nil,
              !resolved.deviceInput.isEmpty else {
            return
        }
        if resolved.requiresConfirmation {
            pendingDeviceConfirmation = resolved
        } else {
            manager.addAuthorizedDevice(deviceInput: resolved.deviceInput)
        }
        deviceInput = ""
    }
}

private func linkDeviceConfirmationTitle(_: ResolvedDeviceAuthorizationInput?) -> String {
    "Link this device?"
}

private func linkDeviceConfirmationMessage(_: ResolvedDeviceAuthorizationInput?) -> String {
    "This device will be able to use your profile."
}

struct DeviceRosterRow: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let device: DeviceEntrySnapshot
    let canManageDevices: Bool
    @State private var showingRemoveConfirmation = false

    private var displayTitle: String {
        if device.isCurrentDevice {
            return "This device"
        }
        return nonEmpty(device.deviceLabel) ?? "Linked device"
    }

    private var displaySubtitle: String {
        let client = nonEmpty(device.clientLabel)
            ?? (device.isCurrentDevice ? PlatformDeviceLabels.currentClientLabel : "Iris Chat")
        if device.isCurrentDevice, let deviceLabel = nonEmpty(device.deviceLabel) {
            return "\(deviceLabel) - \(client)"
        }
        return client
    }

    private func nonEmpty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        return (trimmed?.isEmpty == false) ? trimmed : nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 12) {
                IrisAvatar(label: displayTitle, size: 36, emphasize: device.isCurrentDevice)
                VStack(alignment: .leading, spacing: 4) {
                    Text(displayTitle)
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    Text(displaySubtitle)
                        .font(.system(.footnote, design: .monospaced))
                        .foregroundStyle(palette.muted)
                        .lineLimit(2)
                }
            }

            HStack(spacing: 8) {
                IrisInfoPill(device.isAuthorized ? "Linked" : "Pending", tint: device.isAuthorized ? .green : .orange)
                if device.isStale {
                    IrisInfoPill("Needs attention", tint: .red)
                }
                if let ago = irisRelativeTime(device.addedAtSecs) {
                    IrisInfoPill("Added \(ago) ago", tint: .gray)
                }
            }

            if canManageDevices && !device.isCurrentDevice {
                removeButton
            }
        }
        .accessibilityIdentifier("deviceRosterRow-\(String(device.devicePubkeyHex.prefix(12)))")
    }

    private var removeButton: some View {
        Button("Remove device", role: .destructive) {
            showingRemoveConfirmation = true
        }
        .buttonStyle(IrisSecondaryButtonStyle())
        .disabled(manager.state.busy.updatingRoster)
        .accessibilityIdentifier("deviceRosterRemove-\(String(device.devicePubkeyHex.prefix(12)))")
        .alert("Remove device?", isPresented: $showingRemoveConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Remove", role: .destructive) {
                manager.removeAuthorizedDevice(devicePubkeyHex: device.devicePubkeyHex)
            }
            .accessibilityIdentifier("deviceRosterConfirmRemove-\(String(device.devicePubkeyHex.prefix(12)))")
        } message: {
            Text("This device will no longer use your profile.")
        }
    }
}

struct DeviceRevokedScreen: View {
    @ObservedObject var manager: AppManager
    @State private var showingLogoutConfirmation = false

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard(accent: true) {
                Text("Device removed")
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)

                Text("This device no longer has access. Sign in again to keep using Iris Chat here.")
                    .font(.system(.body, design: .rounded))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)

                Button("Sign in again") {
                    showingLogoutConfirmation = true
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .accessibilityIdentifier("deviceRevokedLogoutButton")
            }
            .accessibilityIdentifier("deviceRevokedScreen")
        }
        .alert("Delete all local data?", isPresented: $showingLogoutConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                manager.logout()
            }
            .accessibilityIdentifier("deviceRevokedConfirmLogoutButton")
        } message: {
            Text("This removes your secret keys, messages, and cached files from this device.")
        }
    }
}
