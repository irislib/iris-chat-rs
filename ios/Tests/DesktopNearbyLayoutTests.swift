#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class DesktopNearbyLayoutTests: XCTestCase {
    func testPeerUpdatesKeepNearbyAndFollowingChatInPlace() throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        state.preferences.nearbyEnabled = true
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: MockRustApp(state: state), secretStore: InMemorySecretStore(),
                                 pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
                                 desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        manager.nearbyIris.setFipsLanVisible(true)
        let sink = NearbyLayoutSink()
        let host = NSHostingView(rootView: NearbyLayoutFixture(manager: manager, service: manager.nearbyIris, sink: sink))
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 320, height: 180),
                              styleMask: .titled, backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        var baseline: [String: NearbyLayoutSample]?
        for (step, count) in [0, 1, 8, 0].enumerated() {
            let peers = (0..<count).map { index in
                IrisNearbyPeer(id: "peer-\(index)", name: "A long nearby display name \(index)",
                               ownerPubkeyHex: nil, pictureURL: nil, profileEventID: nil, bluetoothRSSI: nil)
            }
            manager.nearbyIris.applyScreenshotFixturePeers(peers: peers, bluetoothPeerIDs: [], lanPeerIDs: peers.map(\.id))
            let deadline = Date().addingTimeInterval(2)
            repeat {
                host.layoutSubtreeIfNeeded()
                RunLoop.main.run(until: Date().addingTimeInterval(0.01))
            } while (sink.samples.count != 2 || sink.samples.values.contains { $0.peerCount != count }) && Date() < deadline
            XCTAssertEqual(sink.samples.count, 2)
            XCTAssertTrue(sink.samples.values.allSatisfy { $0.peerCount == count }, "Wait for the updated production row layout")
            if let baseline {
                for key in ["nearby", "following"] {
                    let before = try XCTUnwrap(baseline[key]?.frame)
                    let after = try XCTUnwrap(sink.samples[key]?.frame)
                    XCTAssertEqual(after.minY, before.minY, accuracy: 0.5, "\(key), \(count) peers")
                    XCTAssertEqual(after.height, before.height, accuracy: 0.5, "\(key), \(count) peers")
                    XCTAssertEqual(after.width, before.width, accuracy: 0.5)
                }
            } else {
                XCTAssertGreaterThan(try XCTUnwrap(sink.samples["nearby"]?.frame.height), 0)
                baseline = sink.samples
            }
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let attachment = XCTAttachment(data: try XCTUnwrap(bitmap.representation(using: .png, properties: [:])), uniformTypeIdentifier: "public.png")
            attachment.name = "nearby-layout-\(step)-\(count)-peers"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }
}

private struct NearbyLayoutSample: Equatable {
    let peerCount: Int
    let frame: CGRect
}

private struct NearbyLayoutPreference: PreferenceKey {
    static var defaultValue: [String: NearbyLayoutSample] = [:]
    static func reduce(value: inout [String: NearbyLayoutSample], nextValue: () -> [String: NearbyLayoutSample]) {
        value.merge(nextValue(), uniquingKeysWith: { _, new in new })
    }
}

private final class NearbyLayoutSink {
    var samples: [String: NearbyLayoutSample] = [:]
}

private struct NearbyLayoutFixture: View {
    let manager: AppManager
    @ObservedObject var service: IrisNearbyService
    let sink: NearbyLayoutSink

    var body: some View {
        VStack(spacing: 0) {
            DesktopNearbyIrisRow(manager: manager, service: service, onOpen: {}, onOpenPeerProfile: { _ in })
                .background(probe("nearby"))
            Text("Following chat").frame(maxWidth: .infinity).frame(height: 60)
                .background(probe("following"))
        }
        .coordinateSpace(name: "nearby-list")
        .onPreferenceChange(NearbyLayoutPreference.self) { sink.samples = $0 }
        .frame(width: 320, height: 180, alignment: .top)
        .environment(\.irisPalette, .dark)
        .preferredColorScheme(.dark)
    }

    private func probe(_ key: String) -> some View {
        GeometryReader { geometry in
            Color.clear.preference(key: NearbyLayoutPreference.self,
                                   value: [key: NearbyLayoutSample(peerCount: service.peers.count, frame: geometry.frame(in: .named("nearby-list")))])
        }
    }
}
#endif
