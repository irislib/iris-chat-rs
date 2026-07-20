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
        sendAndWaitForReceipt(
            message,
            in: app,
            receiptTimeout: receiptTimeout,
            verifyTransportTrace: true
        )
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
        sendAndWaitForReceipt(
            firstMessage,
            in: app,
            receiptTimeout: receiptTimeout,
            verifyTransportTrace: false
        )

        app.terminate()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "0"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        XCTAssertTrue(
            element(app, "chatMessageInput").waitForExistence(timeout: 30),
            "chat did not restore after relaunch"
        )
        print("IRIS_FIPS_RECONNECTED_READY_TO_SEND")
        sendAndWaitForReceipt(
            secondMessage,
            in: app,
            receiptTimeout: receiptTimeout,
            verifyTransportTrace: true
        )
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
    ) {
        let composer = element(app, "chatMessageInput")
        composer.tap()
        composer.typeText(message)
        let send = element(app, "chatSendButton")
        XCTAssertTrue(send.waitForExistence(timeout: 5))
        send.tap()

        let body = app.staticTexts[message]
        XCTAssertTrue(body.waitForExistence(timeout: 15), "outgoing BLE probe did not appear")
        let handedOff = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value IN %@", ["Sent", "Received", "Seen"]),
            object: body
        )
        guard XCTWaiter.wait(for: [handedOff], timeout: 60) == .completed else {
            XCTFail("message never left the protocol queue after peer bootstrap")
            return
        }
        let received = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value IN %@", ["Received", "Seen"]),
            object: body
        )
        guard XCTWaiter.wait(for: [received], timeout: receiptTimeout) == .completed else {
            XCTFail("FIPS BLE receipt did not arrive")
            return
        }
        guard verifyTransportTrace else { return }

        body.press(forDuration: 0.6)
        XCTAssertTrue(element(app, "messageActionsSheet").waitForExistence(timeout: 10))
        let info = app.buttons["Info"]
        XCTAssertTrue(info.waitForExistence(timeout: 5))
        info.tap()
        XCTAssertTrue(element(app, "messageInfoSheet").waitForExistence(timeout: 10))
        XCTAssertTrue(
            app.staticTexts["FIPS nearby"].waitForExistence(timeout: 10),
            "receipt arrived without the FIPS nearby transport trace"
        )
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
