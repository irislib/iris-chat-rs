import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

#if os(iOS)
final class IosNotificationDefaultsTests: XCTestCase {
    @MainActor
    func testFreshInstallKeepsNotificationDefaultsEnabled() async {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState())
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir
        )

        try? await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertFalse(rust.dispatchedActions.contains(.setDesktopNotificationsEnabled(enabled: false)))
        XCTAssertFalse(rust.dispatchedActions.contains(.setInviteAcceptanceNotificationsEnabled(enabled: false)))

        rust.clearDispatchedActions()
        manager.createAccount(name: " Alice ")
        XCTAssertEqual(rust.dispatchedActions, [.createAccount(name: "Alice")])

        rust.clearDispatchedActions()
        manager.restoreSession(ownerNsec: " nsec1restored ")
        XCTAssertEqual(rust.dispatchedActions, [.restoreSession(ownerNsec: "nsec1restored")])

        rust.clearDispatchedActions()
        manager.startLinkedDevice(ownerInput: " user-id ")
        XCTAssertEqual(rust.dispatchedActions, [.startLinkedDevice(ownerInput: "user-id")])

        _ = manager
    }
}
#endif
