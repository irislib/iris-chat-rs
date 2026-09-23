import SwiftUI
import XCTest
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class CallHistoryTests: XCTestCase {
    func testCallOutcomesAndNegotiatedMediaHaveDistinctLabels() {
        let cases: [(String, String, Bool, String)] = [
            ("incoming", "missed", true, "Missed video call"),
            ("incoming", "answered", false, "Incoming voice call"),
            ("outgoing", "answered", true, "Outgoing video call"),
            ("outgoing", "canceled", false, "Canceled voice call"),
            ("incoming", "declined", true, "Declined video call"),
        ]
        for (direction, outcome, video, title) in cases {
            let view = CallHistoryPresentation(call: history(direction: direction, outcome: outcome, video: video))
            XCTAssertEqual(view.title, title)
            XCTAssertEqual(view.isMissed, outcome == "missed")
            XCTAssertEqual(view.icon, video ? "video.fill" : "phone.fill")
            XCTAssertEqual(view.directionIcon, direction == "outgoing" ? "arrow.up.right" : "arrow.down.left")
        }
    }

    func testOnlyAnsweredCallsShowDurationIncludingBriefAndLongCalls() {
        for outcome in ["missed", "declined", "canceled"] {
            XCTAssertNil(CallHistoryPresentation(call: history(outcome: outcome, duration: 20)).duration)
        }
        for (seconds, text) in [(UInt64(0), "0:00"), (65, "1:05"), (3_661, "1:01:01")] {
            XCTAssertEqual(CallHistoryPresentation(call: history(duration: seconds)).duration, text)
        }
    }

    func testCallHistorySeparatesAdjacentMessageClusters() throws {
        var message = try XCTUnwrap(buildLargeTestAppState(directChatCount: 1, groupChatCount: 0,
                                                          messagesInCurrentChat: 1).currentChat?.messages.first)
        message.kind = .user
        var call = message
        call.kind = .system
        call.call = history()
        XCTAssertTrue(irisStartsMessageCluster(previous: message, message: call, chatKind: .direct))
        XCTAssertTrue(irisStartsMessageCluster(previous: call, message: message, chatKind: .direct))
    }

#if os(macOS)
    @MainActor
    func testRenderProductionCallRows() throws {
        for dark in [false, true] {
            let palette: IrisPalette = dark ? .dark : .light
            let view = VStack(spacing: 0) {
                ChatCallHistoryRow(call: history(direction: "incoming", outcome: "missed", video: true))
                ChatCallHistoryRow(call: history(direction: "outgoing", duration: 65))
                ChatCallHistoryRow(call: history(direction: "incoming", video: true, duration: 3_661))
                ChatCallHistoryRow(call: history(direction: "incoming", outcome: "declined"))
                ChatCallHistoryRow(call: history(direction: "outgoing", outcome: "canceled", video: true))
            }
            .padding(.vertical, 16)
            .frame(width: 390)
            .background(palette.background)
            .environment(\.irisPalette, palette)
            .environment(\.colorScheme, dark ? .dark : .light)
            let renderer = ImageRenderer(content: view)
            renderer.scale = 2
            let rendered = try XCTUnwrap(renderer.cgImage)
            XCTAssertEqual(rendered.width, 780)
            guard let directory = ProcessInfo.processInfo.environment["IRIS_CALL_HISTORY_ARTIFACT_DIR"] else { continue }
            let png = try XCTUnwrap(NSBitmapImageRep(cgImage: rendered).representation(using: .png, properties: [:]))
            try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("call-history-\(dark ? "dark" : "light").png"))
        }
    }
#endif

    private func history(direction: String = "incoming", outcome: String = "answered", video: Bool = false,
                         duration: UInt64 = 0) -> CallHistorySnapshot {
        CallHistorySnapshot(callId: "history", direction: direction, outcome: outcome, video: video,
                            startedAtSecs: 1_790_157_600, answeredAtSecs: outcome == "answered" ? 1_790_157_604 : nil,
                            endedAtSecs: 1_790_157_604 + duration, durationSecs: duration)
    }
}
