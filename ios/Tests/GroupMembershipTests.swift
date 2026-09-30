import SwiftUI
import XCTest
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

@MainActor
final class GroupMembershipTests: XCTestCase {
    func testRemovalBlocksStagedOutgoingActionsAndRejoiningRestoresSending() async throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 2)
        state.currentChat?.kind = .group
        state.currentChat?.chatId = "group:membership-test"
        state.currentChat?.groupId = "membership-test"
        state.currentChat?.displayName = "Weekend plans"
        state.currentChat?.directChatCapability = nil
        state.currentChat?.participants = [ChatParticipantSnapshot(ownerPubkeyHex: "local", displayName: "You", pictureUrl: nil, isLocalOwner: true)]
        let chatId = try XCTUnwrap(state.currentChat?.chatId)
        let rust = MockRustApp(state: state)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        XCTAssertFalse(try XCTUnwrap(manager.state.currentChat).isRemovedFromGroup)
#if os(macOS)
        let host = NSHostingView(rootView: IrisTheme { ChatScreen(manager: manager, chatId: chatId) })
        host.frame = NSRect(x: 0, y: 0, width: 680, height: 600)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = host
        defer { window.orderOut(nil) }
#else
        let host = UIHostingController(rootView: IrisTheme { ChatScreen(manager: manager, chatId: chatId) })
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
#endif
        try await Task.sleep(nanoseconds: 100_000_000)
#if os(macOS)
        host.layoutSubtreeIfNeeded()
        XCTAssertTrue(containsEditor(host))
#else
        window.layoutIfNeeded()
        XCTAssertTrue(containsEditor(host.view))
#endif
        state.rev += 1
        state.currentChat?.participants = []
        rust.emit(.fullState(state))
        try await Task.sleep(nanoseconds: 200_000_000)
        XCTAssertTrue(try XCTUnwrap(manager.state.currentChat).isRemovedFromGroup)
        rust.clearDispatchedActions()
        manager.dispatch(.sendMessage(chatId: chatId, text: "staged text"))
        manager.dispatch(.sendDisappearingMessage(chatId: chatId, text: "staged text", expiresAtSecs: 100))
        manager.dispatch(.sendAttachment(chatId: chatId, filePath: "/unused", filename: "voice.m4a", caption: ""))
        manager.dispatch(.sendAttachments(chatId: chatId, attachments: [], caption: "staged files"))
        manager.dispatch(.toggleReaction(chatId: chatId, messageId: "message", emoji: "👍"))
        manager.dispatch(.sendTyping(chatId: chatId))
        XCTAssertTrue(rust.dispatchedActions.isEmpty)
        XCTAssertEqual(manager.state.currentChat?.messages, state.currentChat?.messages)
#if os(macOS)
        host.layoutSubtreeIfNeeded()
        XCTAssertFalse(containsEditor(host))
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
#else
        window.layoutIfNeeded()
        XCTAssertFalse(containsEditor(host.view))
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let png = try XCTUnwrap(image.pngData())
#endif
        let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
        attachment.name = "removed-group-chat"
        attachment.lifetime = .keepAlways
        add(attachment)
        if let output = ProcessInfo.processInfo.environment["IRIS_UI_EVIDENCE_DIR"] {
            try png.write(to: URL(fileURLWithPath: output).appendingPathComponent("removed-group-chat.png"))
        }
        state.rev += 1
        state.currentChat?.participants = [ChatParticipantSnapshot(ownerPubkeyHex: "local", displayName: "You", pictureUrl: nil, isLocalOwner: true)]
        rust.emit(.fullState(state))
        try await Task.sleep(nanoseconds: 100_000_000)
        rust.clearDispatchedActions()
        manager.dispatch(.sendMessage(chatId: chatId, text: "back again"))
        XCTAssertTrue(rust.dispatchedActions.contains { if case .sendMessage = $0 { return true }; return false })
    }

#if os(macOS)
    private func containsEditor(_ view: NSView) -> Bool {
        (view as? NSTextView)?.isEditable == true || view.subviews.contains(where: containsEditor)
    }
#else
    private func containsEditor(_ view: UIView) -> Bool {
        (view as? UITextView)?.isEditable == true || view.subviews.contains(where: containsEditor)
    }
#endif
}
