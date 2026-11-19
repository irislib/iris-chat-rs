import XCTest

final class GroupPickerUITests: IrisChatUITestCase {
    // Public test vectors; no accounts or real contacts are used.
    private let peers = [
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5",
        "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",
        "e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13",
        "2f8bde4d1a07209355b4a7250a5c5128e88b84bddc619ab7cba8d569b240efe4",
        "fff97bd5755eeea420453a14355235d382f6472f8568a18b2f057a1460297556",
        "5cbdf0646e5db4eaa398f365f2ea7a0e3d419b7e0330e39ce92bddedcac4f9bc",
        "2f01e5e15cca351daff3843fb70f3c2f0a1bdd05e5af888a67784ef3e10a2a01",
        "acd484e2f0c7f65309ad178a9f559abde09796974c57e714c35f110dfc27ccbe",
        "a0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247c7",
        "774ae7f858a9411e5ef4246b70c65aac5649980be5c17891bbec17895da008cb",
        "d01115d548e7561b15c38f004d734633687cf4419620095bc5b0f47070afe85a",
        "f28773c2d975288bc7d1d205c3748651b075fbc6610e58cddeeddf8f19405aa8",
        "499fdf9e895e719cfd64e67f07d38e3226aa7b63678949e6e49b241a60e823e4",
        "d7924d4f7d43ea965a465ae3095ff41131e5946f3c85f79e44adbcf8e27e080e",
        "e60fce93b59e9ec53011aabc21c23e97b2a31369b87a5ae9c44ee89e2a6dec0a",
        "defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a34",
        "5601570cb47f238d2b0286db4a990fa0f3ba28d1a319f5e7cf55c2a2444da7cc",
        "2b4ea0a797a443d293ef5cff444f4979f06acfebd7e86d277475656138385b6c",
        "4ce119c96e2fa357200b559b2f7dd5a5f02d5290aff74b03f3e471b273211c97",
        "352bbf4a4cdd12564f93fa332ce333301d9ad40271f8107181340aef25be59d5",
        "421f5fc9a21065445c96fdb91c0c1e2f2431741c72713b4b99ddcb316f31e9fc",
        "2fa2104d6b38d11b0230010559879124e42ab8dfeff5ff29dc9cdadd4ecacc3f",
        "fe72c435413d33d48ac09c9161ba8b09683215439d62b7940502bda8b202e6ce",
    ]

    func testLongContactListCreationAndMemberManagement() {
        continueAfterFailure = false
#if os(macOS)
        executionTimeAllowance = 180
#endif
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = "group-picker-\(UUID().uuidString)"
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launchEnvironment["IRIS_DEMO_RELAYS"] = "ws://127.0.0.1:9"
        app.launchEnvironment["IRIS_UI_TEST_SCREENSHOT_FIXTURE"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_GROUP_CONTACTS"] = peers.joined(separator: ",")
        app.launch()
        createAccount(app)
        tapNewChat(app)
        element(app, "newChatNewGroupButton").tap()
        let search = editableElement(app, "newGroupMemberInput")
        XCTAssertTrue(search.waitForExistence(timeout: 10))
        let creationScroll = app.scrollViews.containing(.any, identifier: "newGroupMemberInput").firstMatch
        let candidateList = app.scrollViews["newGroupKnownUsersList"]
        let boundedCreationList = candidateList.exists
#if os(macOS)
        capture(app, "group-creation-before-scroll")
#endif
        let next = element(app, "newGroupNextButton")
        let nextFrame = next.frame
        // The same driver runs on the baseline: its whole page scrolls and
        // later fails the bounded-list assertion, after completing the journey.
        let creationCandidates = boundedCreationList ? candidateList : creationScroll
        let contact = boundedCreationList
            ? app.buttons["newGroupKnownUser-\(peers[20].prefix(12))"]
            : app.buttons.containing(.staticText, identifier: "Contact 21").firstMatch
        reveal(contact, in: creationCandidates, upward: true)
        if boundedCreationList {
            XCTAssertEqual(next.frame.minY, nextFrame.minY, accuracy: 1, "Browsing contacts must leave Next in place")
        }
        capture(app, "group-creation-contact-21")
        contact.tap()
        reveal(search, in: creationScroll, upward: false)
        typeText("Contact 02", into: search, app: app)
        let second = app.buttons.containing(.staticText, identifier: "Contact 02").firstMatch
        reveal(second, in: creationScroll, upward: true)
        second.tap()
        reveal(next, in: creationScroll, upward: false)
        XCTAssertEqual(next.label, "Next (2)", "Search must preserve the earlier selection")
        capture(app, "group-creation-two-selected")
        next.tap()
        typeText("Weekend crew", into: editableElement(app, "newGroupNameInput"), app: app)
        element(app, "newGroupCreateButton").tap()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 45))
        openGroupDetails(app)
        let detailsScroll = app.scrollViews.containing(.any, identifier: "groupDetailsNameInput").firstMatch
        let member = app.buttons["groupDetailsOpenMember-\(peers[20].prefix(12))"]
        reveal(member, in: detailsScroll, upward: true)
        let memberMenu = element(app, "groupDetailsMemberMenu-\(peers[20].prefix(12))")
        let compactMenu = memberMenu.exists
        if compactMenu {
            XCTAssertEqual(member.frame.midY, memberMenu.frame.midY, accuracy: 1)
            XCTAssertLessThanOrEqual(member.frame.height, 60, "Normal-size member rows should occupy one line")
            XCTAssertTrue(memberMenu.label.hasPrefix("Actions for "))
        }
        capture(app, "group-members-ordinary")
        openAdminAction(app, menu: memberMenu, owner: peers[20], title: "Make admin")
        let confirmation = app.alerts.firstMatch
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5))
        confirmation.buttons["Cancel"].tap()
        openAdminAction(app, menu: memberMenu, owner: peers[20], title: "Make admin")
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5))
        capture(app, "group-member-promotion-confirmation")
        confirmation.buttons["groupDetailsConfirmAdminButton"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Admin"].firstMatch.waitForExistence(timeout: 10))
        capture(app, "group-members-admin-and-ordinary")
        if compactMenu {
            memberMenu.tap()
#if os(macOS)
            XCTAssertTrue(app.menuItems["Dismiss admin"].firstMatch.waitForExistence(timeout: 5))
            app.typeKey(.escape, modifierFlags: [])
#else
            XCTAssertTrue(app.buttons["Dismiss admin"].firstMatch.waitForExistence(timeout: 5))
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.12)).tap()
#endif
        }

        let remaining = app.scrollViews["groupDetailsKnownUsersList"]
        reveal(remaining, in: detailsScroll, upward: true)
        let laterContact = app.buttons["groupDetailsKnownUser-\(peers[23].prefix(12))"]
        reveal(laterContact, in: remaining, upward: true)
        laterContact.tap()
        XCTAssertEqual(laterContact.value as? String, "Selected")
        capture(app, "group-details-select-contact-24")
        let add = element(app, "groupDetailsAddMembersButton")
        reveal(add, in: detailsScroll, upward: false)
        add.tap()
        XCTAssertTrue(waitUntil(timeout: 15) { !laterContact.exists })
        XCTAssertTrue(remaining.exists, "Remaining contacts must stay available after adding a member")
        let added = app.buttons["groupDetailsOpenMember-\(peers[23].prefix(12))"]
        reveal(added, in: detailsScroll, upward: false)
        XCTAssertTrue(app.staticTexts["4 people in this conversation."].exists)
        capture(app, "group-members-after-add")
        added.tap()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 15), "The member name must still open their chat")
        continueAfterFailure = true
        XCTAssertTrue(boundedCreationList, "Creation must use a bounded scrolling contact list")
        XCTAssertTrue(compactMenu, "Member actions must share one row with the avatar and name")
    }

    private func openAdminAction(_ app: XCUIApplication, menu: XCUIElement, owner: String, title: String) {
        if menu.exists {
            menu.tap()
#if os(macOS)
            app.menuItems[title].firstMatch.tap()
#else
            app.buttons[title].firstMatch.tap()
#endif
        } else {
            // Baseline compatibility only; the final assertion requires a menu.
            app.buttons["groupDetailsAdminMember-\(owner.prefix(12))"].tap()
        }
    }

    private func reveal(_ target: XCUIElement, in scroll: XCUIElement, upward: Bool,
                        file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<20 where !target.isHittable {
#if os(macOS)
            scroll.scroll(byDeltaX: 0, deltaY: upward ? -250 : 250)
#else
            if upward { scroll.swipeUp() } else { scroll.swipeDown() }
#endif
        }
        if !target.isHittable {
            let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
            attachment.name = "group-picker-unreachable-target"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        XCTAssertTrue(target.isHittable, "Target must be reachable by scrolling: \(target)", file: file, line: line)
    }

    private func capture(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
