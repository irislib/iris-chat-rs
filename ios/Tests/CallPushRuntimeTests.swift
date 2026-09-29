#if os(iOS)
import XCTest
@testable import IrisChat

private final class CallPushHTTP: URLProtocol {
    static var respond: ((URLRequest) -> (Int, Data))?
    static var hold: ((CallPushHTTP) -> Void)?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        if let hold = Self.hold { hold(self); return }
        let (code, data) = Self.respond?(request) ?? (500, Data())
        finish(code, data)
    }
    func finish(_ code: Int, _ data: Data) {
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
    @MainActor
    func testBlockingWhileUpdateInFlightReconcilesAfterItEvenWhenReturningToConfirmedState() async throws {
        let suite = "call-push-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite); CallPushHTTP.hold = nil }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CallPushHTTP.self]
        let runtime = CallPushRuntime(defaults: defaults, session: URLSession(configuration: configuration))
        var state = makeAppState(rev: 1)
        state.mobilePush.callDevicePubkeyHex = String(repeating: "a", count: 64)
        state.mobilePush.callAuthorPubkeys = []
        state.preferences.mobilePushServerUrl = "https://push.test"
        let secret = String(repeating: "1", count: 64)
        // Establish the already-blocked state before an unblock request starts.
        runtime.sync(state: state, ownerNsec: secret, token: "token")
        await Task.yield()
        let started = expectation(description: "unblock request started")
        let removed = expectation(description: "new subscription removed after block")
        var pending: CallPushHTTP?
        var methods: [String] = []
        CallPushHTTP.hold = { request in
            Task { @MainActor in
                methods.append(request.request.httpMethod!)
                if request.request.httpMethod == "POST" {
                    pending = request
                    started.fulfill()
                } else {
                    XCTAssertEqual(request.request.url?.path, "/subscriptions/created")
                    request.finish(200, Data("{}".utf8))
                    removed.fulfill()
                }
            }
        }
        state.mobilePush.callAuthorPubkeys = [String(repeating: "b", count: 64)]
        runtime.sync(state: state, ownerNsec: secret, token: "token")
        await fulfillment(of: [started], timeout: 2)
        state.mobilePush.callAuthorPubkeys = []
        runtime.sync(state: state, ownerNsec: secret, token: "token")
        await Task.yield()
        XCTAssertEqual(methods, ["POST"], "Do not overlap a deletion with the in-flight create")
        pending?.finish(201, Data("{\"id\":\"created\"}".utf8))
        await fulfillment(of: [removed], timeout: 2)
        XCTAssertEqual(methods, ["POST", "DELETE"])
    }

    @MainActor
    func testFailedUpdateRetryUsesLatestBlockedState() async throws {
        let suite = "call-push-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite); CallPushHTTP.respond = nil }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CallPushHTTP.self]
        let runtime = CallPushRuntime(defaults: defaults, session: URLSession(configuration: configuration),
                                      retryDelayNanoseconds: 100_000_000)
        var state = makeAppState(rev: 1)
        state.mobilePush.callDevicePubkeyHex = String(repeating: "a", count: 64)
        state.mobilePush.callAuthorPubkeys = [String(repeating: "b", count: 64)]
        state.preferences.mobilePushServerUrl = "https://push.test"
        let secret = String(repeating: "1", count: 64)
        let created = expectation(description: "initial registration")
        CallPushHTTP.respond = { _ in
            created.fulfill()
            return (201, Data("{\"id\":\"existing\"}".utf8))
        }
        runtime.sync(state: state, ownerNsec: secret, token: "token")
        await fulfillment(of: [created], timeout: 2)
        for _ in 0..<30 where defaults.string(forKey: "settings.call_push_subscription_id.ios") == nil {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let failed = expectation(description: "update fails")
        CallPushHTTP.respond = { _ in failed.fulfill(); return (503, Data()) }
        state.mobilePush.callAuthorPubkeys.append(String(repeating: "c", count: 64))
        runtime.sync(state: state, ownerNsec: secret, token: "token")
        await fulfillment(of: [failed], timeout: 2)
        // Let the response handler schedule its retry before changing the policy.
        try await Task.sleep(nanoseconds: 30_000_000)
        let restored = expectation(description: "blocked caller removed from existing subscription")
        CallPushHTTP.respond = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertEqual(request.url?.path, "/subscriptions/existing")
            let body = self.bodyData(request)
            let object = try? JSONSerialization.jsonObject(with: body) as? [String: Any]
            let filter = object?["filter"] as? [String: Any]
            XCTAssertEqual(filter?["authors"] as? [String], [String(repeating: "b", count: 64)],
                           "Returning to the confirmed state must replace any possibly applied stale update")
            restored.fulfill()
            return (200, Data("{}".utf8))
        }
        state.mobilePush.callAuthorPubkeys = [String(repeating: "b", count: 64)]
        runtime.sync(state: state, ownerNsec: secret, token: "token")
        await fulfillment(of: [restored], timeout: 2)
        try await Task.sleep(nanoseconds: 200_000_000)
        XCTAssertEqual(defaults.string(forKey: "settings.call_push_subscription_id.ios"), "existing")
    }

    private func bodyData(_ request: URLRequest) -> Data {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return Data() }
        stream.open()
        defer { stream.close() }
        var body = Data()
        var bytes = [UInt8](repeating: 0, count: 1024)
        while stream.hasBytesAvailable {
            let count = stream.read(&bytes, maxLength: bytes.count)
            if count <= 0 { break }
            body.append(contentsOf: bytes.prefix(count))
        }
        return body
    }

}
#endif
