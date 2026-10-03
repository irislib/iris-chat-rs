import Combine
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class IrisNearbyServiceTests: XCTestCase {
    func testRepeatedSnapshotsAndLastSeenChangesDoNotPublish() {
        let service = IrisNearbyService()
        var changes = 0
        var snapshots = 0
        let observation = service.objectWillChange.sink { changes += 1 }
        let peers = service.$peers.dropFirst().sink { _ in snapshots += 1 }
        defer { observation.cancel(); peers.cancel() }

        for second in 1...100 {
            apply([peer(lastSeen: UInt64(second))], to: service, lan: ["device"])
        }

        XCTAssertEqual(changes, 1)
        XCTAssertEqual(snapshots, 1)
        XCTAssertEqual(service.peers.map(\.name), ["Alice"])
        XCTAssertEqual(service.lanPeers.map(\.id), ["device"])
    }

    func testPeerDetailsArrivalAndDepartureStillPublish() {
        let service = IrisNearbyService()
        var changes = 0
        let observation = service.objectWillChange.sink { changes += 1 }
        defer { observation.cancel() }

        var current = peer()
        apply([current], to: service)
        let edits: [(inout DesktopNearbyPeerSnapshot) -> Void] = [
            { $0.name = "Alicia" },
            { $0.ownerPubkeyHex = "new-owner" },
            { $0.pictureUrl = "https://example.com/avatar.png" },
            { $0.profileEventId = "new-profile" },
            { $0.id = "new-device" },
        ]
        for (index, edit) in edits.enumerated() {
            edit(&current)
            apply([current], to: service)
            XCTAssertEqual(changes, index + 2)
            XCTAssertEqual(service.peers.first?.id, current.id)
            XCTAssertEqual(service.peers.first?.name, current.name)
            XCTAssertEqual(service.peers.first?.ownerPubkeyHex, current.ownerPubkeyHex)
            XCTAssertEqual(service.peers.first?.pictureURL, current.pictureUrl)
            XCTAssertEqual(service.peers.first?.profileEventID, current.profileEventId)
        }
        apply([current, peer()], to: service)
        XCTAssertEqual(changes, 7)
        XCTAssertEqual(service.peers.count, 2)
        apply([], to: service)
        XCTAssertEqual(changes, 8)
        XCTAssertTrue(service.peers.isEmpty)
        apply([], to: service)
        XCTAssertEqual(changes, 8)
    }

    func testTransportMembershipChangesPublishWithUpdatedClassification() {
        let service = IrisNearbyService()
        apply([peer()], to: service, bluetooth: ["device"])
        var changes = 0
        var bluetoothAtPublication: [String] = []
        var lanAtPublication: [String] = []
        let observation = service.objectWillChange.sink { changes += 1 }
        let peers = service.$peers.dropFirst().sink { _ in
            bluetoothAtPublication = service.bluetoothPeers.map(\.id)
            lanAtPublication = service.lanPeers.map(\.id)
        }
        defer { observation.cancel(); peers.cancel() }

        apply([peer()], to: service, lan: ["device"])
        XCTAssertEqual(changes, 1)
        XCTAssertEqual(bluetoothAtPublication, [])
        XCTAssertEqual(lanAtPublication, ["device"])
        apply([peer()], to: service, lan: ["device", "device"])
        XCTAssertEqual(changes, 1)
        apply([peer()], to: service, bluetooth: ["device"], lan: ["device"])
        XCTAssertEqual(changes, 2)
        XCTAssertEqual(service.bluetoothPeers.map(\.id), ["device"])
        XCTAssertEqual(service.lanPeers.map(\.id), ["device"])
    }

    func testVisibilityOnlyPublishesActualChanges() {
        let service = IrisNearbyService()
        var changes = 0
        var bluetooth: [Bool] = []
        var lan: [Bool] = []
        let observation = service.objectWillChange.sink { changes += 1 }
        let bluetoothObservation = service.$isVisible.dropFirst().sink { bluetooth.append($0) }
        let lanObservation = service.$isLanVisible.dropFirst().sink { lan.append($0) }
        defer { observation.cancel(); bluetoothObservation.cancel(); lanObservation.cancel() }

        for _ in 0..<100 {
            service.setFipsBluetoothVisible(false)
            service.setFipsLanVisible(false)
        }
        XCTAssertEqual(changes, 0)
        for _ in 0..<100 {
            service.setFipsBluetoothVisible(true)
            service.setFipsLanVisible(true)
        }
        XCTAssertEqual(changes, 2)
        XCTAssertEqual(service.status, "Visible")
        XCTAssertEqual(service.lanStatus, "Visible")
        XCTAssertTrue(service.isNearbyActive)
        service.setFipsBluetoothVisible(false)
        service.setFipsLanVisible(false)
        XCTAssertEqual(changes, 4)
        XCTAssertEqual(bluetooth, [true, false])
        XCTAssertEqual(lan, [true, false])
        XCTAssertEqual(service.status, "Off")
        XCTAssertEqual(service.lanStatus, "Off")
        XCTAssertFalse(service.isNearbyActive)
    }

    func testScreenshotFixtureSharesSnapshotDeduplication() {
        let service = IrisNearbyService()
        apply([peer()], to: service)
        let snapshot = service.peers
        var changes = 0
        let observation = service.objectWillChange.sink { changes += 1 }
        defer { observation.cancel() }

        service.applyScreenshotFixturePeers(peers: snapshot, bluetoothPeerIDs: [], lanPeerIDs: [])
        XCTAssertEqual(changes, 0)
        service.applyScreenshotFixturePeers(peers: snapshot, bluetoothPeerIDs: [], lanPeerIDs: ["device"])
        XCTAssertEqual(changes, 2)
        XCTAssertEqual(service.lanPeers.map(\.id), ["device"])
        XCTAssertTrue(service.isLanVisible)
        service.applyScreenshotFixturePeers(peers: snapshot, bluetoothPeerIDs: [], lanPeerIDs: ["device"])
        XCTAssertEqual(changes, 2)
    }

    private func peer(lastSeen: UInt64 = 1) -> DesktopNearbyPeerSnapshot {
        DesktopNearbyPeerSnapshot(
            id: "device", name: "Alice", ownerPubkeyHex: "alice",
            pictureUrl: nil, profileEventId: nil, lastSeenSecs: lastSeen
        )
    }

    private func apply(
        _ peers: [DesktopNearbyPeerSnapshot],
        to service: IrisNearbyService,
        bluetooth: [String] = [],
        lan: [String] = []
    ) {
        service.applyFipsPeerSnapshot(
            DesktopNearbySnapshot(visible: true, status: "Visible", peers: peers),
            bluetoothPeerIds: bluetooth, lanPeerIds: lan
        )
    }
}
