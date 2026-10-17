#if DEBUG && os(iOS)
import Foundation
import XCTest
@testable import IrisChat

extension InteropHarnessTests {
    /// Check before AppManager or storage initialization. The regular app shell
    /// must also use its own isolated run ID; this never opens ordinary storage.
    func validateAudioContinuityFixture(_ env: [String: String]) throws {
        guard IrisCallAudioContinuityProbe.accepts(env),
              env["IRIS_IOS_HARNESS_AUDIO_REFERENCE_KIND"] == "external-varying-voiced",
              let hash = env["IRIS_IOS_HARNESS_AUDIO_REFERENCE_SHA256"],
              hash.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil,
              let peer = env["IRIS_IOS_HARNESS_CHAT_ID"],
              peer.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil,
              env["IRIS_IOS_HARNESS_DISABLE_RELAYS"] == "0" else {
            throw HarnessError.unexpected("Audio continuity requires explicit isolated fixture inputs")
        }
    }

    func runCallAudioContinuity(manager: AppManager, dataDir: URL, env: [String: String]) async throws {
        try validateAudioContinuityFixture(env)
        guard let runID = env["IRIS_IOS_HARNESS_RUN_ID"], let peer = env["IRIS_IOS_HARNESS_CHAT_ID"],
              dataDir.lastPathComponent == "harness-" + runID,
              dataDir.standardizedFileURL == dataDir.resolvingSymlinksInPath(),
              FileManager.default.fileExists(atPath: dataDir.appendingPathComponent("account-secret.json").path),
              let account = manager.state.account,
              manager.state.chatList.contains(where: { $0.chatId == peer && !$0.isRequest }) else {
            throw HarnessError.unexpected("Prepare an accepted isolated echo-peer chat before the audio probe")
        }
        let owner = account.publicKeyHex
        let probe = IrisCallAudioContinuityProbe.shared
        try probe.begin(environment: env, duration: 90)
        defer { manager.calls.end() }
        manager.calls.start(chatID: peer, video: false)
        var output: [String: Any] = [:]
        defer {
            output["probe"] = probe.finish()
            if let data = try? JSONSerialization.data(withJSONObject: output, options: [.sortedKeys]),
               let value = String(data: data, encoding: .utf8) { status("audio_continuity", value) }
        }
        _ = try await waitFor(label: "isolated voice echo connection", timeout: 40) {
            manager.state.call?.phase == "connected" && manager.state.call?.videoCapable == false ? true : nil
        }
        let begin = DispatchTime.now().uptimeNanoseconds
        output["connectedMeasurementStartUptimeNs"] = begin
        output["referenceKind"] = "external-varying-voiced"
        output["referenceSHA256"] = env["IRIS_IOS_HARNESS_AUDIO_REFERENCE_SHA256"]
        output["referencePlaybackIndependentReceiptRequired"] = true
        output["transport"] = "one-phone-real-ffi-fips-echo-peer"
        output["continuityQualified"] = false
        status("audio_reference_ready", "1")
        // Host starts the retained 30-second varying voiced acoustic reference
        // on this signal. No generated PCM is injected into capture or receive.
        for _ in 0..<160 {
            try await Task.sleep(nanoseconds: 250_000_000)
            guard manager.state.account?.publicKeyHex == owner, manager.state.call?.phase == "connected",
                  manager.state.call?.videoCapable == false, manager.state.call?.muted == false else {
                throw HarnessError.unexpected("Isolated voice call or account changed during observation")
            }
        }
        output["connectedMeasurementEndUptimeNs"] = DispatchTime.now().uptimeNanoseconds
        output["completed"] = true
        manager.calls.end()
        _ = try await waitFor(label: "isolated voice echo ended", timeout: 10) {
            manager.state.call == nil || manager.state.call?.phase == "ended" ? true : nil
        }
    }
}
#endif
