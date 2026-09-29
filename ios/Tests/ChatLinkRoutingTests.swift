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
            for authority in ["chat.iris.to", "chat.iris.to:443", "CHAT.IRIS.TO:443"] {
                for suffix in ["/\(peer)", "/#/\(peer)"] {
                    let url = try XCTUnwrap(URL(string: "\(scheme)://\(authority)\(suffix)"))
                    guard case let .createChat(value) = IrisChatLinks.action(for: url) else {
                        return XCTFail("Expected a chat action for \(url)")
                    }
                    XCTAssertEqual(value, normalizePeerInput(input: peer))
                }
            }
        }
    }

    func testPrivateInviteFragmentSurvivesCustomSchemeWithoutReencoding() throws {
        let fragment = "/invite/%7B%22ephemeralKey%22%3A%22test%22%2C%22sharedSecret%22%3A%22a%2Fb%2B%3D%22%7D"
        for scheme in ["https", "irischat"] {
            for port in ["", ":443"] {
                for suffix in ["/invite/token%2Fpart?source=web#secret%2Bvalue+tail", "/#\(fragment)"] {
                    let url = try XCTUnwrap(URL(string: "\(scheme)://chat.iris.to\(port)\(suffix)"))
                    guard case let .acceptInvite(value) = IrisChatLinks.action(for: url) else {
                        return XCTFail("Expected an invite action for \(url)")
                    }
                    XCTAssertEqual(value, "https://chat.iris.to\(port)\(suffix)")
                }
            }
        }
    }

    func testNonCanonicalAuthoritiesAreRejectedForBothSchemes() throws {
        for scheme in ["https", "irischat"] {
            for authority in [
                "other.example",
                "chat.iris.to.other.example",
                "chat.iris.to@other.example",
                "user@chat.iris.to",
                "user:password@chat.iris.to:443",
                ":password@chat.iris.to:443",
                "@chat.iris.to:443",
                "chat.iris.to:80",
                "chat.iris.to:8443",
            ] {
                for suffix in ["/\(peer)", "/invite/token"] {
                    let url = try XCTUnwrap(URL(string: "\(scheme)://\(authority)\(suffix)"))
                    XCTAssertNil(IrisChatLinks.action(for: url), "\(url)")
                }
            }
        }
    }

    func testUnrelatedSchemesHostsAndSettingsAreIgnored() throws {
        for value in [
            "irischat://share/example",
            "http://chat.iris.to/#/\(peer)", "https://chat.iris.to/#settings",
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
