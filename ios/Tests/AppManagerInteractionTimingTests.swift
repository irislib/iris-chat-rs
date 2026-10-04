import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

@MainActor
final class AppManagerInteractionTimingTests: XCTestCase {
    private let chatID = "chat-1"
    private var directory: URL!

    override func setUp() {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    }

    override func tearDown() { try? FileManager.default.removeItem(at: directory) }

    private func snapshot(enabled: Bool, rev: UInt64 = 1) -> AppState {
        var state = makeLargeFixtureState(
            rev: rev, router: Router(defaultScreen: .chatList, screenStack: [.chat(chatId: chatID)]),
            account: makeAccount(), currentChat: makeCurrentChat(chatId: chatID)
        )
        state.preferences.debugLoggingEnabled = enabled
        return state
    }

    private func manager(_ rust: MockRustApp, perfLaunch: Bool = false) -> AppManager {
        var environment = ["IRIS_UI_TEST_RUN_ID": "interaction-unit", "IRIS_DISABLE_NOTIFICATIONS": "1"]
        if perfLaunch { environment["IRIS_PERF_LOG"] = "1" }
        return AppManager(rust: rust, secretStore: InMemorySecretStore(),
                          pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
                          dataDir: directory, environment: environment)
    }

    private func apply(enabled: Bool, rev: UInt64, via rust: MockRustApp, to app: AppManager) async {
        let state = snapshot(enabled: enabled, rev: rev)
        rust.currentState = state
        rust.emit(.fullState(state))
        let applied = await waitUntil { app.state.rev == rev }
        XCTAssertTrue(applied)
    }

    func testDisabledPreferenceDoesNotCreateTimingForOpenOrSend() {
        let rust = MockRustApp(state: snapshot(enabled: false))
        let app = manager(rust)
        app.dispatch(.openChat(chatId: chatID))
        app.dispatch(.sendMessage(chatId: chatID, text: "untimed message"))
        app.dispatch(.sendDisappearingMessage(chatId: chatID, text: "untimed disappearing message", expiresAtSecs: 100))
        XCTAssertNil(app.interactionTiming, "Disabled actions must not retain timing message copies")
        XCTAssertTrue(rust.dispatchedActions.contains(.sendMessage(chatId: chatID, text: "untimed message")))
    }

    func testPersistedPreferenceEnablesTimingAndEnabledUpdatesReuseIt() async throws {
        let rust = MockRustApp(state: snapshot(enabled: true))
        let app = manager(rust)
        let timing = try XCTUnwrap(app.interactionTiming)
        app.dispatch(.sendMessage(chatId: chatID, text: "pending measurement"))
        await apply(enabled: true, rev: 2, via: rust, to: app)
        XCTAssertTrue(app.interactionTiming === timing, "Reconciliation must not restart an in-flight measurement")
    }

    func testAuthoritativeToggleEnablesTimingAndOffReleasesPendingSend() async {
        let rust = MockRustApp(state: snapshot(enabled: false))
        let app = manager(rust)
        app.dispatch(.setDebugLoggingEnabled(enabled: true))
        XCTAssertEqual(rust.dispatchedActions.last, .setDebugLoggingEnabled(enabled: true))
        XCTAssertNil(app.interactionTiming, "Wait for the authoritative preference")
        await apply(enabled: true, rev: 2, via: rust, to: app)
        XCTAssertNotNil(app.interactionTiming)
        weak var previous = app.interactionTiming
        app.dispatch(.sendMessage(chatId: chatID, text: "pending measurement"))
        app.dispatch(.setDebugLoggingEnabled(enabled: false))
        await apply(enabled: false, rev: 3, via: rust, to: app)
        XCTAssertNil(app.interactionTiming)
        XCTAssertNil(previous, "Turning off must release the helper and its pending message IDs and body")
        app.dispatch(.sendMessage(chatId: chatID, text: "untimed message"))
        XCTAssertNil(app.interactionTiming)
        await apply(enabled: true, rev: 4, via: rust, to: app)
        XCTAssertNotNil(app.interactionTiming)
    }

    func testExplicitPerformanceLaunchStaysEnabledWhenPreferenceTurnsOff() async throws {
        let rust = MockRustApp(state: snapshot(enabled: false))
        let app = manager(rust, perfLaunch: true)
        let timing = try XCTUnwrap(app.interactionTiming)
        app.dispatch(.sendMessage(chatId: chatID, text: "explicit measurement"))
        await apply(enabled: true, rev: 2, via: rust, to: app)
        await apply(enabled: false, rev: 3, via: rust, to: app)
        XCTAssertTrue(app.interactionTiming === timing, "An explicit launch opt-in takes precedence over the preference")
    }
}
