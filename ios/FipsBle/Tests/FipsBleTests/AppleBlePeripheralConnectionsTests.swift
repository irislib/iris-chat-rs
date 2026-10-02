import Foundation
@testable import FipsBle
import XCTest

final class AppleBlePeripheralConnectionsTests: XCTestCase {
    func testExpiredDiscoveryCannotRetryUntilCancellationIsAcknowledged() {
        let peer = UUID()
        var discovery = AppleBleBootstrapDiscovery()
        var connections = AppleBlePeripheralConnections()
        XCTAssertTrue(discovery.begin(peer, now: 10))
        XCTAssertEqual(discovery.expirePending(now: 25), [peer])
        connections.retire(peer)
        XCTAssertTrue(connections.beginCancellation(peer, connecting: false))
        // Even beyond the backoff, B must not start while callbacks from A can
        // still arrive. New L2CAP requests use this same admission boundary.
        XCTAssertFalse(connections.canConnect(peer))
        XCTAssertTrue(discovery.dueRetries(now: 40, allowing: connections.canConnect).isEmpty)
        XCTAssertFalse(discovery.begin(peer, canRead: connections.canConnect(peer), now: 40))
        XCTAssertFalse(discovery.complete(peer))
        discovery.fail(peer, now: 40)
        XCTAssertFalse(connections.beginCancellation(peer, connecting: false))
        discovery.reset() // A scan restart must not reopen the cancelled attempt either.
        XCTAssertFalse(discovery.begin(peer, canRead: connections.canConnect(peer), now: 100))
        // The central's terminal callback ends A before the queued timer starts B.
        connections.didDisconnect(peer)
        XCTAssertTrue(connections.canConnect(peer))
        XCTAssertEqual(discovery.dueRetries(now: 100, allowing: connections.canConnect), [peer])
        XCTAssertTrue(discovery.begin(peer, canRead: connections.canConnect(peer), now: 100))
        XCTAssertTrue(discovery.complete(peer))
    }

    func testExpiredSharedReadWaitsForOpenAndChannelBeforeCancelling() {
        let peer = UUID()
        var connections = AppleBlePeripheralConnections()
        connections.retire(peer)
        XCTAssertFalse(connections.beginCancellation(peer, connecting: true))
        connections.insert(1, peer: peer)
        XCTAssertFalse(connections.beginCancellation(peer, connecting: false))
        XCTAssertFalse(connections.canConnect(peer))
        connections.remove(1)
        XCTAssertTrue(connections.beginCancellation(peer, connecting: false))
        XCTAssertTrue(connections.isCancelling(peer))
        connections.didDisconnect(peer)
        XCTAssertFalse(connections.isCancelling(peer))
        XCTAssertTrue(connections.canConnect(peer))
    }

    func testClosingOldChannelKeepsReplacementOnTheSamePeripheral() {
        let peer = UUID()
        var connections = AppleBlePeripheralConnections()
        connections.insert(1, peer: peer)
        connections.insert(2, peer: peer)
        connections.remove(1)
        XCTAssertFalse(connections.canDisconnect(peer, scanning: false, connecting: false))
        connections.remove(2)
        XCTAssertTrue(connections.canDisconnect(peer, scanning: false, connecting: false))
    }

    func testClosingLastChannelKeepsDiscoveryAndQueuedOpensAlive() {
        let peer = UUID()
        var connections = AppleBlePeripheralConnections()
        connections.insert(1, peer: peer)
        connections.remove(1)
        XCTAssertFalse(connections.canDisconnect(peer, scanning: true, connecting: false))
        XCTAssertFalse(connections.canDisconnect(peer, scanning: false, connecting: true))
        // Stop scanning releases the connection once no open is in flight.
        XCTAssertTrue(connections.canDisconnect(peer, scanning: false, connecting: false))
    }
}
