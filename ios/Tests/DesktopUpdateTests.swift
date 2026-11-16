#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

/// The Rust core owns the update flow and its words (see
/// core/src/desktop_update/flow_tests.rs for the end-to-end states). These
/// tests check that the Mac shell shows each view the core sends.
@MainActor
final class DesktopUpdateTests: XCTestCase {
    private var suite = ""
    private var stateDir: URL!

    override func setUp() async throws {
        suite = "DesktopUpdateTests.\(UUID().uuidString)"
        stateDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("DesktopUpdateTests-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() async throws {
        UserDefaults().removePersistentDomain(forName: suite)
        try? FileManager.default.removeItem(at: stateDir)
        unsetenv("IRIS_UPDATE_MANIFEST_URL")
    }

    func testCheckFailureFromTheCoreIsShownWithRetry() async throws {
        let missing = stateDir.appendingPathComponent("missing-release.json")
        setenv("IRIS_UPDATE_MANIFEST_URL", missing.absoluteString, 1)
        let updates = controller()

        updates.check()
        try await waitFor(updates) { $0.step == .failed }
        XCTAssertEqual(updates.view.status, "Couldn’t check for updates")
        XCTAssertEqual(updates.view.detail, "Check your connection and try again.")
        XCTAssertEqual(updates.view.actionLabel, "Retry")
        XCTAssertNotNil(updates.view.lastChecked)
        XCTAssertEqual(updates.diagnostics.last?.category, "updates.failed")
        try captureSettings(updates, name: "desktop-update-check-failed")

        // The next launch shows the same result without checking again.
        let restored = controller()
        XCTAssertEqual(restored.view.status, updates.view.status)
        XCTAssertEqual(restored.view.lastCheckedSecs, updates.view.lastCheckedSecs)
    }

    func testEveryInstallStepIsShownInTheStripAndSettings() throws {
        let tag = "v2026.11.17"
        let steps: [(String, DesktopUpdateStage)] = [
            ("available", .available(version: tag)),
            ("fetching", .fetching(version: tag)),
            ("downloading", .downloading(version: tag, downloadedBytes: 12_400_000, totalBytes: 36_200_000)),
            ("verifying", .verifying(version: tag)),
            ("installing", .installing(version: tag)),
            ("restarting", .restarting(version: tag, installerPath: nil)),
        ]
        for (name, stage) in steps {
            let updates = preview(stage)
            XCTAssertTrue(updates.view.showBanner, name)
            if name == "available" {
                XCTAssertEqual(updates.view.actionLabel, "Install")
                XCTAssertFalse(updates.view.busy)
            } else {
                XCTAssertTrue(updates.view.busy, name)
                XCTAssertNil(updates.view.actionLabel, name)
            }
            try captureStrip(updates, name: "desktop-update-strip-\(name)")
            try captureSettings(updates, name: "desktop-update-settings-\(name)")
        }
        let downloading = preview(steps[2].1).view
        XCTAssertEqual(downloading.status, "Downloading update v2026.11.17…")
        XCTAssertEqual(downloading.detail, "12.4 MB of 36.2 MB")
        XCTAssertEqual(downloading.progress ?? 0, 0.3425, accuracy: 0.001)
    }

    func testEveryFailureSaysWhatWentWrongAndOffersRetry() throws {
        let tag = "v2026.11.17"
        let failures: [(String, DesktopUpdateFailure, String?, String)] = [
            ("fetch-timed-out", .fetchTimedOut, nil, "Couldn’t get the update"),
            ("download-stalled", .downloadStalled, nil, "The download stopped"),
            ("damaged", .verifyFailed, nil, "The download was damaged"),
            ("no-permission", .noPermission, "Applications", "Couldn’t replace Iris Chat"),
            ("did-not-finish", .didNotFinish, "v2026.11.13.6", "The update didn’t finish"),
        ]
        for (name, failure, context, status) in failures {
            let updates = preview(.failed(version: tag, failure: failure, context: context))
            XCTAssertEqual(updates.view.status, status, name)
            XCTAssertNotNil(updates.view.detail, name)
            XCTAssertEqual(updates.view.actionLabel, "Retry", name)
            XCTAssertTrue(updates.view.showBanner, name)
            XCTAssertFalse(updates.view.busy, name)
            try captureStrip(updates, name: "desktop-update-strip-failed-\(name)")
            try captureSettings(updates, name: "desktop-update-settings-failed-\(name)")
        }
    }

    func testQuitsOnceWhenTheCoreIsReadyToRestart() {
        var quits = 0
        let updates = controller(quit: { quits += 1 })
        let restarting = desktopUpdateView(snapshot: DesktopUpdateSnapshot(
            stage: .restarting(version: "v2026.11.17", installerPath: nil),
            lastCheckedSecs: nil,
            diagnostic: nil
        ))
        updates.apply(restarting)
        updates.apply(restarting)
        XCTAssertEqual(quits, 1)
        XCTAssertEqual(updates.view.status, "Restarting Iris Chat…")
    }

    func testStripStaysHiddenWithoutAnUpdate() throws {
        let updates = preview(.upToDate(updatedTo: nil))
        let host = NSHostingView(rootView: IrisTheme { DesktopUpdateStripe(updates: updates).frame(width: 720) })
        host.layoutSubtreeIfNeeded()
        XCTAssertEqual(host.fittingSize.height, 0, accuracy: 1)
    }

    // MARK: - Helpers

    private func controller(quit: @escaping () -> Void = {}) -> DesktopUpdateController {
        DesktopUpdateController(
            defaults: UserDefaults(suiteName: suite)!,
            stateDir: stateDir,
            appPath: nil,
            quit: quit
        )
    }

    private func preview(_ stage: DesktopUpdateStage) -> DesktopUpdateController {
        let updates = controller()
        updates.apply(desktopUpdateView(snapshot: DesktopUpdateSnapshot(
            stage: stage,
            lastCheckedSecs: 1_794_900_000,
            diagnostic: nil
        )))
        return updates
    }

    private func waitFor(
        _ updates: DesktopUpdateController,
        _ condition: (DesktopUpdateView) -> Bool
    ) async throws {
        for _ in 0..<200 where !condition(updates.view) {
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        XCTAssertTrue(condition(updates.view), "still showing \(updates.view.status)")
    }

    private func captureStrip(_ updates: DesktopUpdateController, name: String) throws {
        let host = NSHostingView(rootView: IrisTheme { DesktopUpdateStripe(updates: updates).frame(width: 720) })
        host.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(host.fittingSize.height, 20, name)
        host.setFrameSize(host.fittingSize)
        try capture(host, name: name)
    }

    private func captureSettings(_ updates: DesktopUpdateController, name: String) throws {
        let host = NSHostingView(rootView: IrisTheme {
            DesktopUpdateSettingsSection(buildSummary: "2026.11.13.6", updates: updates)
                .padding(24).frame(width: 620)
                .background(Color(nsColor: .windowBackgroundColor))
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
