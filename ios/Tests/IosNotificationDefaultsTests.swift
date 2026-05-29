import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

#if os(iOS)
final class IosNotificationDefaultsTests: XCTestCase {
    @MainActor
    func testFreshInstallNotificationDefaultsDisableBeforeOnboarding() async {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState())
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: ["IRIS_IOS_ENABLE_NOTIFICATION_DEFAULTS_IN_TESTS": "1"]
        )

        let disabledOnEmptyLaunch = await waitUntil {
            rust.dispatchedActions.contains(.setDesktopNotificationsEnabled(enabled: false)) &&
            rust.dispatchedActions.contains(.setInviteAcceptanceNotificationsEnabled(enabled: false))
        }
        XCTAssertTrue(disabledOnEmptyLaunch)

        rust.clearDispatchedActions()
        manager.createAccount(name: " Alice ")
        XCTAssertEqual(rust.dispatchedActions, [
            .setDesktopNotificationsEnabled(enabled: false),
            .setInviteAcceptanceNotificationsEnabled(enabled: false),
            .createAccount(name: "Alice"),
        ])

        rust.clearDispatchedActions()
        manager.restoreSession(ownerNsec: " nsec1restored ")
        XCTAssertEqual(rust.dispatchedActions, [
            .setDesktopNotificationsEnabled(enabled: false),
            .setInviteAcceptanceNotificationsEnabled(enabled: false),
            .restoreSession(ownerNsec: "nsec1restored"),
        ])

        rust.clearDispatchedActions()
        manager.startLinkedDevice(ownerInput: " user-id ")
        XCTAssertEqual(rust.dispatchedActions, [
            .setDesktopNotificationsEnabled(enabled: false),
            .setInviteAcceptanceNotificationsEnabled(enabled: false),
            .startLinkedDevice(ownerInput: "user-id"),
        ])

        _ = manager
    }
}
#endif
