#if os(iOS) || os(macOS)
import Foundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class IrisAudioDownloadTests: XCTestCase {
    @MainActor
    func testAutomaticDownloadsRespectNetworkPreferenceLowDataAndCalls() {
        typealias Network = IrisAudioDownloads.NetworkState
        let wifi = Network(connected: true)
        let mobile = Network(connected: true, expensive: true)
        let lowData = Network(connected: true, constrained: true)
        let offline = Network()
        for network in [wifi, mobile, lowData, offline] {
            XCTAssertFalse(IrisAudioDownloads.allowsPreview(preference: .never, network: network, callActive: false))
            XCTAssertFalse(IrisAudioDownloads.allowsPreview(preference: .wifiAndCellular, network: network, callActive: true))
        }
        XCTAssertTrue(IrisAudioDownloads.allowsPreview(preference: .wifi, network: wifi, callActive: false))
        XCTAssertFalse(IrisAudioDownloads.allowsPreview(preference: .wifi, network: mobile, callActive: false))
        XCTAssertTrue(IrisAudioDownloads.allowsPreview(preference: .wifiAndCellular, network: mobile, callActive: false))
        for network in [lowData, offline] {
            XCTAssertFalse(IrisAudioDownloads.allowsPreview(preference: .wifiAndCellular, network: network, callActive: false))
        }
    }

    func testWaveformsAreSerializedAndReusedAcrossConcurrentCells() async {
        let probe = WaveformProbe()
        let worker = IrisAudioWaveformWorker(sample: { _ in probe.sample() })
        await withTaskGroup(of: [Float].self) { group in
            for _ in 0..<12 { group.addTask { await worker.decode(URL(fileURLWithPath: "/unused"), cacheKey: "same-content") } }
            for await result in group { XCTAssertEqual(result, [0.25, 1]) }
        }
        XCTAssertEqual(probe.calls, 1)
        await withTaskGroup(of: [Float].self) { group in
            for i in 0..<8 { group.addTask { await worker.decode(URL(fileURLWithPath: "/unused"), cacheKey: "clip-\(i)") } }
            for await _ in group {}
        }
        XCTAssertEqual(probe.maximumConcurrent, 1)
    }

    func testWaveformCacheIsBoundedAndRemembersSkippedAnalysis() async {
        let probe = WaveformProbe()
        let worker = IrisAudioWaveformWorker(capacity: 2, sample: { _ in _ = probe.sample(); return [] })
        let url = URL(fileURLWithPath: "/unused")
        for key in ["a", "b", "a", "c", "a"] { _ = await worker.decode(url, cacheKey: key) }
        XCTAssertEqual(probe.calls, 3)
        _ = await worker.decode(url, cacheKey: "b")
        XCTAssertEqual(probe.calls, 4, "Only the least recently used waveform should be evicted")
    }
}

private final class WaveformProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var active = 0
    private(set) var calls = 0
    private(set) var maximumConcurrent = 0
    func sample() -> [Float] {
        lock.lock()
        calls += 1
        active += 1
        maximumConcurrent = max(active, maximumConcurrent)
        lock.unlock()
        Thread.sleep(forTimeInterval: 0.01)
        lock.lock()
        active -= 1
        lock.unlock()
        return [0.25, 1]
    }
}
#endif
