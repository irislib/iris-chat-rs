import XCTest

/// Opt-in physical-device gate for the FIPS BLE transport.
///
/// The peer must be an Iris account whose phone has IP networking disabled
/// but Bluetooth enabled. Normal test runs skip this method because they do
/// not provide `IRIS_FIPS_PHYSICAL_PEER_NPUB`.
final class FipsBlePhysicalUITests: XCTestCase {
    func testSendAndReceiveReceiptOverFipsBle() throws {
#if os(macOS)
        throw XCTSkip("FIPS BLE physical gate is iOS-only")
#else
        let environment = ProcessInfo.processInfo.environment
        guard let peerNpub = environment["IRIS_FIPS_PHYSICAL_PEER_NPUB"],
              peerNpub.hasPrefix("npub") else {
            throw XCTSkip("Set IRIS_FIPS_PHYSICAL_PEER_NPUB for the physical BLE gate")
        }
        let runID = environment["IRIS_FIPS_PHYSICAL_RUN_ID"] ?? "fips-ble-physical"
        let message = environment["IRIS_FIPS_PHYSICAL_MESSAGE"] ?? "fips-ble-physical-receipt"
        let preSendDelay = TimeInterval(environment["IRIS_FIPS_PRE_SEND_DELAY"] ?? "10") ?? 10
        let receiptTimeout = TimeInterval(environment["IRIS_FIPS_RECEIPT_TIMEOUT"] ?? "90") ?? 90

        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = runID
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launchEnvironment["IRIS_FIPS_PHYSICAL_PEER_DEVICE_HEX"] =
            environment["IRIS_FIPS_PHYSICAL_PEER_DEVICE_HEX"]
        app.launchEnvironment["IRIS_FIPS_BLE_TRACE"] = environment["IRIS_FIPS_BLE_TRACE"]
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        dismissBlockingSystemAlertIfPresent()

        createTestAccount(in: app)
        enableFipsBluetooth(in: app)
        openChat(with: peerNpub, in: app)

        guard waitForPeerProtocolReady(in: app) else {
            return
        }

        print("IRIS_FIPS_READY_TO_SEND")
        Thread.sleep(forTimeInterval: preSendDelay)
        guard sendAndWaitForReceipt(
            message,
            in: app,
            receiptTimeout: receiptTimeout,
            verifyTransportTrace: true
        ) else { return }
        if environment["IRIS_FIPS_IDLE_METRICS"] == "1" {
            try measureIdleAfterBluetoothReceipt(app, environment: environment)
        }
#endif
    }

    /// Xcode measures the app process, while this separate UI-test runner sleeps.
    /// The first interval is XCTest's discarded warmup; two intervals are retained.
    private func measureIdleAfterBluetoothReceipt(
        _ app: XCUIApplication,
        environment: [String: String]
    ) throws {
#if targetEnvironment(simulator)
        throw XCTSkip("Bluetooth idle metrics require a physical iPhone")
#else
        let seconds = try XCTUnwrap(Double(environment["IRIS_FIPS_IDLE_SECONDS"] ?? "60"))
        guard (10...120).contains(seconds) else {
            XCTFail("Idle interval must be between 10 and 120 seconds")
            return
        }
        let close = element(app, "messageInfoCloseButton")
        guard close.waitForExistence(timeout: 5) else {
            XCTFail("could not close the verified BLE probe before idle measurement")
            return
        }
        close.tap()
        let options = XCTMeasureOptions()
        options.iterationCount = 2
        options.invocationOptions = [.manuallyStart, .manuallyStop]
        measure(metrics: [XCTClockMetric(), XCTCPUMetric(application: app),
                          XCTStorageMetric(application: app)], options: options) {
            XCTAssertEqual(app.state, .runningForeground)
            XCTAssertTrue(ProcessInfo.processInfo.thermalState == .nominal ||
                          ProcessInfo.processInfo.thermalState == .fair,
                          "Phone is too hot for a comparable idle measurement")
            startMeasuring()
            Thread.sleep(forTimeInterval: seconds)
            stopMeasuring()
            XCTAssertEqual(app.state, .runningForeground)
            XCTAssertTrue(ProcessInfo.processInfo.thermalState == .nominal ||
                          ProcessInfo.processInfo.thermalState == .fair)
        }
#endif
    }

    func testReconnectAndReceiveSecondReceiptOverFipsBle() throws {
#if os(macOS)
        throw XCTSkip("FIPS BLE physical gate is iOS-only")
#else
        let environment = ProcessInfo.processInfo.environment
        guard let peerNpub = environment["IRIS_FIPS_PHYSICAL_PEER_NPUB"],
              peerNpub.hasPrefix("npub") else {
            throw XCTSkip("Set IRIS_FIPS_PHYSICAL_PEER_NPUB for the physical BLE gate")
        }
        let runID = environment["IRIS_FIPS_PHYSICAL_RUN_ID"] ?? "fips-ble-reconnect"
        let baseMessage = environment["IRIS_FIPS_PHYSICAL_MESSAGE"] ?? "fips-ble-reconnect"
        let receiptTimeout = TimeInterval(environment["IRIS_FIPS_RECEIPT_TIMEOUT"] ?? "120") ?? 120
        let firstMessage = "\(baseMessage)-before"
        let secondMessage = "\(baseMessage)-after"

        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = runID
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        dismissBlockingSystemAlertIfPresent()

        createTestAccount(in: app)
        enableFipsBluetooth(in: app)
        openChat(with: peerNpub, in: app)
        guard waitForPeerProtocolReady(in: app) else { return }
        guard sendAndWaitForReceipt(
            firstMessage,
            in: app,
            receiptTimeout: receiptTimeout,
            verifyTransportTrace: false
        ) else { return }

        app.terminate()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "0"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        XCTAssertTrue(
            element(app, "chatMessageInput").waitForExistence(timeout: 30),
            "chat did not restore after relaunch"
        )
        print("IRIS_FIPS_RECONNECTED_READY_TO_SEND")
        guard sendAndWaitForReceipt(
            secondMessage,
            in: app,
            receiptTimeout: receiptTimeout,
            verifyTransportTrace: true
        ) else { return }
#endif
    }

    func testReceiveBurstOverFipsBle() throws {
#if os(macOS)
        throw XCTSkip("FIPS BLE physical gate is iOS-only")
#else
        let environment = ProcessInfo.processInfo.environment
        guard let peerNpub = environment["IRIS_FIPS_PHYSICAL_PEER_NPUB"],
              peerNpub.hasPrefix("npub") else {
            throw XCTSkip("Set IRIS_FIPS_PHYSICAL_PEER_NPUB for the physical BLE gate")
        }
        let runID = environment["IRIS_FIPS_PHYSICAL_RUN_ID"] ?? "fips-ble-burst"
        let messagePrefix = environment["IRIS_FIPS_BURST_PREFIX"] ?? "fips-ble-burst"
        let messageCount = min(max(Int(environment["IRIS_FIPS_BURST_COUNT"] ?? "24") ?? 24, 1), 64)
        let messageSize = min(max(Int(environment["IRIS_FIPS_BURST_SIZE"] ?? "512") ?? 512, 32), 4_096)
        let receiveTimeout = TimeInterval(environment["IRIS_FIPS_BURST_TIMEOUT"] ?? "180") ?? 180

        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = runID
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_EXPOSE_ACCOUNT_NPUB"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        dismissBlockingSystemAlertIfPresent()

        createTestAccount(in: app)
        let profileButton = element(app, "chatListProfileButton")
        XCTAssertTrue(profileButton.waitForExistence(timeout: 10))
        guard let localNpub = profileButton.value as? String, localNpub.hasPrefix("npub") else {
            XCTFail("test account user ID was not exposed to the physical harness")
            return
        }
        enableFipsBluetooth(in: app)
        openChat(with: peerNpub, in: app)

        print("IRIS_FIPS_BURST_RECEIVER_NPUB=\(localNpub)")
        print("IRIS_FIPS_BURST_RECEIVER_ADVERTISING")
        guard waitForPeerProtocolReady(in: app) else { return }
        print("IRIS_FIPS_BURST_READY")
        let deadline = Date().addingTimeInterval(receiveTimeout)
        for index in 1...messageCount {
            let header = burstMessageHeader(prefix: messagePrefix, index: index)
            let message = burstMessage(
                prefix: messagePrefix,
                index: index,
                size: messageSize
            )
            let body = app.staticTexts
                .matching(NSPredicate(format: "label BEGINSWITH %@", header))
                .firstMatch
            guard body.waitForExistence(timeout: max(deadline.timeIntervalSinceNow, 1)) else {
                XCTFail("BLE burst message \(index) of \(messageCount) did not arrive")
                return
            }
            XCTAssertEqual(body.label, message, "BLE burst message \(index) payload changed")
        }
        // Keep the receiver and BLE link alive while the app's batched Seen
        // receipts leave the device. The Android gate requires all 24 rather
        // than accepting payload visibility alone.
        Thread.sleep(forTimeInterval: 5)
        print("IRIS_FIPS_BURST_RECEIVED=\(messageCount)")
#endif
    }

    private func dismissBlockingSystemAlertIfPresent() {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let ok = springboard.buttons["OK"]
        if ok.waitForExistence(timeout: 3) {
            ok.tap()
        }
    }

    private func createTestAccount(in app: XCUIApplication) {
        let create = element(app, "welcomeCreateAction")
        XCTAssertTrue(create.waitForExistence(timeout: 15))
        if !create.isEnabled {
            element(app, "onboardingTermsAgreementToggle").tap()
        }
        create.tap()
        XCTAssertTrue(element(app, "createAccountScreen").waitForExistence(timeout: 10))
        let name = element(app, "signupNameField")
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        name.tap()
        name.typeText("FIPS iPhone")
        let terms = element(app, "onboardingTermsAgreementToggle")
        let submit = element(app, "generateKeyButton")
        XCTAssertTrue(submit.waitForExistence(timeout: 10))
        if !submit.isEnabled, terms.exists {
            terms.tap()
        }
        XCTAssertTrue(submit.isEnabled)
        submit.tap()
        XCTAssertTrue(element(app, "chatListNewChatButton").waitForExistence(timeout: 30))
    }

    private func enableFipsBluetooth(in app: XCUIApplication) {
        let nearby = element(app, "nearbyChatRow")
        XCTAssertTrue(nearby.waitForExistence(timeout: 10))
        nearby.tap()
        XCTAssertTrue(element(app, "nearbyCloseButton").waitForExistence(timeout: 10))

        let master = element(app, "nearbyEnabledSwitch")
        if (master.value as? String) != "1" {
            master.tap()
        }
        let bluetooth = element(app, "nearbyBluetoothSwitch")
        XCTAssertTrue(bluetooth.waitForExistence(timeout: 10))
        if (bluetooth.value as? String) != "1" {
            bluetooth.tap()
        }

        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.buttons["Allow"]
        if allow.waitForExistence(timeout: 8) {
            allow.tap()
            XCTAssertTrue(app.wait(for: .runningForeground, timeout: 8))
        }
        element(app, "nearbyCloseButton").tap()
    }

    private func openChat(with peerNpub: String, in app: XCUIApplication) {
        let newChat = element(app, "chatListNewChatButton")
        XCTAssertTrue(newChat.waitForExistence(timeout: 10))
        newChat.tap()
        let peer = element(app, "newChatPeerInput")
        XCTAssertTrue(peer.waitForExistence(timeout: 10))
        peer.tap()
        peer.typeText(peerNpub)
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 30))
    }

    private func waitForPeerProtocolReady(in app: XCUIApplication) -> Bool {
        let title = element(app, "chatHeaderTitleButton")
        guard title.waitForExistence(timeout: 10) else {
            XCTFail("chat header did not become available")
            return false
        }
        title.tap()

        let advanced = element(app, "directChatAdvancedCard")
        guard advanced.waitForExistence(timeout: 10) else {
            XCTFail("peer protocol diagnostics did not become available")
            return false
        }
        for _ in 0..<4 where !advanced.isHittable {
            app.swipeUp()
        }
        guard advanced.isHittable else {
            XCTFail("peer protocol diagnostics could not be opened")
            return false
        }
        advanced.tap()

        let ready = app.staticTexts["Ready"]
        guard ready.waitForExistence(timeout: 90) else {
            let missingStates = [
                "MissingLocalAppKeys",
                "MissingPeerAppKeys",
                "MissingPeerInviteOrSession",
                "Unavailable",
            ]
            let readiness = missingStates.first { app.staticTexts[$0].exists } ?? "Unknown"
            XCTFail("peer protocol did not become ready: \(readiness)")
            return false
        }

        if ProcessInfo.processInfo.environment["IRIS_FIPS_IDLE_METRICS"] == "1" {
            let target = element(app, "physicalBluetoothTargetLink")
            guard target.waitForExistence(timeout: 5) else {
                XCTFail("exact Bluetooth target diagnostics are missing")
                return false
            }
            let connected = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "value == %@", "Connected"), object: target
            )
            guard XCTWaiter.wait(for: [connected], timeout: 60) == .completed else {
                XCTFail("the saved Android device has no authenticated Bluetooth link")
                return false
            }
            print("IRIS_FIPS_TARGET_BLE_CONNECTED")
        }

        let back = element(app, "navigationBackButton")
        guard back.waitForExistence(timeout: 5) else {
            XCTFail("could not return to chat after readiness check")
            return false
        }
        back.tap()
        return element(app, "chatMessageInput").waitForExistence(timeout: 10)
    }

    private func sendAndWaitForReceipt(
        _ message: String,
        in app: XCUIApplication,
        receiptTimeout: TimeInterval,
        verifyTransportTrace: Bool
    ) -> Bool {
        let composer = element(app, "chatMessageInput")
        composer.tap()
        composer.typeText(message)
        let send = element(app, "chatSendButton")
        guard send.waitForExistence(timeout: 5) else {
            XCTFail("BLE probe send button did not appear")
            return false
        }
        send.tap()

        let body = app.staticTexts[message]
        guard body.waitForExistence(timeout: 15) else {
            XCTFail("outgoing BLE probe did not appear")
            return false
        }
        let handedOff = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value IN %@", ["Sent", "Received", "Seen"]),
            object: body
        )
        guard XCTWaiter.wait(for: [handedOff], timeout: 60) == .completed else {
            XCTFail("message never left the protocol queue after peer bootstrap")
            return false
        }
        let received = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value IN %@",
                ProcessInfo.processInfo.environment["IRIS_FIPS_IDLE_METRICS"] == "1"
                    ? ["Seen"] : ["Received", "Seen"]),
            object: body
        )
        guard XCTWaiter.wait(for: [received], timeout: receiptTimeout) == .completed else {
            XCTFail("FIPS BLE receipt did not arrive")
            return false
        }
        guard verifyTransportTrace else { return true }

        body.press(forDuration: 0.6)
        guard element(app, "messageActionsSheet").waitForExistence(timeout: 10) else {
            XCTFail("BLE probe actions did not appear")
            return false
        }
        let info = app.buttons["Info"]
        guard info.waitForExistence(timeout: 5) else {
            XCTFail("BLE probe info action did not appear")
            return false
        }
        info.tap()
        guard element(app, "messageInfoSheet").waitForExistence(timeout: 10),
              app.staticTexts["FIPS nearby"].waitForExistence(timeout: 10) else {
            XCTFail("receipt arrived without the FIPS nearby transport trace")
            return false
        }
        return true
    }

    private func burstMessage(prefix: String, index: Int, size: Int) -> String {
        let header = burstMessageHeader(prefix: prefix, index: index)
        return header + String(repeating: "x", count: max(size - header.count, 0))
    }

    private func burstMessageHeader(prefix: String, index: Int) -> String {
        "\(prefix)-\(String(format: "%03d", index))-"
    }

    private func element(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }
}
