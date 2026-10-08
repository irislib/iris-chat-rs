#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class BlockedPeopleSettingsTests: XCTestCase {
    func testBlockedPeopleSettingsFitsDesktopAndNarrowWindows() throws {
        var state = buildLargeTestAppState(directChatCount: 0, groupChatCount: 0, messagesInCurrentChat: 0)
        state.blockedPeople = [person("Ada", key: "a"), person("A person with a longer display name", key: "b")]
        let rust = MockRustApp(state: state)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        for width in [360.0, 680.0] {
            let host = NSHostingView(rootView: IrisTheme {
                BlockedPeopleSettings(manager: manager).padding(20).frame(width: width)
            })
            host.setFrameSize(host.fittingSize)
            host.layoutSubtreeIfNeeded()
            XCTAssertEqual(host.frame.width, width, accuracy: 1)
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            let name = "blocked-people-\(Int(width))"
            let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
            attachment.name = name
            attachment.lifetime = .keepAlways
            add(attachment)
            if let output = ProcessInfo.processInfo.environment["IRIS_UI_EVIDENCE_DIR"] {
                try png.write(to: URL(fileURLWithPath: output).appendingPathComponent("\(name).png"))
            }
        }
    }

    private func person(_ name: String, key: String) -> FollowedUserSearchResult {
        FollowedUserSearchResult(socialConnection: nil, ownerPubkeyHex: String(repeating: key, count: 64), displayLabel: name,
            profileLabel: name, pictureUrl: nil, about: nil,
            userId: "npub1" + String(repeating: key, count: 58))
    }
}
#endif
