#if os(iOS)
import XCTest
@testable import IrisChat

final class IosDemoLocalLinkTests: XCTestCase {
    func testOfflineDemoPeersExchangeMessagesAndCallSignaling() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let a = FfiApp(dataDir: root.appendingPathComponent("a").path, keychainGroup: "", appVersion: "test")
        let b = FfiApp(dataDir: root.appendingPathComponent("b").path, keychainGroup: "", appVersion: "test")
        defer { a.shutdown(); b.shutdown(); try? FileManager.default.removeItem(at: root) }
        for app in [a, b] {
            app.dispatch(action: .setNostrRelays(relayUrls: []))
            app.dispatch(action: .setNearbyLanEnabled(enabled: false))
            app.dispatch(action: .setNearbyBluetoothEnabled(enabled: true))
        }
        let link = try IosDemoLocalLink(first: a, second: b)
        a.dispatch(action: .createAccount(name: "Demo reviewer"))
        b.dispatch(action: .createAccount(name: "Demo helper"))
        try await eventually("accounts") { a.state().account != nil && b.state().account != nil }
        let aID = try XCTUnwrap(a.state().account?.publicKeyHex)
        let bID = try XCTUnwrap(b.state().account?.publicKeyHex)
        a.dispatch(action: .createChat(peerInput: bID))
        b.dispatch(action: .createChat(peerInput: aID))
        a.dispatch(action: .sendMessage(chatId: bID, text: "Hello from the demo"))
        try await eventually("message arrives through the local encrypted link") {
            b.chatSnapshot(chatId: aID, limit: 20)?.messages.contains { $0.body == "Hello from the demo" } == true
        }
        b.dispatch(action: .sendMessage(chatId: aID, text: "Reply from helper"))
        try await eventually("reply") {
            a.chatSnapshot(chatId: bID, limit: 20)?.messages.contains { $0.body == "Reply from helper" } == true
        }
        a.dispatch(action: .startCall(chatId: bID, video: true))
        try await eventually("incoming call") { b.state().call?.phase == "incoming" }
        let callID = try XCTUnwrap(b.state().call?.callId)
        b.dispatch(action: .answerCall(callId: callID))
        try await eventually("connected call") { a.state().call?.phase == "connected" && b.state().call?.phase == "connected" }
        a.dispatch(action: .endCall(callId: callID))
        try await eventually("both ends stop") {
            [a.state().call, b.state().call].allSatisfy { $0 == nil || $0?.phase == "ended" }
        }
        // Active endpoints must process their shutdown commands before close
        // returns and before either core accepts a replacement host adapter.
        try link.close()
        let replacement = try IosDemoLocalLink(first: a, second: b)
        try replacement.close()
    }

    private func eventually(_ description: String, _ ready: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(30)
        while !ready() {
            guard Date() < deadline else { XCTFail("Timed out: \(description)"); throw DemoTestError.timeout }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
    }
    private enum DemoTestError: Error { case timeout }
}
#endif
