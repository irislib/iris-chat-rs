import Foundation

/// Several L2CAP channels can share one CoreBluetooth peripheral connection.
/// Discovery keeps that connection too, so a replacement channel can open
/// without racing an asynchronous peripheral cancellation.
struct AppleBlePeripheralConnections {
    // CoreBluetooth callbacks carry no attempt ID. Keep an expired peripheral
    // unavailable until its terminal disconnect/failure callback, even if the
    // retry backoff has elapsed or scanning has stopped and started again.
    private var retiring: Set<UUID> = []
    private var cancelling: Set<UUID> = []
    private var outgoing: [UInt64: UUID] = [:]

    mutating func insert(_ id: UInt64, peer: UUID) { outgoing[id] = peer }
    @discardableResult
    mutating func remove(_ id: UInt64) -> Bool { outgoing.removeValue(forKey: id) != nil }
    mutating func reset() {
        outgoing.removeAll()
        retiring.removeAll()
        cancelling.removeAll()
    }
    mutating func retire(_ peer: UUID) { retiring.insert(peer) }
    func canConnect(_ peer: UUID) -> Bool { !retiring.contains(peer) }
    func isCancelling(_ peer: UUID) -> Bool { cancelling.contains(peer) }
    mutating func beginCancellation(_ peer: UUID, connecting: Bool) -> Bool {
        guard retiring.contains(peer), !connecting, !contains(peer) else { return false }
        return cancelling.insert(peer).inserted
    }
    mutating func didDisconnect(_ peer: UUID) {
        // Stream close callbacks can arrive after the central's terminal callback
        // and even after a replacement has opened. Those IDs no longer own the
        // peripheral and must not initiate another cancellation.
        outgoing = outgoing.filter { $0.value != peer }
        retiring.remove(peer)
        cancelling.remove(peer)
    }
    func contains(_ peer: UUID) -> Bool { outgoing.values.contains(peer) }

    func canDisconnect(_ peer: UUID, scanning: Bool, connecting: Bool) -> Bool {
        !scanning && !connecting && !contains(peer)
    }
}
