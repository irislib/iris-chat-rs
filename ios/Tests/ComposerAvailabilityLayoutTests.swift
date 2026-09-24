#if os(iOS)
import SwiftUI
import XCTest
@testable import IrisChat

@MainActor
final class ComposerAvailabilityLayoutTests: XCTestCase {
    func testCheckingKeepsNativeEditorDraftFocusAndFrame() async throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 2)
        state.currentChat?.directChatCapability = .checking
        state.currentChat?.isRequest = false
        let chatId = try XCTUnwrap(state.currentChat?.chatId)
        let rust = MockRustApp(state: state)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        let host = UIHostingController(rootView: ChatScreen(manager: manager, chatId: chatId))
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        try await Task.sleep(nanoseconds: 200_000_000)
        window.layoutIfNeeded()
        let editor = try XCTUnwrap(findEditor(host.view))
        XCTAssertTrue(editor.isEditable)
        editor.becomeFirstResponder()
        editor.text = "See you soon"
        editor.delegate?.textViewDidChange?(editor)
        try await Task.sleep(nanoseconds: 400_000_000)
        let frame = editor.convert(editor.bounds, to: window)
        try await Task.sleep(nanoseconds: 2_100_000_000)
        window.layoutIfNeeded()
        XCTAssertTrue(findEditor(host.view) === editor)
        XCTAssertEqual(editor.text, "See you soon")
        XCTAssertTrue(editor.isFirstResponder)
        XCTAssertEqual(editor.convert(editor.bounds, to: window), frame)
        attach(window, name: "checking-composer")

        state.rev += 1
        state.currentChat?.directChatCapability = .available
        rust.emit(.fullState(state))
        try await Task.sleep(nanoseconds: 300_000_000)
        window.layoutIfNeeded()
        XCTAssertTrue(findEditor(host.view) === editor)
        XCTAssertEqual(editor.text, "See you soon")
        XCTAssertTrue(editor.isFirstResponder)
        XCTAssertEqual(editor.convert(editor.bounds, to: window), frame)
        if #available(iOS 17.0, *) {
            let regularSize = try XCTUnwrap(editor.font?.pointSize)
            window.traitOverrides.preferredContentSizeCategory = .accessibilityExtraLarge
            try await Task.sleep(nanoseconds: 300_000_000)
            window.layoutIfNeeded()
            XCTAssertGreaterThan(try XCTUnwrap(editor.font?.pointSize), regularSize)
            XCTAssertGreaterThanOrEqual(editor.bounds.height, try XCTUnwrap(editor.font?.lineHeight))
            attach(window, name: "large-type-composer")
        }
    }

    private func findEditor(_ view: UIView) -> UITextView? {
        if let editor = view as? UITextView, editor.accessibilityIdentifier == "chatMessageInput" { return editor }
        return view.subviews.lazy.compactMap(findEditor).first
    }

    private func attach(_ window: UIWindow, name: String) {
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
#endif
