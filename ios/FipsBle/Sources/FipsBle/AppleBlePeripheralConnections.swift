import Foundation

/// Several L2CAP channels can share one CoreBluetooth peripheral connection.
/// Discovery keeps that connection too, so a replacement channel can open
/// without racing an asynchronous peripheral cancellation.
struct AppleBlePeripheralConnections {
    private var outgoing: [UInt64: UUID] = [:]

    mutating func insert(_ id: UInt64, peer: UUID) { outgoing[id] = peer }
    mutating func remove(_ id: UInt64) { outgoing.removeValue(forKey: id) }
    mutating func reset() { outgoing.removeAll() }
    func contains(_ peer: UUID) -> Bool { outgoing.values.contains(peer) }

    func canDisconnect(_ peer: UUID, scanning: Bool, connecting: Bool) -> Bool {
        !scanning && !connecting && !contains(peer)
    }
}
