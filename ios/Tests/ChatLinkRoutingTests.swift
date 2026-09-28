import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class ChatLinkRoutingTests: XCTestCase {
    private let peer = String(repeating: "ab", count: 32)

    func testCustomSchemeRoutesToSameChatAsUniversalLink() throws {
        for scheme in ["https", "irischat"] {
            let url = try XCTUnwrap(URL(string: "\(scheme)://chat.iris.to/#/\(peer)"))
            guard case let .createChat(value) = IrisChatLinks.action(for: url) else {
                return XCTFail("Expected a chat action")
            }
            XCTAssertEqual(value, normalizePeerInput(input: peer))
        }
    }

    func testPrivateInviteFragmentSurvivesCustomSchemeWithoutReencoding() throws {
        let fragment = "/invite/%7B%22ephemeralKey%22%3A%22test%22%2C%22sharedSecret%22%3A%22a%2Fb%2B%3D%22%7D"
        let url = try XCTUnwrap(URL(string: "irischat://chat.iris.to/#\(fragment)"))
        guard case let .acceptInvite(value) = IrisChatLinks.action(for: url) else {
            return XCTFail("Expected an invite action")
        }
        XCTAssertEqual(value, "https://chat.iris.to/#\(fragment)")
    }

    func testUnrelatedSchemesHostsAndSettingsAreIgnored() throws {
        for value in [
            "irischat://share/example", "irischat://other.example/#/\(peer)",
            "http://chat.iris.to/#/\(peer)", "https://chat.iris.to/#settings",
            "irischat://chat.iris.to@other.example/#/\(peer)",
            "irischat://user@chat.iris.to/#/\(peer)",
        ] {
            XCTAssertNil(IrisChatLinks.action(for: try XCTUnwrap(URL(string: value))))
        }
    }

    @MainActor
    func testChatLinkWaitsForOnboardingAndRunsOnlyOnce() async throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        let account = try XCTUnwrap(state.account)
        state.account = nil
        state.rev = 1
        let rust = MockRustApp(state: state)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        manager.handleChatLink(try XCTUnwrap(URL(string: "irischat://chat.iris.to/#/\(peer)")))
        XCTAssertTrue(chatActions(rust).isEmpty)
        state.account = account
        state.account?.authorizationState = .awaitingApproval
        state.rev = 2
        rust.emit(.fullState(state))
        let waiting = await waitUntil { manager.state.rev == 2 }
        XCTAssertTrue(waiting)
        XCTAssertTrue(chatActions(rust).isEmpty)
        state.account?.authorizationState = .authorized
        state.rev = 3
        rust.emit(.fullState(state))
        let opened = await waitUntil { self.chatActions(rust).count == 1 }
        XCTAssertTrue(opened)
        XCTAssertEqual(chatActions(rust), [normalizePeerInput(input: peer)])
        state.rev = 4
        rust.emit(.fullState(state))
        let settled = await waitUntil { manager.state.rev == 4 }
        XCTAssertTrue(settled)
        XCTAssertEqual(chatActions(rust).count, 1)
    }

    private func chatActions(_ rust: MockRustApp) -> [String] {
        rust.dispatchedActions.compactMap { action in
            if case let .createChat(peerInput) = action { return peerInput }
            return nil
        }
    }
}
