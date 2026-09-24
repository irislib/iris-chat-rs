#if os(iOS)
import XCTest
@testable import IrisChat

private final class CallPushHTTP: URLProtocol {
    static var respond: ((URLRequest) -> (Int, Data))?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let (code, data) = Self.respond?(request) ?? (500, Data())
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: code,
            httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

final class CallPushRuntimeTests: XCTestCase {
    @MainActor
    func testRegisterRotateAndDisableUseDedicatedSubscriptionAndVoipToken() async {
        let suite = "call-push-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite); CallPushHTTP.respond = nil }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CallPushHTTP.self]
        let runtime = CallPushRuntime(defaults: defaults, session: URLSession(configuration: configuration))
        var state = makeAppState(rev: 1)
        state.mobilePush.callDevicePubkeyHex = String(repeating: "a", count: 64)
        state.mobilePush.callAuthorPubkeys = [String(repeating: "b", count: 64)]
        state.preferences.mobilePushServerUrl = "https://push.test"
        state.preferences.desktopNotificationsEnabled = false
        let registered = expectation(description: "call subscription registered despite message alerts disabled")
        CallPushHTTP.respond = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertEqual(request.url?.path, "/subscriptions")
            XCTAssertTrue(request.value(forHTTPHeaderField: "authorization")?.hasPrefix("Nostr ") == true)
            registered.fulfill()
            return (201, Data("{\"id\":\"call-subscription\"}".utf8))
        }
        let secret = String(repeating: "1", count: 64)
        runtime.sync(state: state, ownerNsec: secret, token: "voip-token")
        await fulfillment(of: [registered], timeout: 2)
        // Wait for the response handler to commit its subscription ID.
        for _ in 0..<30 where defaults.string(forKey: "settings.call_push_subscription_id.ios") == nil {
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        let rotated = expectation(description: "rotated token updates subscription")
        CallPushHTTP.respond = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertEqual(request.url?.path, "/subscriptions/call-subscription")
            rotated.fulfill()
            return (200, Data("{}".utf8))
        }
        runtime.sync(state: state, ownerNsec: secret, token: "rotated-token")
        await fulfillment(of: [rotated], timeout: 2)
        let removed = expectation(description: "turning off calls unregisters")
        CallPushHTTP.respond = { request in
            XCTAssertEqual(request.httpMethod, "DELETE")
            XCTAssertEqual(request.url?.path, "/subscriptions/call-subscription")
            removed.fulfill()
            return (200, Data("{}".utf8))
        }
        state.preferences.voiceCallsEnabled = false
        state.preferences.videoCallsEnabled = false
        runtime.sync(state: state, ownerNsec: secret, token: "rotated-token")
        await fulfillment(of: [removed], timeout: 2)
    }
}
#endif
