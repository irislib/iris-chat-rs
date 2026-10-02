import Foundation
@testable import FipsBle
import XCTest

final class AppleBlePeripheralConnectionsTests: XCTestCase {
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
