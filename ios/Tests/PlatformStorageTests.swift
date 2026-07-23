import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
import Security
@testable import IrisChat
#endif

final class PlatformStorageTests: XCTestCase {
    func testKeychainSecretStoreRoundTrip() throws {
#if os(macOS)
        throw XCTSkip("macOS test lane uses the file-backed test store to avoid Keychain permission UI")
#else
        let service = "fi.siriusbusiness.irischat.tests.\(UUID().uuidString)"
        let account = "stored-account-bundle"
        let probeQuery: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: "\(account)-probe",
            kSecValueData: Data()
        ]
        let probeStatus = SecItemAdd(probeQuery as CFDictionary, nil)
        if probeStatus == errSecMissingEntitlement {
            throw XCTSkip("unsigned simulator test bundle cannot access Keychain")
        }
        XCTAssertEqual(probeStatus, errSecSuccess)
        SecItemDelete(probeQuery as CFDictionary)

        let expected = StoredAccountBundle(
            ownerNsec: "nsec1owner",
            ownerPubkeyHex: "owner-hex",
            deviceNsec: "nsec1device"
        )

        let legacyStore = KeychainSecretStore(service: service, account: account, accessibility: nil)
        legacyStore.clear()
        legacyStore.save(expected)
        XCTAssertEqual(legacyStore.load(), expected)

        let store = KeychainSecretStore(service: service, account: account)
        XCTAssertEqual(store.load(), expected)
        store.save(expected)
        XCTAssertEqual(store.load(), expected)

        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
            kSecReturnAttributes: true,
            kSecMatchLimit: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        XCTAssertEqual(SecItemCopyMatching(query as CFDictionary, &item), errSecSuccess)
        let attributes = item as? [String: Any]
        XCTAssertEqual(
            attributes?[kSecAttrAccessible as String] as? String,
            kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String
        )

        XCTAssertTrue(store.clear())
        XCTAssertNil(store.load())
#endif
    }

    func testNotificationDataDirUsesBackgroundReadableProtection() throws {
#if os(macOS)
        throw XCTSkip("macOS has no iOS Notification Service Extension data protection")
#else
        let fileManager = FileManager.default
        let tempDir = fileManager.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        let nestedDir = tempDir.appendingPathComponent("core", isDirectory: true)
        let nestedFile = nestedDir.appendingPathComponent("state.json")
        defer { try? fileManager.removeItem(at: tempDir) }

        try fileManager.createDirectory(at: nestedDir, withIntermediateDirectories: true)
        try Data("{}".utf8).write(to: nestedFile)

        AppPaths.prepareDataDirForBackgroundNotificationReads(tempDir, fileManager: fileManager)

        let keys: Set<URLResourceKey> = [.fileProtectionKey]
        let dirProtection = try tempDir.resourceValues(forKeys: keys).fileProtection
        guard dirProtection != nil else {
            throw XCTSkip("simulator filesystem does not report iOS file-protection attributes")
        }
        XCTAssertEqual(dirProtection, .completeUntilFirstUserAuthentication)
        XCTAssertEqual(
            try nestedFile.resourceValues(forKeys: keys).fileProtection,
            .completeUntilFirstUserAuthentication
        )
#endif
    }

    func testNotificationAutomationRequiresExplicitOverride() {
        XCTAssertTrue(AppPaths.notificationsDisabledForAutomation(environment: [
            "IRIS_UI_TEST_RUN_ID": "ordinary-test",
        ]))
        XCTAssertFalse(AppPaths.notificationsDisabledForAutomation(environment: [
            "IRIS_UI_TEST_RUN_ID": "push-e2e",
            "IRIS_ENABLE_NOTIFICATIONS_FOR_AUTOMATION": "1",
        ]))
        XCTAssertTrue(AppPaths.notificationsDisabledForAutomation(environment: [
            "IRIS_UI_TEST_RUN_ID": "push-e2e",
            "IRIS_ENABLE_NOTIFICATIONS_FOR_AUTOMATION": "1",
            "IRIS_DISABLE_NOTIFICATIONS": "1",
        ]))
    }

    func testFileAccountSecretStoreRoundTrip() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let store = FileAccountSecretStore(
            url: tempDir.appendingPathComponent("account-secret.json"),
            fileManager: .default
        )
        let expected = StoredAccountBundle(
            ownerNsec: "nsec1owner",
            ownerPubkeyHex: "owner-hex",
            deviceNsec: "nsec1device"
        )

        store.save(expected)
        XCTAssertEqual(store.load(), expected)
        XCTAssertTrue(store.clear())
        XCTAssertNil(store.load())
    }

    func testFilePendingDeviceLinkSecretStoreRoundTrip() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let store = FileAccountSecretStore(
            url: tempDir.appendingPathComponent("pending-device-link-secret.json"),
            fileManager: .default
        )
        let expected = StoredPendingDeviceLink(
            deviceNsec: "nsec1device",
            approvalBootstrapJson: "{\"v\":1}"
        )

        store.savePendingDeviceLink(expected)
        XCTAssertEqual(store.loadPendingDeviceLink(), expected)
        XCTAssertTrue(store.clear())
        XCTAssertNil(store.loadPendingDeviceLink())
    }

#if os(macOS)
    func testMacUiTestSecretStoreUsesDataDirectoryFile() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let secretFile = tempDir.appendingPathComponent("account-secret.json")
        let store = AppPaths.secretStore(
            dataDir: tempDir,
            fileManager: .default,
            environment: ["IRIS_UI_TEST_RUN_ID": UUID().uuidString]
        )
        let expected = StoredAccountBundle(
            ownerNsec: "nsec1owner",
            ownerPubkeyHex: "owner-hex",
            deviceNsec: "nsec1device"
        )

        store.save(expected)
        XCTAssertEqual(store.load(), expected)
        XCTAssertTrue(FileManager.default.fileExists(atPath: secretFile.path))
    }
#endif
}
