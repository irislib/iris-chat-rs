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

let irisSourceURL = URL(string: "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs")!
let irisSourceLabel = "Iris Chat source code"
let irisPrivacyURL = URL(string: "https://chat.iris.to/privacy")!
let irisTermsURL = URL(string: "https://chat.iris.to/terms")!
let irisChildSafetyURL = URL(string: "https://chat.iris.to/csae")!
let irisSupportEmail = "irismessenger@pm.me"
let irisTermsAcceptedDefaultsKey = "legal.termsAccepted.v1"
let chatListRelativeTimeTicker = Timer.publish(every: 30, on: .main, in: .common).autoconnect()
func irisChatProfileURL(npub: String) -> URL {
    URL(string: "https://chat.iris.to/#/\(npub)")!
}
let disappearingMessageOptions: [(String, UInt64?)] = [
    ("Off", nil),
    ("5 minutes", 300),
    ("1 hour", 3_600),
    ("24 hours", 86_400),
    ("1 week", 604_800),
    ("1 month", 2_592_000),
    ("3 months", 7_776_000),
]

// Compact label for the chat header subtitle when disappearing-messages is
// on. Tries the canonical menu options first so the wording matches what
// the user picked, then falls back to a generic unit-based string for any
// odd value that arrives over the wire.
func irisDisappearingLabel(seconds: UInt64) -> String {
    for (label, value) in disappearingMessageOptions {
        if let v = value, v == seconds {
            return label
        }
    }
    if seconds < 3_600 { return "\(seconds / 60) min" }
    if seconds < 86_400 { return "\(seconds / 3_600) h" }
    if seconds < 604_800 { return "\(seconds / 86_400) d" }
    if seconds < 2_592_000 { return "\(seconds / 604_800) wk" }
    return "\(seconds / 2_592_000) mo"
}
let offlineBannerGraceInterval: TimeInterval = 30

func hasHttpPicture(_ url: String?) -> Bool {
    guard let trimmed = url?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else {
        return false
    }
    return trimmed.hasPrefix("http://") || trimmed.hasPrefix("https://")
}

func hasHashtreePicture(_ url: String?) -> Bool {
    guard let trimmed = url?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else {
        return false
    }
    return trimmed.hasPrefix("htree://") || trimmed.hasPrefix("nhash://")
}

#if os(iOS)
struct IrisCameraImagePicker: UIViewControllerRepresentable {
    let onPick: (URL) -> Void

    @Environment(\.dismiss) private var dismiss

    func makeCoordinator() -> Coordinator {
        Coordinator(onPick: onPick, dismiss: dismiss)
    }

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.mediaTypes = ["public.image"]
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ uiViewController: UIImagePickerController, context: Context) {}

    final class Coordinator: NSObject, UINavigationControllerDelegate, UIImagePickerControllerDelegate {
        private let onPick: (URL) -> Void
        private let dismiss: DismissAction

        init(onPick: @escaping (URL) -> Void, dismiss: DismissAction) {
            self.onPick = onPick
            self.dismiss = dismiss
        }

        func imagePickerController(
            _ picker: UIImagePickerController,
            didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
        ) {
            defer { dismiss() }
            guard let image = info[.originalImage] as? UIImage,
                  let data = image.jpegData(compressionQuality: 0.92) else {
                return
            }
            let directory = FileManager.default.temporaryDirectory
                .appendingPathComponent("iris-camera-picks", isDirectory: true)
            try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let url = directory.appendingPathComponent("\(UUID().uuidString).jpg")
            do {
                try data.write(to: url, options: [.atomic])
                onPick(url)
            } catch {}
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            dismiss()
        }
    }
}
#endif

#if canImport(PhotosUI)
func loadPickedPhotoItem(_ item: PhotosPickerItem, directoryName: String) async -> URL? {
    guard let data = try? await item.loadTransferable(type: Data.self) else {
        return nil
    }
    let ext = item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg"
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent(directoryName, isDirectory: true)
    try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let url = directory.appendingPathComponent("\(UUID().uuidString).\(ext)")
    do {
        try data.write(to: url, options: [.atomic])
        return url
    } catch {
        return nil
    }
}
#endif

func irisMailtoURL(to email: String, subject: String, body: String) -> URL? {
    var components = URLComponents()
    components.scheme = "mailto"
    components.path = email
    components.queryItems = [
        URLQueryItem(name: "subject", value: subject),
        URLQueryItem(name: "body", value: body),
    ]
    return components.url
}

@MainActor
private func irisCopyReportDetails(manager: AppManager, body: String) {
    let fallback = """
    To: \(irisSupportEmail)
    Subject: Iris Chat user report

    \(body)
    """
    PlatformClipboard.setString(fallback)
    manager.toasts.show("Report details copied")
}

@MainActor
func irisReportUser(
    manager: AppManager,
    chatId: String,
    displayName: String,
    block: Bool
) {
    if block {
        manager.setUserBlocked(chatId, blocked: true)
    }

    let userId = peerInputToNpub(input: chatId)
    let body = """
    Reported user: \(displayName)
    User ID: \(userId)
    App: Iris Chat \(manager.buildSummaryText())

    What happened:
    """
    guard let url = irisMailtoURL(
        to: irisSupportEmail,
        subject: "Iris Chat user report",
        body: body
    ) else {
        irisCopyReportDetails(manager: manager, body: body)
        return
    }
    PlatformExternalURL.open(url) { opened in
        guard !opened else { return }
        Task { @MainActor in
            irisCopyReportDetails(manager: manager, body: body)
        }
    }
}

/// Identifies the chat the message-request safety dialog is acting on.
/// `Identifiable` lets `.confirmationDialog(item:)` rebuild the sheet
/// when the user changes target without a separate `isPresented` flag.
struct MessageRequestActionTarget: Identifiable {
    let chatId: String
    let displayName: String
    var id: String { chatId }
}

/// Message-request safety actions stay beside Accept so report/block are
/// reachable before a stranger is accepted.
struct MessageRequestSafetyModifier: ViewModifier {
    @Binding var blockTarget: MessageRequestActionTarget?
    @Binding var reportTarget: MessageRequestActionTarget?
    @Binding var deleteTarget: MessageRequestActionTarget?
    let manager: AppManager

    func body(content: Content) -> some View {
        content
        .confirmationDialog(
            blockTarget.map { "Block \($0.displayName)?" } ?? "Block?",
            isPresented: Binding(
                get: { blockTarget != nil },
                set: { presented in
                    if !presented { blockTarget = nil }
                }
            ),
            titleVisibility: .visible,
            presenting: blockTarget,
            actions: { item in
                Button("Block", role: .destructive) {
                    manager.setUserBlocked(item.chatId, blocked: true)
                    blockTarget = nil
                }
                .accessibilityIdentifier("messageRequestBlockConfirmKeep")
                Button("Block and report", role: .destructive) {
                    blockTarget = nil
                    irisReportUser(
                        manager: manager,
                        chatId: item.chatId,
                        displayName: item.displayName,
                        block: true
                    )
                }
                .accessibilityIdentifier("messageRequestBlockAndReportButton")
                Button("Delete chat", role: .destructive) {
                    blockTarget = nil
                    deleteTarget = item
                }
                .accessibilityIdentifier("messageRequestBlockDeleteChatButton")
                Button("Cancel", role: .cancel) {
                    blockTarget = nil
                }
                .accessibilityIdentifier("messageRequestBlockCancelButton")
            },
            message: { _ in
                Text("They won't be able to message you. No notification is sent.")
            }
        )
        .confirmationDialog(
            reportTarget.map { "Block and report \($0.displayName)?" } ?? "Block and report?",
            isPresented: Binding(
                get: { reportTarget != nil },
                set: { presented in
                    if !presented { reportTarget = nil }
                }
            ),
            titleVisibility: .visible,
            presenting: reportTarget,
            actions: { item in
                Button("Block and report", role: .destructive) {
                    reportTarget = nil
                    irisReportUser(
                        manager: manager,
                        chatId: item.chatId,
                        displayName: item.displayName,
                        block: true
                    )
                }
                .accessibilityIdentifier("messageRequestBlockAndReportConfirmButton")
                Button("Delete chat", role: .destructive) {
                    reportTarget = nil
                    deleteTarget = item
                }
                .accessibilityIdentifier("messageRequestBlockAndReportDeleteChatButton")
                Button("Cancel", role: .cancel) {
                    reportTarget = nil
                }
                .accessibilityIdentifier("messageRequestBlockAndReportCancelButton")
            },
            message: { _ in
                Text("This prepares a report for support and blocks this user. No notification is sent.")
            }
        )
        .confirmationDialog(
            deleteTarget.map { "Delete chat with \($0.displayName)?" } ?? "Delete chat?",
            isPresented: Binding(
                get: { deleteTarget != nil },
                set: { presented in
                    if !presented { deleteTarget = nil }
                }
            ),
            titleVisibility: .visible,
            presenting: deleteTarget,
            actions: { item in
                Button("Delete chat", role: .destructive) {
                    manager.dispatch(.deleteChat(chatId: item.chatId))
                    manager.navigateBack()
                    deleteTarget = nil
                }
                .accessibilityIdentifier("messageRequestDeleteChatConfirmButton")
                Button("Cancel", role: .cancel) {
                    deleteTarget = nil
                }
                .accessibilityIdentifier("messageRequestDeleteChatCancelButton")
            },
            message: { _ in
                Text("This removes the chat from this device. No notification is sent.")
            }
        )
    }
}

func proxiedImageURL(
    _ rawURL: String?,
    preferences: PreferencesSnapshot,
    width: UInt32? = nil,
    height: UInt32? = nil,
    square: Bool = false
) -> String? {
    guard let rawURL else {
        return nil
    }
    let trimmed = rawURL.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else {
        return nil
    }
    return proxiedImageUrl(
        originalSrc: trimmed,
        preferences: preferences,
        width: width,
        height: height,
        square: square
    )
}

enum SecretExportKind: Identifiable {
    case owner

    var id: String {
        switch self {
        case .owner: return "owner"
        }
    }
}

enum SettingsFocusSection: Hashable {
    case messageServers
    case messaging
}

#if os(iOS)
/// Posted with a `SettingsFocusSection` in `userInfo["focus"]` (or
/// `nil`) when a deep child wants the settings sheet opened on a
/// specific page. `IrisRoot` listens and flips its `@State`.
let irisOpenSettingsNotification = Notification.Name("to.iris.chat.open-settings")
#endif

enum SettingsPage: String, CaseIterable, Identifiable {
    case profile
    case devices
    case messaging
    case notifications
    case media
    case nearby
    case messageServers
    case security
    case updates
    case about
    case legal
    case support
    case accountData

    var id: String { rawValue }

    var title: String {
        switch self {
        case .profile: return "Profile"
        case .devices: return "Devices"
        case .messaging: return "Messaging"
        case .notifications: return "Notifications"
        case .media: return "Media"
        case .nearby: return "Nearby"
        case .messageServers: return "Message servers"
        case .security: return "Keys"
        case .updates: return "Updates"
        case .about: return "About"
        case .legal: return "Legal"
        case .support: return "Support"
        case .accountData: return "Account data"
        }
    }

    var systemImage: String {
        switch self {
        case .profile: return "person.crop.circle.fill"
        case .devices: return "laptopcomputer.and.iphone"
        case .messaging: return "bubble.left.and.bubble.right.fill"
        case .notifications: return "bell.fill"
        case .media: return "photo.fill"
        case .nearby: return "dot.radiowaves.left.and.right"
        case .messageServers: return "server.rack"
        case .security: return "key.fill"
        case .updates: return "arrow.down.circle.fill"
        case .about: return "info.circle.fill"
        case .legal: return "doc.text.fill"
        case .support: return "wrench.and.screwdriver.fill"
        case .accountData: return "trash.fill"
        }
    }

    var accessibilityID: String {
        switch self {
        case .profile: return "settingsProfileRow"
        case .devices: return "settingsDevicesRow"
        case .messaging: return "settingsMessagingRow"
        case .notifications: return "settingsNotificationsRow"
        case .media: return "settingsMediaRow"
        case .nearby: return "settingsNearbyRow"
        case .messageServers: return "settingsMessageServersRow"
        case .security: return "settingsSecurityRow"
        case .updates: return "settingsUpdatesRow"
        case .about: return "settingsAboutRow"
        case .legal: return "settingsLegalRow"
        case .support: return "settingsSupportRow"
        case .accountData: return "settingsAccountDataRow"
        }
    }

    static var primaryMenuPages: [SettingsPage] {
        [
            .notifications,
            .messaging,
            .nearby,
            .devices,
            .security,
        ]
    }

    static var infoMenuPages: [SettingsPage] {
        var pages: [SettingsPage] = []
        #if os(macOS)
        pages.append(.updates)
        #endif
        pages.append(contentsOf: [.support, .about])
        #if os(iOS)
        pages.append(.legal)
        #endif
        pages.append(.accountData)
        return pages
    }

    static var advancedMenuPages: [SettingsPage] {
        [.media, .messageServers]
    }
}
