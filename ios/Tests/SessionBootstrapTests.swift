import XCTest
import Combine
#if os(macOS)
import AppKit
import SwiftUI
import Vision
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

@MainActor
final class SessionBootstrapTests: XCTestCase {
    private var directory: URL!

    override func setUp() {
        super.setUp()
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
        super.tearDown()
    }

    func testStartupSnapshotBeforeCredentialLookupKeepsWelcomeHidden() async {
        let rust = MockRustApp()
        let manager = makeManager(rust: rust)
        var snapshot = rust.state()
        snapshot.rev = 1

        // A device-label or network update can arrive before the restore task runs.
        manager.apply(update: .fullState(snapshot), generation: 0)
        XCTAssertTrue(manager.bootstrapInFlight)

        await Task.yield()
        XCTAssertTrue(manager.bootstrapInFlight)
    }

    func testQueuedLoggedOutSnapshotKeepsWelcomeHiddenUntilAccountArrives() async throws {
        let rust = MockRustApp()
        let manager = makeManager(rust: rust)
        await Task.yield()
        XCTAssertTrue(rust.dispatchedActions.contains {
            if case .restoreAccountBundle = $0 { return true }
            return false
        })

        var snapshot = rust.state()
        snapshot.rev = 1
        rust.emit(.fullState(snapshot))
        let applied = await waitUntil { manager.state.rev == 1 }
        XCTAssertTrue(applied)
        XCTAssertTrue(manager.bootstrapInFlight)
#if os(macOS)
        try capture(manager, name: "session-loading")
#endif

        snapshot.rev = 2
        snapshot.account = makeAccount()
        snapshot.router = Router(defaultScreen: .chatList, screenStack: [])
        rust.emit(.fullState(snapshot))
        let restored = await waitUntil { manager.state.account != nil }
        XCTAssertTrue(restored)
        XCTAssertFalse(manager.bootstrapInFlight)
#if os(macOS)
        try capture(manager, name: "session-restored")
#endif
    }

    func testRestoreErrorShowsWelcomeEvenWhenBusySnapshotWasCoalesced() async throws {
        let rust = MockRustApp()
        let manager = makeManager(rust: rust)
        await Task.yield()
        var snapshot = rust.state()
        snapshot.rev = 1
        snapshot.toast = "Invalid key."
        rust.emit(.fullState(snapshot))
        let applied = await waitUntil { manager.state.rev == 1 }
        XCTAssertTrue(applied)
        XCTAssertFalse(manager.bootstrapInFlight)
        XCTAssertEqual(manager.activeScreen, .welcome)
#if os(macOS)
        try capture(manager, name: "session-error-sign-in")
#endif
    }

    func testRestoreDispatchFailureShowsWelcome() async {
        let rust = MockRustApp()
        rust.dispatchError = NSError(domain: "SessionBootstrapTests", code: 1)
        let manager = makeManager(rust: rust)
        let settled = await waitUntil { !manager.bootstrapInFlight }
        XCTAssertTrue(settled)
        XCTAssertEqual(manager.activeScreen, .welcome)
    }

    func testFreshInstallShowsWelcomeAfterCredentialLookup() async {
        let manager = makeManager(rust: MockRustApp(), store: InMemorySecretStore())
        let settled = await waitUntil { !manager.bootstrapInFlight }
        XCTAssertTrue(settled)
        XCTAssertEqual(manager.activeScreen, .welcome)
    }

    func testStartupMarkersSeparateQueuedRestoreFromObservedNativeRestoration() async {
        let rust = MockRustApp()
        var records: [IrisStartupLifecycle.Record] = []
        let lifecycle = IrisStartupLifecycle { records.append($0) }
        let manager = makeManager(rust: rust, lifecycle: lifecycle)

        // Bootstrap starts true before secure inputs have even been read. That
        // default must never be reported as a successfully enqueued restoration.
        _ = manager.appBackgrounded()
        XCTAssertEqual(records, [.init(phase: .backgrounded, restorePendingResult: false,
                                     nativeRestoringObserved: false, appBackgrounded: true)])
        let enqueued = await waitUntil { records.contains { $0.phase == .restoreEnqueued } }
        XCTAssertTrue(enqueued)
        XCTAssertEqual(records.last, .init(phase: .restoreEnqueued, restorePendingResult: true,
                                          nativeRestoringObserved: false, appBackgrounded: true))

        var snapshot = rust.state()
        snapshot.rev = 1
        manager.apply(update: .fullState(snapshot), generation: 0)
        manager.appForegrounded()
        _ = manager.appBackgrounded()
        XCTAssertEqual(records.last, .init(phase: .backgrounded, restorePendingResult: true,
                                          nativeRestoringObserved: false, appBackgrounded: true),
                       "a queued idle snapshot cannot establish restoration overlap")
        XCTAssertFalse(records.contains { $0.phase == .restoreResultObserved })

        snapshot.rev = 2
        snapshot.busy.restoringSession = true
        manager.apply(update: .fullState(snapshot), generation: 0)
        manager.appForegrounded()
        _ = manager.appBackgrounded()
        XCTAssertEqual(records.last, .init(phase: .backgrounded, restorePendingResult: true,
                                          nativeRestoringObserved: true, appBackgrounded: true))
        let countBeforeRepeatedBackground = records.count
        _ = manager.appBackgrounded()
        XCTAssertEqual(records.count, countBeforeRepeatedBackground,
                       "a repeated suspend request is not a new background transition")

        snapshot.rev = 3
        snapshot.busy.restoringSession = false
        snapshot.account = makeAccount()
        snapshot.router = Router(defaultScreen: .chatList, screenStack: [])
        manager.apply(update: .fullState(snapshot), generation: 0)
        XCTAssertEqual(records.last, .init(phase: .restoreResultObserved, restorePendingResult: false,
                                          nativeRestoringObserved: false, appBackgrounded: true))
        snapshot.rev = 4
        manager.apply(update: .fullState(snapshot), generation: 0)
        manager.appForegrounded()
        _ = manager.appBackgrounded()
        XCTAssertEqual(records.last?.restorePendingResult, false)
        XCTAssertEqual(records.filter { $0.phase == .restoreResultObserved }.count, 1,
                       "ordinary snapshots must not repeatedly emit startup completion")
        XCTAssertEqual(records.map(\.phase), [.backgrounded, .restoreEnqueued, .backgrounded,
                                             .backgrounded, .restoreResultObserved, .backgrounded])
    }

    func testStartupMarkersDoNotClaimEnqueueForMissingInputsOrDispatchFailure() async {
        for failDispatch in [false, true] {
            let rust = MockRustApp()
            var records: [IrisStartupLifecycle.Record] = []
            let lifecycle = IrisStartupLifecycle { records.append($0) }
            if failDispatch {
                rust.dispatchError = NSError(domain: "SessionBootstrapTests", code: 2)
            }
            let manager = makeManager(rust: rust,
                                      store: failDispatch ? nil : InMemorySecretStore(),
                                      lifecycle: lifecycle)
            let settled = await waitUntil { !manager.bootstrapInFlight }
            XCTAssertTrue(settled)
            _ = manager.appBackgrounded()
            XCTAssertFalse(records.contains { $0.phase == .restoreEnqueued })
            XCTAssertFalse(records.contains { $0.phase == .restoreResultObserved })
            XCTAssertEqual(records.filter { $0.phase == .restoreDispatchFailed }.count,
                           failDispatch ? 1 : 0)
            XCTAssertEqual(records.last?.restorePendingResult, false)
        }
    }

    func testStartupMarkersRecordTerminalErrorWithoutInventingNativeBusyObservation() async {
        let rust = MockRustApp()
        var records: [IrisStartupLifecycle.Record] = []
        let manager = makeManager(rust: rust, lifecycle: IrisStartupLifecycle { records.append($0) })
        let enqueued = await waitUntil { records.contains { $0.phase == .restoreEnqueued } }
        XCTAssertTrue(enqueued)
        var snapshot = rust.state()
        snapshot.rev = 1
        snapshot.toast = "Invalid key."
        manager.apply(update: .fullState(snapshot), generation: 0)
        _ = manager.appBackgrounded()
        XCTAssertEqual(records.map(\.phase), [.restoreEnqueued, .restoreResultObserved, .backgrounded])
        XCTAssertFalse(records.contains { $0.nativeRestoringObserved })
        XCTAssertEqual(records.last?.restorePendingResult, false)
    }

    func testStartupMarkersCancelPendingResultOnLocalReset() async {
        let rust = MockRustApp()
        var records: [IrisStartupLifecycle.Record] = []
        let manager = makeManager(rust: rust, lifecycle: IrisStartupLifecycle { records.append($0) })
        let enqueued = await waitUntil { records.contains { $0.phase == .restoreEnqueued } }
        XCTAssertTrue(enqueued)
        manager.logout()
        _ = manager.appBackgrounded()
        XCTAssertEqual(records.map(\.phase), [.restoreEnqueued, .restoreCancelled, .backgrounded])
        XCTAssertEqual(records.last?.restorePendingResult, false)
        let settled = await waitUntil { !manager.bootstrapInFlight }
        XCTAssertTrue(settled)
        XCTAssertFalse(records.contains { $0.phase == .restoreResultObserved },
                       "a replacement core's idle snapshot cannot finish the abandoned restore")
    }

    func testStartupAllowanceBeginsBeforeNativeFactoryAndForegroundRestoreDoesNotSuspend() async {
        let rust = MockRustApp()
        var allowanceStarted = false
        var allowanceFinished = false
        var finishCount = 0
        var activity: IrisStartupBackgroundActivity?
        let manager = AppManager(
            secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory, environment: [:],
            rustFactory: {
                XCTAssertTrue(allowanceStarted, "native construction must already be protected")
                XCTAssertFalse(allowanceFinished)
                return rust
            },
            startupActivityFactory: {
                allowanceStarted = true
                let started = IrisStartupBackgroundActivity(initiallyBackgrounded: false) {
                    allowanceFinished = true
                    finishCount += 1
                }
                activity = started
                return started
            }
        )
        XCTAssertTrue(allowanceStarted)
        XCTAssertFalse(allowanceFinished, "deferred secure-input lookup still belongs to startup")
        let completed = await waitUntil { allowanceFinished }
        XCTAssertTrue(completed)
        XCTAssertTrue(manager.appSceneIsActive)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 0)
        activity?.finish()
        XCTAssertEqual(finishCount, 1)
    }

#if os(iOS)
    func testSlowBackgroundStartupKeepsLoadingAndStorageProtectionUntilAcknowledged() async {
        let rust = MockRustApp()
        let started = expectation(description: "startup barrier waiting for native restore")
        let finished = expectation(description: "startup allowance ends after native acknowledgement")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        var allowanceFinished = false
        rust.onNextPrepareForSuspend {
            started.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 12), .success)
        }
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(bundle: StoredAccountBundle(
                ownerNsec: "test-owner", ownerPubkeyHex: "test-owner-id", deviceNsec: "test-device"
            )),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory, environment: [:],
            startupActivityFactory: {
                IrisStartupBackgroundActivity(initiallyBackgrounded: true) {
                    allowanceFinished = true
                    finished.fulfill()
                }
            }
        )
        await fulfillment(of: [started], timeout: 2)
        let exposedLogin = await waitUntil(timeoutNanoseconds: 9_000_000_000) { !manager.bootstrapInFlight }
        XCTAssertFalse(exposedLogin, "slow restoration must not expose login while native work is pending")
        XCTAssertFalse(allowanceFinished, "a slow restore must retain protection until native storage acknowledges suspension")
        gate.signal()
        await fulfillment(of: [finished], timeout: 2)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
    }
#endif

    func testLiveCoreRestoreErrorShowsWelcome() async {
        let rust = LiveRustAppClient(dataDir: directory.path, appVersion: "test")
        let manager = makeManager(rust: rust)
        let settled = await waitUntil(timeoutNanoseconds: 5_000_000_000) {
            !manager.bootstrapInFlight && manager.state.toast != nil
        }
        XCTAssertTrue(settled)
        XCTAssertNil(manager.state.account)
        XCTAssertEqual(manager.activeScreen, .welcome)
        await rust.shutdown()
    }

    func testSlowRestoreGoesDirectlyFromLoadingToChatsWithoutShowingWelcome() async throws {
        let rust = MockRustApp()
        let manager = makeManager(rust: rust)
        var exposedLoginDuringTransition = false
        let observation = manager.$bootstrapInFlight.sink { loading in
            if !loading && manager.state.account == nil {
                exposedLoginDuringTransition = true
            }
        }
        defer { observation.cancel() }
#if os(macOS)
        let (window, host) = makeWindow(manager)
        defer { window.orderOut(nil) }
#endif
        await Task.yield()
        var snapshot = rust.state()
        snapshot.rev = 1
        snapshot.busy.restoringSession = true
        rust.emit(.fullState(snapshot))
        let started = await waitUntil { manager.state.rev == 1 }
        XCTAssertTrue(started)
        XCTAssertTrue(manager.bootstrapInFlight)

        let exposedLogin = await waitUntil(timeoutNanoseconds: 9_000_000_000) { !manager.bootstrapInFlight }
        XCTAssertFalse(exposedLogin, "elapsed time does not establish that a saved account is logged out")
        XCTAssertTrue(manager.bootstrapInFlight)
#if os(macOS)
        XCTAssertTrue(containsText("Loading…", in: host))
        XCTAssertFalse(containsText("Restore profile", in: host))
        try capture(host, name: "session-slow-restore-no-login")
#endif

        snapshot.rev = 2
        rust.emit(.fullState(snapshot))
        let updated = await waitUntil { manager.state.rev == 2 }
        XCTAssertTrue(updated)
        XCTAssertTrue(manager.bootstrapInFlight, "intermediate restore snapshots must keep login hidden")

        snapshot.rev = 3
        snapshot.busy.restoringSession = false
        snapshot.account = makeAccount()
        snapshot.router = Router(defaultScreen: .chatList, screenStack: [])
        rust.emit(.fullState(snapshot))
        let restored = await waitUntil { manager.state.account != nil }
        XCTAssertTrue(restored)
        XCTAssertFalse(manager.bootstrapInFlight)
        XCTAssertEqual(manager.activeScreen, .chatList)
        XCTAssertFalse(exposedLoginDuringTransition, "loading must transition directly to the restored account")
#if os(macOS)
        XCTAssertFalse(containsText("Restore profile", in: host))
        try capture(host, name: "session-slow-restore-chats")
#endif
    }

    func testPendingDeviceLinkRestoreWaitsForLinkScreen() async {
        let rust = MockRustApp()
        var records: [IrisStartupLifecycle.Record] = []
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(link: StoredPendingDeviceLink(
                deviceNsec: "test-device", approvalBootstrapJson: "{}"
            )),
            dataDir: directory,
            environment: ["IRIS_UI_TEST_RUN_ID": "session-bootstrap", "IRIS_DISABLE_NOTIFICATIONS": "1"],
            startupLifecycle: IrisStartupLifecycle { records.append($0) }
        )
        await Task.yield()
        var snapshot = rust.state()
        snapshot.rev = 1
        rust.emit(.fullState(snapshot))
        let applied = await waitUntil { manager.state.rev == 1 }
        XCTAssertTrue(applied)
        XCTAssertTrue(manager.bootstrapInFlight)
        XCTAssertEqual(records.map(\.phase), [.restoreEnqueued])

        snapshot.rev = 2
        snapshot.linkDevice = LinkDeviceSnapshot(url: "test-link", deviceInput: "test-device")
        snapshot.router = Router(defaultScreen: .welcome, screenStack: [.addDevice])
        rust.emit(.fullState(snapshot))
        let linked = await waitUntil { manager.state.rev == 2 }
        XCTAssertTrue(linked)
        XCTAssertFalse(manager.bootstrapInFlight)
        XCTAssertEqual(manager.activeScreen, .addDevice)
        XCTAssertEqual(records.map(\.phase), [.restoreEnqueued, .restoreResultObserved])
    }

    private func makeManager(rust: RustAppClient, store: InMemorySecretStore? = nil,
                             lifecycle: IrisStartupLifecycle? = nil) -> AppManager {
        AppManager(
            rust: rust,
            secretStore: store ?? InMemorySecretStore(bundle: StoredAccountBundle(
                ownerNsec: "test-owner", ownerPubkeyHex: "test-owner-id", deviceNsec: "test-device"
            )),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory,
            environment: ["IRIS_UI_TEST_RUN_ID": "session-bootstrap", "IRIS_DISABLE_NOTIFICATIONS": "1"],
            startupLifecycle: lifecycle
        )
    }

#if os(macOS)
    func testSlowRestoreShowsLoadingOnlyAfterTwoSeconds() async throws {
        let manager = makeManager(rust: MockRustApp())
        let started = ContinuousClock.now
        let (window, host) = makeWindow(manager)
        defer { window.orderOut(nil) }
        XCTAssertFalse(containsText("Loading…", in: host))
        try capture(host, name: "session-initial-blank")

        let appeared = await waitUntil(timeoutNanoseconds: 3_000_000_000) {
            self.containsText("Loading…", in: host)
        }
        XCTAssertTrue(appeared)
        XCTAssertGreaterThanOrEqual(started.duration(to: .now), .seconds(2))
        try capture(host, name: "session-delayed-loading")
    }

    func testErrorAppearsDuringInitialBlankStartup() async throws {
        let rust = MockRustApp()
        let manager = makeManager(rust: rust)
        let (window, host) = makeWindow(manager)
        defer { window.orderOut(nil) }
        var snapshot = rust.state()
        snapshot.rev = 1
        snapshot.busy.restoringSession = true
        snapshot.toast = "Could not restore profile."
        manager.apply(update: .fullState(snapshot), generation: 0)

        XCTAssertEqual(manager.toasts.message, "Could not restore profile.")
        XCTAssertTrue(manager.bootstrapInFlight)
        await Task.yield()
        XCTAssertFalse(containsText("Loading…", in: host))
        try capture(host, name: "session-immediate-error")
    }

    func testCompletedRestoreDoesNotShowDelayedLoading() async throws {
        let rust = MockRustApp()
        let manager = makeManager(rust: rust)
        let (window, host) = makeWindow(manager)
        defer { window.orderOut(nil) }
        await Task.yield()
        var snapshot = rust.state()
        snapshot.rev = 1
        snapshot.account = makeAccount()
        snapshot.router = Router(defaultScreen: .chatList, screenStack: [])
        manager.apply(update: .fullState(snapshot), generation: 0)
        XCTAssertFalse(manager.bootstrapInFlight)

        // Give the removed loading view's task time to finish if it was not cancelled.
        let appeared = await waitUntil(timeoutNanoseconds: 2_500_000_000) {
            self.containsText("Loading…", in: host)
        }
        XCTAssertFalse(appeared)
        XCTAssertEqual(manager.activeScreen, .chatList)
    }

    private func containsText(_ text: String, in host: NSView) -> Bool {
        host.layoutSubtreeIfNeeded()
        host.displayIfNeeded()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return false }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        guard let image = bitmap.cgImage else { return false }
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        try? VNImageRequestHandler(cgImage: image).perform([request])
        let expected = text.trimmingCharacters(in: .punctuationCharacters)
        return request.results?.contains {
            $0.topCandidates(1).first?.string.contains(expected) == true
        } == true
    }

    private func makeWindow(_ manager: AppManager) -> (NSWindow, NSHostingView<RootView>) {
        let host = NSHostingView(rootView: RootView(manager: manager))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 980, height: 640),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        host.layoutSubtreeIfNeeded()
        host.displayIfNeeded()
        return (window, host)
    }

    private func capture(_ manager: AppManager, name: String) throws {
        let (window, host) = makeWindow(manager)
        defer { window.orderOut(nil) }
        try capture(host, name: name)
    }

    private func capture(_ host: NSView, name: String) throws {
        host.layoutSubtreeIfNeeded()
        host.displayIfNeeded()
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
#endif
}
