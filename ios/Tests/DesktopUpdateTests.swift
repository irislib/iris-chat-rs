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
        let diagnostic = longDiscoveryFailure
        let updates = DesktopUpdateController(defaults: defaults, fetchUpdate: {
            self.result(error: failed ? diagnostic : nil)
        }, now: { checkedAt })

        await updates.check(manual: false)?.value
        XCTAssertFalse(updates.checking)
        XCTAssertEqual(updates.status, "Couldn’t check for updates. Try again.")
        XCTAssertEqual(defaults.string(forKey: "updates.lastCheckStatus"), updates.status)
        XCTAssertEqual(updates.diagnostics.last?.detail, diagnostic)
        XCTAssertEqual(updates.diagnostics.last?.category, "updates.check.failed")
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

    func testSavedRawFailureIsHiddenImmediatelyAndRetainedForSupport() throws {
        let suite = "DesktopUpdateTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let diagnostic = longDiscoveryFailure
        defaults.set("Couldn’t check for updates: \(diagnostic)", forKey: "updates.lastCheckStatus")
        let updates = DesktopUpdateController(defaults: defaults)

        XCTAssertEqual(updates.status, "Couldn’t check for updates. Try again.")
        XCTAssertFalse(updates.checking)
        XCTAssertEqual(defaults.string(forKey: "updates.lastCheckStatus"), updates.status)
        XCTAssertEqual(updates.diagnostics.last?.jsonObject["detail"] as? String, diagnostic)
        try captureSettings(updates, name: "desktop-update-saved-failure")
    }

    func testFailedRefreshKeepsPreviouslyVerifiedUpdateAvailable() async throws {
        let suite = "DesktopUpdateTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        var failed = false
        let updates = DesktopUpdateController(defaults: defaults, fetchUpdate: {
            self.result(available: true, error: failed ? self.longDiscoveryFailure : nil)
        })
        await updates.check()?.value
        XCTAssertTrue(updates.available)
        let verifiedVersion = updates.version

        failed = true
        await updates.check()?.value
        XCTAssertEqual(updates.status, "Couldn’t check for updates. Try again.")
        XCTAssertTrue(updates.available)
        XCTAssertTrue(updates.canInstall)
        XCTAssertEqual(updates.version, verifiedVersion)
        try captureSettings(updates, name: "desktop-update-available-refresh-failed")
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
        XCTAssertEqual(updates.status, "Update could not be verified.")
        XCTAssertNotNil(updates.lastCheckedAt)
    }

    func testInstallFailureIsVisibleAndRetryUsesVerifiedDownload() async throws {
        let suite = "DesktopUpdateTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        var failDownload = true
        var installed: URL?
        let archive = URL(fileURLWithPath: "/tmp/update.app.tar.gz")
        let updates = DesktopUpdateController(defaults: defaults, fetchUpdate: {
            self.result(available: true)
        }, downloadUpdate: {
            if failDownload { throw URLError(.networkConnectionLost) }
            return archive
        }, applyDownload: { installed = $0 })
        await updates.check()?.value
        await updates.install()?.value
        XCTAssertFalse(updates.installing)
        XCTAssertTrue(updates.canInstall)
        XCTAssertEqual(updates.bannerStatus, "Couldn’t install the update. Try again.")
        XCTAssertEqual(updates.diagnostics.last?.category, "updates.install.failed")
        await updates.check(manual: false)?.value
        XCTAssertEqual(updates.bannerStatus, "Couldn’t install the update. Try again.")
        let host = NSHostingView(rootView: IrisTheme { DesktopUpdateStripe(updates: updates).frame(width: 720) })
        host.setFrameSize(host.fittingSize)
        try capture(host, name: "desktop-update-install-failed")
        failDownload = false
        await updates.install()?.value
        XCTAssertEqual(installed, archive)
        XCTAssertNil(updates.installFailure)
    }

    func testCheckCannotReplaceAnActiveInstall() async throws {
        let suite = "DesktopUpdateTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        var resume: CheckedContinuation<URL, Error>?
        let updates = DesktopUpdateController(defaults: defaults, fetchUpdate: {
            self.result(available: true)
        }, downloadUpdate: {
            try await withCheckedThrowingContinuation { resume = $0 }
        }, applyDownload: { _ in })
        await updates.check()?.value
        let installation = updates.install()
        while resume == nil { await Task.yield() }
        XCTAssertNil(updates.check())
        XCTAssertNil(updates.install())
        XCTAssertEqual(updates.bannerStatus, "Downloading v2026.10.8.2")
        resume?.resume(returning: URL(fileURLWithPath: "/tmp/update.app.tar.gz"))
        await installation?.value
    }

    private var longDiscoveryFailure: String {
        "failed to resolve signed release: no current peer observation\n" +
            (1...40).map { "Attempt \($0): signed release lookup failed; peer observation unavailable." }
                .joined(separator: "\n")
    }

    private func result(available: Bool = false, error: String? = nil) -> IrisDesktopUpdateResult {
        IrisDesktopUpdateResult(ok: error == nil, error: error, available: available,
            currentVersion: "2026.10.7", latestVersion: "2026.10.8.2", tag: "v2026.10.8.2",
            asset: "iris-chat-v2026.10.8.2-macos-arm64.app.tar.gz", source: "hashtree-nostr-blossom",
            verified: true, url: nil, path: nil)
    }

    private func captureSettings(_ updates: DesktopUpdateController, name: String) throws {
        let host = NSHostingView(rootView: IrisTheme {
            DesktopUpdateSettingsSection(buildSummary: "2026.10.7", updates: updates)
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
