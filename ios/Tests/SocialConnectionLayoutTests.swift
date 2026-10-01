#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class SocialConnectionLayoutTests: XCTestCase {
    func testRenderAvatarAndProfileConnections() throws {
        let connections = [
            SocialConnectionSnapshot(badge: .following, followDistance: 1, followedByFriends: 0, description: "Followed by you"),
            SocialConnectionSnapshot(badge: .friend, followDistance: 2, followedByFriends: 3, description: "Followed by 3 friends"),
            SocialConnectionSnapshot(badge: .trusted, followDistance: 2, followedByFriends: 12, description: "Followed by 12 friends"),
            SocialConnectionSnapshot(badge: nil, followDistance: 3, followedByFriends: 0, description: "Followed by friends of friends"),
            SocialConnectionSnapshot(badge: .warning, followDistance: 2, followedByFriends: 1, description: "More mutes than follows in your network"),
        ]
        for dark in [false, true] {
            let view = VStack(alignment: .leading, spacing: 24) {
                ForEach(Array(connections.enumerated()), id: \.offset) { index, connection in
                    HStack(spacing: 16) {
                        IrisAvatar(socialConnection: connection, label: ["Alice", "Bob", "Charlie", "Diana", "Eve"][index], size: 56)
                        VStack(alignment: .leading, spacing: 6) {
                            Text(["Alice", "Bob", "Charlie", "Diana", "Eve"][index]).font(.headline)
                            IrisSocialConnectionLabel(connection: connection)
                        }
                    }
                }
            }
            .padding(28)
            .frame(width: 430)
            .background(dark ? Color.black : Color.white)
            .environment(\.colorScheme, dark ? .dark : .light)
            .environment(\.irisPalette, dark ? .dark : .light)
            let renderer = ImageRenderer(content: view)
            renderer.scale = 2
            let image = try XCTUnwrap(renderer.nsImage)
            let data = try XCTUnwrap(image.tiffRepresentation)
            let png = try XCTUnwrap(NSBitmapImageRep(data: data)?.representation(using: .png, properties: [:]))
            let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
            attachment.name = dark ? "social-connections-dark" : "social-connections-light"
            attachment.lifetime = .keepAlways
            add(attachment)
            if let output = ProcessInfo.processInfo.environment["IRIS_VISUAL_OUTPUT"] {
                try png.write(to: URL(fileURLWithPath: output).appendingPathComponent("social-\(dark ? "dark" : "light").png"))
            }
        }
    }
}
#endif

#if os(macOS)
extension SocialConnectionLayoutTests {
    @MainActor
    func testRenderProfileWithPrivateFavoriteAndNameApproval() throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 2)
        state.currentChat?.displayName = "Alice"
        state.currentChat?.pictureUrl = nil
        state.currentChat?.about = "Coffee, good books, and long walks."
        state.currentChat?.socialConnection = SocialConnectionSnapshot(badge: .friend, followDistance: 2, followedByFriends: 3, description: "Followed by 3 friends")
        state.currentChat?.contactIdentity = ContactIdentitySnapshot(isFollowing: false, canFollow: true, updatingFollow: false, firstSeenName: "Alice", savedName: "Alice", pendingName: "Alice Cooper", isFavorite: true)
        let rust = MockRustApp(state: state)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        let chat = try XCTUnwrap(state.currentChat)
        let connection = try XCTUnwrap(chat.socialConnection)
        let view = VStack(alignment: .leading, spacing: 22) {
            HStack(spacing: 20) {
                IrisAvatar(socialConnection: chat.socialConnection, label: chat.displayName, size: 80)
                VStack(alignment: .leading, spacing: 8) {
                    Text(chat.displayName).font(.title2.bold())
                    IrisSocialConnectionLabel(connection: connection)
                }
            }
            Text(chat.about ?? "")
            IrisContactActions(manager: manager, chat: chat)
        }
        .padding(28).frame(width: 360).background(Color.white)
        .environment(\.colorScheme, .light).environment(\.irisPalette, .light)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        let image = try XCTUnwrap(renderer.nsImage)
        let data = try XCTUnwrap(image.tiffRepresentation)
        let png = try XCTUnwrap(NSBitmapImageRep(data: data)?.representation(using: .png, properties: [:]))
        let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
        attachment.name = "contact-profile-actions"
        if let output = ProcessInfo.processInfo.environment["IRIS_VISUAL_OUTPUT"] {
            try png.write(to: URL(fileURLWithPath: output).appendingPathComponent("contact-profile.png"))
        }
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
#endif
