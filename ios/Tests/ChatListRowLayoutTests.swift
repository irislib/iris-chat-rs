import SwiftUI
import XCTest
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
import UIKit
@testable import IrisChat
#endif

@MainActor
final class ChatListRowLayoutTests: XCTestCase {
    private let previews = ["", "Short preview", String(repeating: "A preview that wraps onto another line. ", count: 5), "One\nTwo\nThree", "Hello 👋 🎉"]

    private func height<V: View>(_ view: V, width: CGFloat, type: DynamicTypeSize, screenshot: String? = nil) throws -> CGFloat {
        let renderer = ImageRenderer(content: IrisTheme {
            view.frame(width: width).fixedSize(horizontal: false, vertical: true).dynamicTypeSize(type)
        })
        renderer.scale = 1
        let image = try XCTUnwrap(renderer.cgImage, "A chat row must render")
        if let screenshot {
#if os(macOS)
            let png = try XCTUnwrap(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
#else
            let png = try XCTUnwrap(UIImage(cgImage: image).pngData())
#endif
            let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
            attachment.name = screenshot
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        return CGFloat(image.height)
    }

    func testSharedChatRowsReservePreviewSpaceAtDifferentWidthsAndTextSizes() throws {
        for width in [CGFloat(280), CGFloat(352), CGFloat(520)] {
            for type in [DynamicTypeSize.large, .accessibility2] {
                var expected: CGFloat?
                for (index, preview) in previews.enumerated() {
                    for draft in [false, true] {
                        let view = IrisChatRow(title: "Contact", preview: preview,
                            draftPreview: draft ? preview : nil, subtitle: nil,
                            timeLabel: "Now", unreadCount: draft ? 123 : 0) {}
                        let measured = try height(view, width: width, type: type,
                            screenshot: width == 352 && type == .large && !draft && index < 3 ? "chat-row-preview-\(index)" : nil)
                        expected = expected ?? measured
                        XCTAssertGreaterThan(measured, 0)
                        XCTAssertEqual(measured, try XCTUnwrap(expected), accuracy: 1,
                            "Preview/draft length moved a shared row at width \(width), text size \(type)")
                    }
                }
            }
        }
    }

#if os(macOS)
    func testMacSidebarRowsReserveSpaceAcrossMessageTypingAndDraftUpdates() throws {
        let state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        let client = MockRustApp(state: state)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: client, secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        let original = try XCTUnwrap(state.chatList.first)
        for width in [CGFloat(280), CGFloat(352), CGFloat(520)] {
            for type in [DynamicTypeSize.large, .accessibility2] {
                var expected: CGFloat?
                for (index, preview) in previews.enumerated() {
                    for draft in [false, true] {
                        for typing in [false, true] {
                            var chat = original
                            chat.lastMessagePreview = preview
                            chat.draft = draft ? preview : ""
                            chat.isTyping = typing
                            chat.unreadCount = draft ? 123 : 0
                            let view = DesktopSidebarChatRow(manager: manager, chat: chat, timeLabel: "Now",
                                selected: false, preferences: state.preferences)
                            let measured = try height(view, width: width, type: type,
                                screenshot: width == 352 && type == .large && !draft && !typing && index < 3 ? "mac-sidebar-preview-\(index)" : nil)
                            expected = expected ?? measured
                            XCTAssertGreaterThan(measured, 0)
                            XCTAssertEqual(measured, try XCTUnwrap(expected), accuracy: 1,
                                "Preview/typing/draft changed the sidebar row height at width \(width), text size \(type)")
                        }
                    }
                }
            }
        }
    }
#endif
}
