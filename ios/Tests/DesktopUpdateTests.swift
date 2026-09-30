#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class DesktopUpdateTests: XCTestCase {
    func testAutomaticFailureIsVisibleAndSuccessfulRetryReplacesIt() async throws {
        let suite = "DesktopUpdateTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        var failed = true
        let checkedAt = Date(timeIntervalSince1970: 1_790_750_000)
        let updates = DesktopUpdateController(defaults: defaults, fetchUpdate: {
            self.result(error: failed ? "Message server unavailable" : nil)
        }, now: { checkedAt })

        await updates.check(manual: false)?.value
        XCTAssertFalse(updates.checking)
        XCTAssertTrue(updates.status.contains("Message server unavailable"))
        XCTAssertEqual(updates.lastCheckedAt, checkedAt)
        try captureSettings(updates, name: "desktop-update-check-failed")

        failed = false
        await updates.check(manual: false)?.value
        XCTAssertEqual(updates.status, "Up to date")
        XCTAssertEqual(updates.lastCheckedAt, checkedAt)
        let restored = DesktopUpdateController(defaults: defaults)
        XCTAssertEqual(restored.status, updates.status)
        XCTAssertEqual(restored.lastCheckedAt, checkedAt)
    }

    func testUpdateBannerAppearsWithoutParentStateChanges() async throws {
        let suite = "DesktopUpdateTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let updates = DesktopUpdateController(defaults: defaults, fetchUpdate: { self.result(available: true) })
        let host = NSHostingView(rootView: IrisTheme { DesktopUpdateStripe(updates: updates).frame(width: 720) })
        host.layoutSubtreeIfNeeded()
        XCTAssertEqual(host.fittingSize.height, 0, accuracy: 1)

        await updates.check(manual: false)?.value
        await Task.yield()
        host.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(host.fittingSize.height, 20)
        XCTAssertTrue(updates.canInstall)
        host.setFrameSize(host.fittingSize)
        try capture(host, name: "desktop-update-available")
    }

    func testUnverifiedAutomaticUpdateShowsFailureAndCannotInstall() async throws {
        let suite = "DesktopUpdateTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let updates = DesktopUpdateController(defaults: defaults, fetchUpdate: {
            var result = self.result(available: true)
            result.verified = false
            return result
        })
        await updates.check(manual: false)?.value
        XCTAssertFalse(updates.available)
        XCTAssertFalse(updates.canInstall)
        XCTAssertTrue(updates.status.contains("could not be verified"))
        XCTAssertNotNil(updates.lastCheckedAt)
    }

    private func result(available: Bool = false, error: String? = nil) -> IrisDesktopUpdateResult {
        IrisDesktopUpdateResult(ok: error == nil, error: error, available: available,
            currentVersion: "2026.9.24.4", latestVersion: "2026.9.29", tag: "v2026.9.29",
            asset: "iris-chat-v2026.9.29-macos-arm64.app.tar.gz", source: "hashtree-nostr-blossom",
            verified: true, url: nil, path: nil)
    }

    private func captureSettings(_ updates: DesktopUpdateController, name: String) throws {
        let host = NSHostingView(rootView: IrisTheme {
            DesktopUpdateSettingsSection(buildSummary: "2026.9.24.4", updates: updates)
                .padding(24).frame(width: 620)
        })
        host.setFrameSize(host.fittingSize)
        try capture(host, name: name)
    }

    private func capture(_ host: NSView, name: String) throws {
        host.layoutSubtreeIfNeeded()
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
