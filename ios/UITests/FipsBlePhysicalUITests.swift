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
        app.launchEnvironment["IRIS_PERF_LOG"] = environment["IRIS_PERF_LOG"]
        app.launchEnvironment["IRIS_RUNTIME_DEBUG_SNAPSHOT"] = environment["IRIS_RUNTIME_DEBUG_SNAPSHOT"]
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        dismissBlockingSystemAlertIfPresent()

        createTestAccount(in: app)
        if environment["IRIS_FIPS_BACKGROUND_REPLY"] != nil {
            enableReadReceipts(in: app)
        }
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
        if let reply = environment["IRIS_FIPS_BACKGROUND_REPLY"] {
            element(app, "messageInfoCloseButton").tap()
            // Activating another app also works on devices where XCTest's
            // synthetic Home press is ignored. Verify the state explicitly.
            let settings = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
            settings.activate()
            let enteredBackground = app.wait(for: .runningBackground, timeout: 5)
                || app.state == .runningBackgroundSuspended
            guard enteredBackground else {
                XCTFail("Iris did not enter the background; state=\(app.state.rawValue)")
                return
            }
            print("IRIS_FIPS_BACKGROUND_ENTERED=\(Int64(Date().timeIntervalSince1970 * 1000))")
            Thread.sleep(forTimeInterval: 75)
            XCTAssertTrue(app.state == .runningBackground || app.state == .runningBackgroundSuspended,
                          "Iris left the background; state=\(app.state.rawValue)")
            print("IRIS_FIPS_BACKGROUND_LEAVING=\(Int64(Date().timeIntervalSince1970 * 1000))")
            app.activate()
            XCTAssertTrue(app.staticTexts[reply].waitForExistence(timeout: 10), "Background Bluetooth reply is missing")
        }
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
        guard close.waitForExistence(timeout: 5) || environment["IRIS_FIPS_BACKGROUND_REPLY"] != nil else {
            XCTFail("could not close the verified BLE probe before idle measurement")
            return
        }
        if close.exists { close.tap() }
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

    /// Opt-in physical gate: a group keeps working over Bluetooth while both
    /// phones are offline, and catches up with an online member afterwards.
    /// Driven by `scripts/physical_ble_group.py`, which takes the Android
    /// member offline and keeps a third member online.
    func testGroupMessagesOverFipsBleWhileOffline() throws {
#if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("Offline Bluetooth group gate needs a physical iPhone")
#else
        let environment = ProcessInfo.processInfo.environment
        guard let members = environment["IRIS_FIPS_GROUP_MEMBERS"]?
                .split(separator: ",").map(String.init), !members.isEmpty,
              let groupName = environment["IRIS_FIPS_GROUP_NAME"],
              let offlineIncoming = environment["IRIS_FIPS_GROUP_EXPECT_OFFLINE"],
              let offlineOutgoing = environment["IRIS_FIPS_GROUP_SEND_OFFLINE"],
              let offlineAck = environment["IRIS_FIPS_GROUP_OFFLINE_ACK"],
              let onlineIncoming = environment["IRIS_FIPS_GROUP_EXPECT_ONLINE"] else {
            throw XCTSkip("Run through scripts/physical_ble_group.py")
        }
        let readyMessages = (environment["IRIS_FIPS_GROUP_READY_MESSAGES"] ?? "")
            .split(separator: ",").map(String.init)

        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = environment["IRIS_FIPS_PHYSICAL_RUN_ID"] ?? "fips-ble-group"
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        dismissBlockingSystemAlertIfPresent()
        createTestAccount(in: app)
        enableFipsBluetooth(in: app)

        element(app, "chatListNewChatButton").tap()
        let newGroup = element(app, "newChatNewGroupButton")
        XCTAssertTrue(newGroup.waitForExistence(timeout: 15))
        newGroup.tap()
        let memberInput = element(app, "newGroupMemberInput")
        XCTAssertTrue(memberInput.waitForExistence(timeout: 10))
        for (index, member) in members.enumerated() {
            memberInput.tap()
            memberInput.typeText(member)
            let chips = app.descendants(matching: .any).matching(identifier: "memberChipRemove")
            let added = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "count >= %d", index + 1), object: chips
            )
            XCTAssertEqual(XCTWaiter.wait(for: [added], timeout: 10), .completed, "group member was not added")
        }
        element(app, "newGroupNextButton").tap()
        let nameInput = element(app, "newGroupNameInput")
        XCTAssertTrue(nameInput.waitForExistence(timeout: 10))
        nameInput.tap()
        nameInput.typeText(groupName)
        element(app, "newGroupCreateButton").tap()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 60))
        print("IRIS_FIPS_GROUP_CREATED")

        for message in readyMessages {
            XCTAssertTrue(app.staticTexts[message].waitForExistence(timeout: 300),
                          "member did not join the group while online: \(message)")
        }
        print("IRIS_FIPS_GROUP_MEMBERS_READY")

        let previous = setPhoneNetwork(enabled: false)
        // A fresh test runner cannot launch offline (iOS must verify the developer
        // certificate online), so restore from this process even if a check fails.
        addTeardownBlock { [weak self] in
            self?.setPhoneNetwork(enabled: true, only: previous.filter { $0.value })
        }
        print("IRIS_FIPS_PHONE_NETWORK_PREVIOUS=" + previous.map { "\($0.key):\($0.value ? 1 : 0)" }
            .sorted().joined(separator: ","))
        app.activate()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 20))
        print("IRIS_FIPS_IPHONE_OFFLINE")

        XCTAssertTrue(app.staticTexts[offlineIncoming].waitForExistence(timeout: 300),
                      "offline group message did not arrive over Bluetooth")
        print("IRIS_FIPS_GROUP_OFFLINE_RECEIVED")
        typeExactly(offlineOutgoing, into: element(app, "chatMessageInput"))
        element(app, "chatSendButton").tap()
        XCTAssertTrue(app.staticTexts[offlineOutgoing].waitForExistence(timeout: 15))
        print("IRIS_FIPS_GROUP_OFFLINE_SENT")
        XCTAssertTrue(app.staticTexts[offlineAck].waitForExistence(timeout: 300),
                      "offline group reply was not acknowledged over Bluetooth")
        print("IRIS_FIPS_GROUP_OFFLINE_ACKED")

        _ = setPhoneNetwork(enabled: true, only: previous)
        app.activate()
        print("IRIS_FIPS_IPHONE_ONLINE")
        XCTAssertTrue(app.staticTexts[onlineIncoming].waitForExistence(timeout: 300),
                      "message from the online member did not arrive after reconnecting")
        print("IRIS_FIPS_GROUP_DONE")
#endif
    }

    /// Always run by the driver afterwards, so a failed group run cannot leave
    /// the phone offline.
    func testRestorePhoneNetworkAfterBleGroup() throws {
#if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("Physical iPhone only")
#else
        guard let spec = ProcessInfo.processInfo.environment["IRIS_FIPS_RESTORE_NETWORK"] else {
            throw XCTSkip("Run through scripts/physical_ble_group.py")
        }
        var wanted: [String: Bool] = [:]
        for pair in spec.split(separator: ",") {
            let parts = pair.split(separator: ":")
            if parts.count == 2 { wanted[String(parts[0])] = parts[1] == "1" }
        }
        // Restoring an already-on radio is a no-op, so this is safe after success too.
        setPhoneNetwork(enabled: true, only: wanted.filter { $0.value })
        print("IRIS_FIPS_PHONE_NETWORK_RESTORED")
#endif
    }

    /// Switches Wi-Fi and mobile data in Settings. Returns each radio's state
    /// before the change, only for radios this phone has.
    @discardableResult
    private func setPhoneNetwork(enabled: Bool, only: [String: Bool]? = nil) -> [String: Bool] {
        let settings = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
        let rows: [(String, NSPredicate)] = [
            ("wifi", NSPredicate(format: "identifier == 'com.apple.settings.wifi' OR label BEGINSWITH 'Wi'")),
            ("cellular", NSPredicate(format:
                "identifier == 'com.apple.settings.cellular' OR label IN {'Cellular', 'Mobile Service', 'Mobile Data'}")),
        ]
        var before: [String: Bool] = [:]
        for (radio, predicate) in rows {
            if let only, only[radio] == nil { continue }
            settings.terminate()
            settings.launch()
            let row = settings.descendants(matching: .any).matching(predicate)
                .matching(NSPredicate(format: "elementType IN {%d, %d}",
                                      XCUIElement.ElementType.cell.rawValue,
                                      XCUIElement.ElementType.button.rawValue)).firstMatch
            guard row.waitForExistence(timeout: 8) else {
                XCTAssertNotEqual(radio, "wifi", "Wi-Fi settings row is missing")
                continue
            }
            row.tap()
            let toggle = settings.switches.firstMatch
            guard toggle.waitForExistence(timeout: 8) else {
                XCTFail("\(radio) switch is missing")
                continue
            }
            let isOn = (toggle.value as? String) == "1"
            before[radio] = isOn
            if isOn != enabled {
                toggle.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).tap()
                let changed = XCTNSPredicateExpectation(
                    predicate: NSPredicate(format: "value == %@", enabled ? "1" : "0"), object: toggle
                )
                XCTAssertEqual(XCTWaiter.wait(for: [changed], timeout: 10), .completed,
                               "\(radio) did not switch \(enabled ? "on" : "off")")
            }
        }
        return before
    }

    private func typeExactly(_ text: String, into field: XCUIElement) {
        field.tap()
        var entered = ""
        let characters = Array(text)
        for offset in stride(from: 0, to: characters.count, by: 8) {
            let chunk = String(characters[offset..<min(offset + 8, characters.count)])
            field.typeText(chunk)
            entered += chunk
            let exact = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "value == %@", entered), object: field
            )
            XCTAssertEqual(XCTWaiter.wait(for: [exact], timeout: 3), .completed, "text was not entered exactly")
        }
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

    private func enableReadReceipts(in app: XCUIApplication) {
        element(app, "chatListProfileButton").tap()
        let messaging = app.buttons["Messaging"]
        XCTAssertTrue(messaging.waitForExistence(timeout: 10))
        messaging.tap()
        let receipts = element(app, "myProfileReadReceiptsToggle")
        for _ in 0..<8 where !receipts.isHittable { app.swipeUp() }
        XCTAssertTrue(receipts.isHittable)
        if (receipts.value as? String) != "1" { receipts.tap() }
        element(app, "settingsCloseButton").tap()
        XCTAssertTrue(element(app, "chatListNewChatButton").waitForExistence(timeout: 10))
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
        // Long synthetic key bursts can lose a character on a busy physical
        // phone. Verify each chunk before sending any fixture to the peer.
        var entered = ""
        let characters = Array(message)
        for offset in stride(from: 0, to: characters.count, by: 8) {
            let chunk = String(characters[offset..<min(offset + 8, characters.count)])
            composer.typeText(chunk)
            entered += chunk
            let exact = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "value == %@", entered), object: composer
            )
            guard XCTWaiter.wait(for: [exact], timeout: 3) == .completed else {
                XCTFail("BLE probe text was not entered exactly; nothing sent")
                return false
            }
        }
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
