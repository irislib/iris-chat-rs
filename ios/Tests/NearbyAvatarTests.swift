#if os(iOS)
import SwiftUI
import XCTest
@testable import IrisChat

@MainActor
final class NearbyAvatarTests: XCTestCase {
    func testNearbyIdentityUsesCurrentPrivateContactSnapshot() throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        let owner = try XCTUnwrap(state.chatList.first?.chatId)
        state.chatList[0].socialConnection = SocialConnectionSnapshot(
            badge: nil, followDistance: nil, followedByFriends: 0,
            description: "Favorite · Only you", isFavorite: true
        )
        XCTAssertEqual(nearbyPeerChat(owner: " \(owner.uppercased()) ", chats: state.chatList)?.socialConnection?.isFavorite, true)
        state.chatList[0].socialConnection?.isFavorite = false
        XCTAssertEqual(nearbyPeerChat(owner: owner, chats: state.chatList)?.socialConnection?.isFavorite, false)
        XCTAssertNil(nearbyPeerChat(owner: nil, chats: state.chatList))
        XCTAssertNil(nearbyPeerChat(owner: " ", chats: state.chatList))
        XCTAssertNil(nearbyPeerChat(owner: "unknown", chats: state.chatList))
        state.chatList[0].kind = .group
        XCTAssertNil(nearbyPeerChat(owner: owner, chats: state.chatList))
        XCTAssertNil(nearbyPeerChat(owner: owner, chats: []))
    }

    func testNearbyOwnersRequireEnabledLivePeerAndExcludeSelf() {
        let peers = [peer("alice"), peer("self"), peer(nil)]
        XCTAssertEqual(irisNearbyAvatarOwners(peers: peers, isActive: true, enabled: true, localOwner: "self"), ["alice"])
        XCTAssertEqual(irisNearbyAvatarOwners(peers: peers, isActive: false, enabled: true, localOwner: "self"), [])
        XCTAssertEqual(irisNearbyAvatarOwners(peers: peers, isActive: true, enabled: false, localOwner: "self"), [])
        XCTAssertEqual(irisNearbyAvatarOwners(peers: peers, isActive: true, enabled: true, localOwner: nil), [])
        XCTAssertEqual(irisNearbyAvatarOwners(peers: peers, isActive: true, enabled: true, localOwner: " "), [])
        XCTAssertEqual(irisNearbyAvatarOwners(peers: [], isActive: true, enabled: true, localOwner: "self"), [])
    }

    func testChatHeaderAndListShowNearbyWithIdentityBadge() async throws {
        var state = buildLargeTestAppState(directChatCount: 2, groupChatCount: 1, messagesInCurrentChat: 2)
        let owner = try XCTUnwrap(state.currentChat?.chatId)
        let social = SocialConnectionSnapshot(badge: .following, followDistance: 1, followedByFriends: 0, description: "Followed by you")
        state.currentChat?.displayName = "Alice"
        state.currentChat?.socialConnection = social
        state.currentChat?.pictureUrl = nil
        state.currentChat?.isRequest = false
        state.currentChat?.directChatCapability = .available
        state.preferences.nearbyEnabled = true
        state.preferences.nearbyShowInChatList = false
        state.router = Router(defaultScreen: .chatList, screenStack: [.chat(chatId: owner)])
        for index in state.chatList.indices {
            state.chatList[index].pictureUrl = nil
            if state.chatList[index].chatId == owner {
                state.chatList[index].displayName = "Alice"
                state.chatList[index].socialConnection = social
            }
        }
        let rust = MockRustApp(state: state)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        manager.nearbyIris.applyScreenshotFixturePeers(peers: [peer(owner)], bluetoothPeerIDs: [], lanPeerIDs: ["device"])
        let host = UIHostingController(rootView: RootView(manager: manager).preferredColorScheme(.light))
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 375, height: 812)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        try await Task.sleep(nanoseconds: 500_000_000)
        attach(window, name: "ios-nearby-chat-header")

        state.rev += 1
        state.router.screenStack = []
        rust.emit(.fullState(state))
        try await Task.sleep(nanoseconds: 400_000_000)
        attach(window, name: "ios-nearby-chat-list")
        let table = try XCTUnwrap(findTable(host.view))
        let cell = try XCTUnwrap(table.visibleCells.first { $0.accessibilityLabel?.contains("Alice") == true })
        XCTAssertTrue(cell.accessibilityLabel?.contains("Nearby") == true)
        manager.nearbyIris.applyScreenshotFixturePeers(peers: [], bluetoothPeerIDs: [], lanPeerIDs: [])
        try await Task.sleep(nanoseconds: 300_000_000)
        XCTAssertFalse(table.visibleCells.first { $0.accessibilityLabel?.contains("Alice") == true }?.accessibilityLabel?.contains("Nearby") == true)
        attach(window, name: "ios-nearby-cleared")
    }

    func testGroupMessageSenderBadgeUpdatesInsideEquatableRow() async throws {
        var state = buildLargeTestAppState(directChatCount: 0, groupChatCount: 1, messagesInCurrentChat: 1)
        let chatId = try XCTUnwrap(state.currentChat?.chatId)
        let social = SocialConnectionSnapshot(badge: .following, followDistance: 1, followedByFriends: 0, description: "Followed by you")
        state.preferences.nearbyEnabled = true
        state.currentChat?.displayName = "Weekend plans"
        state.currentChat?.pictureUrl = nil
        state.currentChat?.messageTtlSeconds = nil
        state.currentChat?.draft = ""
        state.currentChat?.messages[0].kind = .user
        state.currentChat?.messages[0].author = "Alice"
        state.currentChat?.messages[0].authorOwnerPubkeyHex = "alice"
        state.currentChat?.messages[0].isOutgoing = false
        state.currentChat?.messages[0].body = "See you at the park!"
        state.currentChat?.messages[0].reactions = []
        state.currentChat?.messages[0].expiresAtSecs = nil
        state.currentChat?.participants = [
            ChatParticipantSnapshot(socialConnection: social, ownerPubkeyHex: "alice", displayName: "Alice", pictureUrl: nil, isLocalOwner: false),
            ChatParticipantSnapshot(socialConnection: nil, ownerPubkeyHex: try XCTUnwrap(state.account?.publicKeyHex), displayName: "You", pictureUrl: nil, isLocalOwner: true),
        ]
        state.router = Router(defaultScreen: .chatList, screenStack: [.chat(chatId: chatId)])
        let rust = MockRustApp(state: state)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        manager.nearbyIris.applyScreenshotFixturePeers(peers: [peer("alice")], bluetoothPeerIDs: [], lanPeerIDs: ["device"])
        let host = UIHostingController(rootView: RootView(manager: manager).preferredColorScheme(.light))
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 375, height: 812)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        try await Task.sleep(nanoseconds: 500_000_000)
        let nearbyImage = attach(window, name: "ios-nearby-group-sender")
        // No AppState/message update: only the badge child observes Nearby.
        manager.nearbyIris.applyScreenshotFixturePeers(peers: [], bluetoothPeerIDs: [], lanPeerIDs: [])
        try await Task.sleep(nanoseconds: 300_000_000)
        let clearedImage = attach(window, name: "ios-nearby-group-sender-cleared")
        XCTAssertNotEqual(nearbyImage, clearedImage)
        XCTAssertEqual(manager.state.currentChat?.messages, state.currentChat?.messages)
    }

    private func peer(_ owner: String?) -> IrisNearbyPeer {
        IrisNearbyPeer(id: "device", name: "Alice", ownerPubkeyHex: owner, pictureURL: nil, profileEventID: nil, bluetoothRSSI: nil)
    }

    private func findTable(_ view: UIView) -> UITableView? {
        if let table = view as? UITableView { return table }
        return view.subviews.lazy.compactMap(findTable).first
    }

    @discardableResult
    private func attach(_ window: UIWindow, name: String) -> Data? {
        window.layoutIfNeeded()
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        if let output = ProcessInfo.processInfo.environment["IRIS_UI_EVIDENCE_DIR"], let png = image.pngData() {
            try? png.write(to: URL(fileURLWithPath: output).appendingPathComponent(name + ".png"))
        }
        return image.pngData()
    }
}
#endif
