#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class DesktopSidebarDraftPreviewTests: XCTestCase {
    func testProductionRowUpdatesDraftWithoutReplacingTheChat() throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        var chat = try XCTUnwrap(state.chatList.first)
        chat.displayName = "River"
        chat.profileName = "River"
        chat.pictureUrl = nil
        chat.lastMessagePreview = "Earlier message"
        chat.draft = ""
        chat.isTyping = false
        state.chatList = [chat]
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: MockRustApp(state: state), secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        for dark in [false, true] {
            let host = NSHostingView(rootView: row(manager: manager, chat: chat, dark: dark))
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 350, height: 110),
                styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            window.contentView = host
            window.makeKeyAndOrderFront(nil)
            defer { window.orderOut(nil); window.close() }

            func update(draft: String, typing: Bool = false, expected: String, absent: [String]) throws {
                chat.draft = draft
                chat.isTyping = typing
                host.rootView = row(manager: manager, chat: chat, dark: dark)
                let deadline = Date().addingTimeInterval(2)
                var text = ""
                repeat {
                    host.layoutSubtreeIfNeeded()
                    text = accessibilityText(host)
                    if text.contains(expected) && absent.allSatisfy({ !text.contains($0) }) { return }
                    RunLoop.main.run(until: Date().addingTimeInterval(0.01))
                } while Date() < deadline
                try retain(host, name: "draft-preview-failure-\(dark ? "dark" : "light")")
                XCTFail("Production sidebar preview expected \(expected), excluding \(absent); accessibility text: \(text)")
            }

            try update(draft: "", expected: "Earlier message", absent: ["Draft:"])
            try update(draft: "  Bring a map\n", expected: "Draft: Bring a map", absent: ["Earlier message"])
            try retain(host, name: "sidebar-draft-\(dark ? "dark" : "light")")
            try update(draft: "Bring a compass", expected: "Draft: Bring a compass", absent: ["Bring a map", "Earlier message"])
            try update(draft: " \n\t ", expected: "Earlier message", absent: ["Draft:", "Bring a compass"])
            try update(draft: "Saved plan", typing: true, expected: "Typing", absent: ["Draft:", "Saved plan", "Earlier message"])
            try update(draft: "Saved plan", expected: "Draft: Saved plan", absent: ["Typing", "Earlier message"])
            try update(draft: "", expected: "Earlier message", absent: ["Draft:", "Saved plan"])
        }
    }

    private func row(manager: AppManager, chat: ChatThreadSnapshot, dark: Bool) -> some View {
        DesktopSidebarChatRow(manager: manager, chat: chat, timeLabel: "12:34", selected: false,
            preferences: manager.state.preferences)
            .equatable()
            .frame(width: 350)
            .background(dark ? IrisPalette.dark.background : IrisPalette.light.background)
            .environment(\.irisPalette, dark ? .dark : .light)
            .environment(\.colorScheme, dark ? .dark : .light)
    }

    private func accessibilityText(_ element: Any, depth: Int = 0) -> String {
        guard depth < 12, let accessible = element as? NSObject else { return "" }
        // SwiftUI accessibility nodes expose AppKit getters without adopting
        // the complete NSAccessibilityProtocol.
        func value(_ name: String) -> Any? {
            let selector = NSSelectorFromString(name)
            guard accessible.responds(to: selector) else { return nil }
            return accessible.perform(selector)?.takeUnretainedValue()
        }
        let own = [value("accessibilityLabel") as? String, value("accessibilityValue") as? String].compactMap { $0 }
        let children = (value("accessibilityChildren") as? [Any] ?? []).map { accessibilityText($0, depth: depth + 1) }
        return (own + children).joined(separator: "\n")
    }

    private func retain(_ host: NSView, name: String) throws {
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        if let output = ProcessInfo.processInfo.environment["IRIS_UI_EVIDENCE_DIR"] {
            try png.write(to: URL(fileURLWithPath: output).appendingPathComponent("\(name).png"))
        }
    }
}
#endif
