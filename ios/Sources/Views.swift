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
