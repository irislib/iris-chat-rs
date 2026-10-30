import XCTest

/// Standalone, opt-in observations of an independently verified installed app.
/// All attachments contain private UI evidence; never publish the xcresult.
final class InstalledQualityUITests: XCTestCase {
    private let app = XCUIApplication(bundleIdentifier: "fi.siriusbusiness.irischat")
    private let environment = ProcessInfo.processInfo.environment
    private var sequence = 0
    private var events: [[String: Any]] = []
    private var ownProfileRouteSelected = false
    private var clock: TimeInterval { ProcessInfo.processInfo.systemUptime }
    private enum ObservationError: Error { case failed(String) }

    private struct Original {
        let row: String
        let message: String
        let body: String
        let draft: String
    }

    private func require(_ condition: Bool, _ reason: String) throws {
        guard condition else { throw ObservationError.failed(reason) }
    }

    private func configure(_ mode: String) throws -> Original {
        #if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("Installed quality observations require the reserved physical iPhone")
        #else
        guard environment["IRIS_INSTALLED_QUALITY_MODE"] == mode else {
            throw XCTSkip("Enable this installed-app observation explicitly")
        }
        continueAfterFailure = false
        let run = try XCTUnwrap(environment["IRIS_QUALITY_RUN_ID"])
        let binding = try XCTUnwrap(environment["IRIS_QUALITY_CANDIDATE_BINDING_SHA256"])
        try require(UUID(uuidString: run) != nil, "Use a fresh UUID for this observation")
        try require(binding.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil,
                    "Bind the independently verified installed-candidate receipt")
        // Discovery derives the row from actual UI; other modes still require it.
        let row = mode == "discover-row" ? "" : try XCTUnwrap(environment["IRIS_QUALITY_ORIGINAL_CHAT_ROW"])
        let original = Original(
            row: row,
            message: try XCTUnwrap(environment["IRIS_ACCEPTANCE_MESSAGE_IDENTIFIER"]),
            body: try XCTUnwrap(environment["IRIS_ACCEPTANCE_MESSAGE_BODY"]),
            draft: try XCTUnwrap(environment["IRIS_ACCEPTANCE_DRAFT"]))
        if mode != "discover-row" {
            try require(original.row.hasPrefix("chatRow-") && original.row.count == 20,
                        "Supply the exact original chat row identifier")
        }
        try require(original.message.hasPrefix("chatMessage-") && !original.body.isEmpty,
                    "Supply the exact existing message, not a newly created fixture")
        events.append(["event": "binding", "run_id": run, "mode": mode,
                       "candidate_binding_sha256": binding, "wall": Date().timeIntervalSince1970,
                       "uptime": clock])
        return original
        #endif
    }

    func testDiscoverOriginalChatRowAndReturn() throws {
        let original = try configure("discover-row")
        defer { finish() }
        try require(app.state == .runningForeground, "The original chat must already be foreground")
        try require(!original.draft.isEmpty
                    && original.draft == original.draft.trimmingCharacters(in: .whitespacesAndNewlines),
                    "Discovery requires the exact nonempty draft without trimmed whitespace")
        try verifyOriginal(original, name: "discovery-original")
        try requirePlainComposer()
        let header = app.buttons["chatHeaderTitleButton"]
        try require(header.exists && header.isHittable, "Require the actual current chat header")
        let titles = header.staticTexts.allElementsBoundByIndex.filter { $0.exists && $0.isHittable }
        try require(titles.count == 1, "Require one unambiguous visible header title")
        let title = titles[0].label
        try require(!title.isEmpty && header.label == title, "Header must expose the exact title alone")
        // The tagged UIKit cell joins components with a literal comma-space.
        // Delimiters inside input fields cannot be parsed unambiguously here.
        try require(!title.contains(", ") && !original.draft.contains(", "),
                    "Title/draft contains the row-label separator; stop instead of guessing")
        try tap("navigationBackButton") // One Back, no search, scroll or other fallback.
        let table = app.tables["chatListTable"]
        try require(table.waitForExistence(timeout: 10) && table.isHittable, "Require the actual chat list")
        _ = try capture("discovery-chat-list")
        let rows = table.cells.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chatRow-"))
        let candidates = rows.allElementsBoundByIndex.filter { row in
            guard row.exists && row.isHittable && !row.frame.isEmpty
                    && table.frame.contains(row.frame) && app.frame.contains(row.frame) else { return false }
            let parts = row.label.components(separatedBy: ", ")
            let previewIndex = parts.count == 4 && parts[1] == "Nearby" ? 2 : 1
            return (parts.count == 3 || (parts.count == 4 && previewIndex == 2))
                && parts[0] == title && parts[previewIndex] == "Draft: " + original.draft
        }
        events.append(["event": "return-row-candidates", "uptime": clock, "header_title": title,
                       "matches": candidates.map { ["identifier": $0.identifier, "label": $0.label] }])
        try require(candidates.count == 1, "Require exactly one visible row with the exact title and draft")
        let row = candidates[0]
        let identifier = row.identifier
        let label = row.label
        try require(identifier.hasPrefix("chatRow-") && identifier.count == 20,
                    "Record only the actual complete row identifier")
        events.append(["event": "return-row-discovered", "uptime": clock,
                       "row_identifier": identifier, "row_label": label])
        try require(row.exists && row.isHittable && row.label == label, "Discovered row changed before return")
        try tap(identifier)
        try verifyOriginal(original, name: "discovery-original-restored")
    }

    /// The external driver waits for READY, then performs an ordinary cold launch.
    /// This runner neither launches nor activates Iris, including on failure.
    func testObserveExternalColdStartup() throws {
        let original = try configure("startup")
        defer { finish() }
        try require(app.state == .notRunning, "Iris must already be stopped by the external driver")
        _ = try capture("before-external-launch")
        print("IRIS_INSTALLED_QUALITY_READY=\(environment["IRIS_QUALITY_RUN_ID"]!)")
        let deadline = clock + 60
        var started = false
        var openedOriginal = false
        var preservedSince: TimeInterval?
        while clock < deadline && sequence < 120 {
            let nodes = try capture("startup")
            if app.state != .notRunning { started = true }
            try require(!started || app.state != .notRunning, "The observed process stopped; do not retry")
            if app.state == .runningForeground {
                if preserved(nodes, original) {
                    if preservedSince == nil { preservedSince = clock }
                    if clock - preservedSince! >= 2 {
                        let finalNodes = try capture("startup-preserved-state")
                        try require(app.state == .runningForeground && preserved(finalNodes, original),
                                    "Preserved foreground state changed before final capture")
                        events.append(["event": "preserved-state-observed", "uptime": clock])
                        return
                    }
                } else {
                    preservedSince = nil
                    // Ordinary startup may restore the list. Open only the exact
                    // previously verified row, once, without editing its draft.
                    if !openedOriginal && visible(nodes, "chatListProfileButton") {
                        try tap(original.row)
                        openedOriginal = true
                    }
                }
            } else {
                preservedSince = nil
            }
            Thread.sleep(forTimeInterval: 0.1)
        }
        throw ObservationError.failed("Cold startup did not reach the preserved state within the observation budget")
    }

    /// Read-only recovery after a stopped observation; no typing or sending.
    func testReturnToOriginalConversation() throws {
        let original = try configure("return-original")
        defer { finish() }
        try require(app.state == .runningForeground, "The installed chat must already be foreground")
        try requirePlainComposer()
        _ = try capture("return-from-current-chat")
        try tap("navigationBackButton")
        try tap(original.row)
        try verifyOriginal(original, name: "original-restored")
    }

    func testSendOneNoteToSelfAndReturn() throws {
        let original = try configure("self-send")
        defer { finish() }
        try require(app.state == .runningForeground, "The installed app must already be foreground")
        try verifyOriginal(original, name: "original-before")
        try requirePlainComposer()

        // Prove the supplied return row resolves to the original message/draft
        // before opening Note to self or typing anything.
        try tap("navigationBackButton")
        try tap(original.row)
        try verifyOriginal(original, name: "original-return-preflight")
        try tap("navigationBackButton")
        try tap("chatListProfileButton")
        try tap("settingsProfileRow")
        let note = app.buttons["myProfileNoteToSelfButton"]
        try require(note.waitForExistence(timeout: 10) && note.isHittable,
                    "The dedicated own-profile Note to self action must be visible")
        _ = try capture("own-profile-note-to-self-action")
        note.tap() // Tagged SettingsScreen opens account.publicKeyHex, never a search result.
        ownProfileRouteSelected = true
        let input = app.textViews["chatMessageInput"]
        try require(input.waitForExistence(timeout: 10), "Note to self composer did not open")
        try requireSelfChat()
        try requirePlainComposer()
        try requireEmptyComposer(input, reason: "Preserve the existing Note to self draft")
        _ = try capture("self-before-typing")

        let body = "Iris quality check " + environment["IRIS_QUALITY_RUN_ID"]!
        input.tap()
        try requireSelfChat()
        try requireEmptyComposer(input, reason: "Draft changed before typing; preserve it")
        input.typeText(body)
        try sendPendingSelfMessage(body, input: input, original: original)
    }

    /// Finish the exact diagnosed synthetic draft; never type or replace it.
    func testFinishPendingNoteToSelfAndReturn() throws {
        let original = try configure("pending-self-send")
        defer { finish() }
        let pendingRun = try XCTUnwrap(environment["IRIS_QUALITY_PENDING_RUN_ID"])
        try require(UUID(uuidString: pendingRun) != nil, "Bind the diagnosed pending run")
        let body = "Iris quality check " + pendingRun
        try require(app.state == .runningForeground, "The pending self draft must already be foreground")
        try requirePlainComposer()
        let input = app.textViews["chatMessageInput"]
        _ = try capture("pending-self-draft-before-navigation")
        try require(input.exists && input.isHittable && (input.value as? String) == body,
                    "Preserve any draft other than the exact diagnosed synthetic text")
        try requireNoMatchingMessage(body)
        try tap("navigationBackButton")
        try tap(original.row)
        try verifyOriginal(original, name: "original-return-preflight")
        try tap("navigationBackButton")
        try tap("chatListProfileButton")
        try tap("settingsProfileRow")
        let note = app.buttons["myProfileNoteToSelfButton"]
        try require(note.waitForExistence(timeout: 10) && note.isHittable,
                    "The dedicated own-profile Note to self action must be visible")
        _ = try capture("own-profile-note-to-self-action")
        note.tap() // This tagged action opens account.publicKeyHex directly.
        ownProfileRouteSelected = true
        try require(input.waitForExistence(timeout: 10), "Note to self composer did not open")
        // No navigation or typing occurs between this own-account action and Send.
        try sendPendingSelfMessage(body, input: input, original: original)
    }

    private func requireNoMatchingMessage(_ body: String) throws {
        let matches = app.staticTexts.matching(NSPredicate(
            format: "identifier BEGINSWITH %@ AND label == %@", "chatMessage-", body))
        try require(!matches.firstMatch.exists, "Matching message already exists; do not send again")
    }

    private func sendPendingSelfMessage(_ body: String, input: XCUIElement, original: Original) throws {
        try requireSelfChat()
        try requirePlainComposer()
        try require(input.exists && input.isHittable && (input.value as? String) == body,
                    "Pending text differs; do not send or erase it")
        try requireNoMatchingMessage(body)
        let send = app.buttons["chatSendButton"]
        try require(send.exists && send.isHittable && send.isEnabled, "Send must be ready")
        _ = try capture("self-before-send")
        let before = clock
        events.append(["event": "send-tap-start", "uptime": before, "wall": Date().timeIntervalSince1970])
        send.tap() // Exactly one send; no retry, newline-submit, deletion or draft replacement.
        events.append(["event": "send-tap-return", "uptime": clock])
        var seen = false
        let deadline = before + 20
        while clock < deadline {
            let nodes = try capture("self-send-observation")
            let message = app.staticTexts.matching(NSPredicate(
                format: "identifier BEGINSWITH %@ AND label == %@", "chatMessage-", body)).firstMatch
            if nodes.contains(where: { $0.identifier.hasPrefix("chatMessage-") && $0.label == body })
                && message.exists && message.isHittable {
                events.append(["event": "sent-message-first-observed", "uptime": clock,
                               "send_to_visible_upper_bound_seconds": clock - before])
                seen = true
                break
            }
            Thread.sleep(forTimeInterval: 0.1)
        }
        try require(seen, "Synthetic self-message was not observed; preserve evidence and do not resend")
        _ = try capture("self-message-visible")
        try requireEmptyComposer(input, reason: "The sent draft must be cleared by the app")
        try requireSelfChat()
        try tap("navigationBackButton")
        try tap(original.row)
        try verifyOriginal(original, name: "original-restored")
    }

    private func requirePlainComposer() throws {
        for identifier in ["chatSelectedAttachments", "chatAttachmentLoading", "chatReplyComposer",
                           "chatEditingBanner", "chatVoicePreview", "chatVoiceSendButton", "chatVoiceRecording"] {
            try require(!app.descendants(matching: .any)[identifier].exists,
                        "Preserve pending attachments, reply, edit or voice recording: " + identifier)
        }
        try require(!app.alerts.firstMatch.exists && !app.sheets.firstMatch.exists,
                    "Do not dismiss an alert or sheet")
    }

    private func requireEmptyComposer(_ input: XCUIElement, reason: String) throws {
        try require(input.exists && input.isHittable, "The composer must be visible")
        let value = input.value
        try require(value == nil || value is String, "Unexpected composer accessibility value")
        // UIKit may expose nil for empty text. The tagged SwiftUI placeholder is
        // visible only when the draft is exactly empty; nonempty drafts show Send.
        let placeholder = app.staticTexts.matching(NSPredicate(format: "label == %@", "Message"))
            .allElementsBoundByIndex.contains { $0.exists && !$0.frame.isEmpty && $0.frame.intersects(input.frame) }
        let emptyValue = value == nil || (value as? String) == "" || (value as? String) == "Message"
        try require(emptyValue && placeholder
                    && !app.buttons["chatSendButton"].exists
                    && app.descendants(matching: .any)["chatVoiceRecordButton"].exists, reason)
        events.append(["event": "empty-composer-observed", "uptime": clock,
                       "value_was_nil": value == nil, "value": (value as? String) ?? "", "placeholder_over_input": placeholder])
    }

    private func requireSelfChat() throws {
        try require(app.state == .runningForeground, "Iris left the foreground")
        // The dedicated action establishes the recipient. Display names and call
        // controls can load asynchronously and do not authenticate the chat.
        try require(ownProfileRouteSelected, "Use only the dedicated own-profile action")
        try require(app.textViews["chatMessageInput"].exists, "The own-chat composer must exist")
        try require(!app.descendants(matching: .any)["settingsScreen"].exists,
                    "Settings must have closed before touching the composer")
    }

    private func tap(_ identifier: String) throws {
        let matches = app.descendants(matching: .any).matching(identifier: identifier)
        let target = matches.firstMatch
        try require(target.waitForExistence(timeout: 10) && matches.count == 1 && target.isHittable,
                    "Require one exact visible navigation target: " + identifier)
        target.tap()
    }

    private func verifyOriginal(_ original: Original, name: String) throws {
        let message = app.staticTexts.matching(identifier: original.message)
            .matching(NSPredicate(format: "label == %@", original.body)).firstMatch
        try require(message.waitForExistence(timeout: 10), "The exact original message must remain")
        let nodes = try capture(name)
        try require(app.state == .runningForeground, "Original chat must be foreground")
        try require(preserved(nodes, original), "The original message or draft changed")
    }

    private func preserved(_ nodes: [XCUIElementSnapshot], _ original: Original) -> Bool {
        nodes.contains { $0.identifier == original.message && $0.label == original.body }
            && nodes.contains { $0.identifier == "chatMessageInput" && ($0.value as? String) == original.draft }
            && app.staticTexts.matching(identifier: original.message)
                .matching(NSPredicate(format: "label == %@", original.body)).firstMatch.isHittable
            && app.textViews["chatMessageInput"].isHittable
    }

    private func visible(_ nodes: [XCUIElementSnapshot], _ identifier: String) -> Bool {
        nodes.contains { $0.identifier == identifier }
    }

    /// Each image precedes its AX query. Store both clock bounds: XCTest may
    /// wait for quiescence, so samples do not prove absence between captures.
    @discardableResult private func capture(_ phase: String) throws -> [XCUIElementSnapshot] {
        sequence += 1
        let began = clock
        let wall = Date().timeIntervalSince1970
        let screen = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screen.name = String(format: "%03d-", sequence) + phase
        screen.lifetime = .keepAlways
        add(screen)
        let imageAt = clock
        let state = app.state
        var nodes: [XCUIElementSnapshot] = []
        if state != .notRunning && state != .unknown {
            let snapshot = try app.snapshot()
            var pending = [snapshot]
            while let node = pending.popLast() {
                if !node.frame.isEmpty && node.frame.intersects(snapshot.frame) { nodes.append(node) }
                pending.append(contentsOf: node.children)
            }
        }
        let login = Set(["welcomeCreateAction", "welcomeRestoreAction", "createAccountScreen",
                         "restoreAccountScreen", "signupNameField", "generateKeyButton",
                         "restoreLinkDeviceAction", "onboardingTermsAgreementToggle"])
        events.append(["event": phase, "sequence": sequence, "wall": wall,
                       "uptime_start": began, "screenshot_complete": imageAt,
                       "snapshot_complete": clock, "application_state": state.rawValue,
                       "nodes": nodes.map { ["identifier": $0.identifier, "label": $0.label,
                                               "value": ($0.value as? String) ?? "",
                                               "type": $0.elementType.rawValue] as [String: Any] }])
        try require(!nodes.contains { login.contains($0.identifier) }, "Observed login UI: startup quality fails")
        return nodes
    }

    private func finish() {
        let screen = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screen.name = "installed-quality-final-state"
        screen.lifetime = .keepAlways
        add(screen)
        let data = try? JSONSerialization.data(withJSONObject: events, options: [.prettyPrinted, .sortedKeys])
        let attachment = XCTAttachment(string: data.flatMap { String(data: $0, encoding: .utf8) } ?? "event serialization failed")
        attachment.name = "installed-quality-private-observations"
        attachment.lifetime = .keepAlways
        add(attachment)
        // Failure is deliberately not repaired by navigation, deletion or relaunch.
        // The first failure and any unsent synthetic draft remain for review.
    }
}
