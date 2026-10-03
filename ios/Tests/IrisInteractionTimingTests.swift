import XCTest
import CoreGraphics
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class IrisInteractionTimingTests: XCTestCase {
    private struct Message: IrisInteractionMessage {
        let id: String
        var body = "hello"
        var isOutgoing = true
    }

    @MainActor
    func testOpenRequiresMatchingStateAndVisibleReadyTarget() {
        var now = 10.0
        var records: [IrisInteractionTiming.Record] = []
        let timing = IrisInteractionTiming(clock: { now }, emit: { records.append($0) })
        timing.beginOpen(chatID: "chat", targetID: "target")
        let messages = [Message(id: "old"), Message(id: "target")]
        timing.stateAvailable(chatID: "other", messages: messages, historyLoaded: true)
        timing.stateAvailable(chatID: "chat", messages: [Message(id: "old")], historyLoaded: true)
        XCTAssertTrue(records.isEmpty)
        now = 10.05
        timing.stateAvailable(chatID: "chat", messages: messages, historyLoaded: false)
        XCTAssertEqual(records.map(\.stage), [.state])
        let visible = CGRect(x: 0, y: 30, width: 100, height: 20)
        timing.layout(chatID: "other", frames: ["target": visible], viewportMinY: 0, viewportMaxY: 100, ready: true, messageCount: 2)
        timing.layout(chatID: "chat", frames: ["target": visible], viewportMinY: 0, viewportMaxY: 100, ready: false, messageCount: 2)
        timing.layout(chatID: "chat", frames: ["target": visible], viewportMinY: 50, viewportMaxY: 100, ready: true, messageCount: 2)
        timing.layout(chatID: "chat", frames: ["old": visible], viewportMinY: 0, viewportMaxY: 100, ready: true, messageCount: 2)
        XCTAssertEqual(records.count, 1)
        now = 10.1
        timing.layout(chatID: "chat", frames: ["target": visible], viewportMinY: 0, viewportMaxY: 100, ready: true, messageCount: 2)
        timing.layout(chatID: "chat", frames: ["target": visible], viewportMinY: 0, viewportMaxY: 100, ready: true, messageCount: 2)
        XCTAssertEqual(records.map(\.stage), [.state, .visibleLayout])
        XCTAssertEqual(records[0].durationMilliseconds, 50, accuracy: 0.001)
        XCTAssertEqual(records[1].durationMilliseconds, 100, accuracy: 0.001)
    }

    @MainActor
    func testSendOnlyMatchesNewOutgoingMessageAndDistinctQueuedCopies() {
        var records: [IrisInteractionTiming.Record] = []
        let timing = IrisInteractionTiming(emit: { records.append($0) })
        let old = Message(id: "old")
        timing.beginSend(chatID: "chat", body: "hello", messages: [old])
        timing.beginSend(chatID: "chat", body: "hello", messages: [old])
        var messages = [old, Message(id: "incoming", isOutgoing: false), Message(id: "different", body: "other")]
        timing.stateAvailable(chatID: "chat", messages: messages, historyLoaded: true)
        XCTAssertTrue(records.isEmpty)
        messages.append(Message(id: "first"))
        timing.stateAvailable(chatID: "chat", messages: messages, historyLoaded: true)
        XCTAssertEqual(records.count, 1)
        let visible = CGRect(x: 0, y: 10, width: 100, height: 20)
        timing.layout(chatID: "chat", frames: ["old": visible], viewportMinY: 0, viewportMaxY: 100, ready: true, messageCount: 4)
        XCTAssertEqual(records.count, 1)
        timing.layout(chatID: "chat", frames: ["first": visible], viewportMinY: 0, viewportMaxY: 100, ready: true, messageCount: 4)
        timing.stateAvailable(chatID: "chat", messages: messages, historyLoaded: true)
        XCTAssertEqual(records.count, 2, "completed first send must not satisfy the queued second send")
        messages.append(Message(id: "second"))
        timing.stateAvailable(chatID: "chat", messages: messages, historyLoaded: true)
        timing.layout(chatID: "chat", frames: ["second": visible], viewportMinY: 0, viewportMaxY: 100, ready: true, messageCount: 5)
        XCTAssertEqual(records.map(\.action), [.send, .send, .send, .send])
        XCTAssertEqual(records.map(\.stage), [.state, .visibleLayout, .state, .visibleLayout])
    }

    @MainActor
    func testEmptyOpenWaitsForConfirmedHistoryAndValidViewport() {
        var records: [IrisInteractionTiming.Record] = []
        let timing = IrisInteractionTiming(emit: { records.append($0) })
        timing.beginOpen(chatID: "chat", targetID: nil)
        timing.stateAvailable(chatID: "chat", messages: [Message](), historyLoaded: false)
        timing.layout(chatID: "chat", frames: [:], viewportMinY: 0, viewportMaxY: 100, ready: true, messageCount: 0)
        XCTAssertTrue(records.isEmpty)
        timing.stateAvailable(chatID: "chat", messages: [Message](), historyLoaded: true)
        timing.layout(chatID: "chat", frames: [:], viewportMinY: 0, viewportMaxY: 0, ready: true, messageCount: 0)
        XCTAssertEqual(records.map(\.stage), [.state])
        timing.layout(chatID: "chat", frames: [:], viewportMinY: 0, viewportMaxY: 100, ready: true, messageCount: 0)
        XCTAssertEqual(records.map(\.stage), [.state, .visibleLayout])
    }

    @MainActor
    func testTracingRequiresExplicitOptIn() {
        XCTAssertNil(IrisInteractionTiming.configured(environment: [:], enabledInBundle: false))
        XCTAssertNil(IrisInteractionTiming.configured(environment: ["IRIS_PERF_LOG": "true"], enabledInBundle: false))
        XCTAssertNotNil(IrisInteractionTiming.configured(environment: ["IRIS_PERF_LOG": "1"], enabledInBundle: false))
        XCTAssertNotNil(IrisInteractionTiming.configured(environment: [:], enabledInBundle: true))
    }
}
