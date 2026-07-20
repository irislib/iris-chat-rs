import Combine
import Foundation

struct IrisNearbyPeer: Identifiable, Equatable {
    let id: String
    var name: String
    var ownerPubkeyHex: String?
    var pictureURL: String?
    var profileEventID: String?
    var bluetoothRSSI: Int?
    var lastSeen: Date
}

/// Published UI state for FIPS-owned Nearby transports. This class performs no communication.
final class IrisNearbyService: ObservableObject {
    @Published private(set) var isVisible = false
    @Published private(set) var isLanVisible = false
    @Published private(set) var status = "Off"
    @Published private(set) var lanStatus = "Off"
    @Published private(set) var peers: [IrisNearbyPeer] = []

    private var screenshotFixtureBluetoothPeerIDs: Set<String>?
    private var screenshotFixtureLanPeerIDs: Set<String>?

    var sidebarSubtitle: String {
        guard isNearbyActive else { return "Tap to enable" }
        guard !peers.isEmpty else { return "No users nearby" }
        let names = peers.map { peer -> String in
            let name = peer.name.trimmingCharacters(in: .whitespacesAndNewlines)
            return name.isEmpty ? "Someone" : name
        }
        switch names.count {
        case 1: return "\(names[0]) nearby"
        case 2: return "\(names[0]) and \(names[1]) nearby"
        case 3: return "\(names[0]), \(names[1]) and \(names[2]) nearby"
        default: return "\(names.prefix(3).joined(separator: ", ")) and \(names.count - 3) others nearby"
        }
    }

    var bluetoothTransportWarning: String? { nil }
    var lanTransportWarning: String? { nil }
    var shouldRestartLanAfterFailure: Bool { false }
    var isNearbyActive: Bool { isVisible || isLanVisible }
    var bluetoothPeers: [IrisNearbyPeer] {
        guard let ids = screenshotFixtureBluetoothPeerIDs else { return [] }
        return peers.filter { ids.contains($0.id) }
    }
    var lanPeers: [IrisNearbyPeer] {
        guard let ids = screenshotFixtureLanPeerIDs else { return [] }
        return peers.filter { ids.contains($0.id) }
    }
    var mailbagSummary: String? { nil }

    func setFipsBluetoothVisible(_ visible: Bool) {
        isVisible = visible
        status = visible ? "Visible" : "Off"
    }

    func setFipsLanVisible(_ visible: Bool) {
        isLanVisible = visible
        lanStatus = visible ? "Visible" : "Off"
    }

    func applyFipsPeerSnapshot(
        _ snapshot: DesktopNearbySnapshot,
        bluetoothPeerIds: [String],
        lanPeerIds: [String]
    ) {
        peers = snapshot.peers.map { peer in
            IrisNearbyPeer(
                id: peer.id,
                name: peer.name,
                ownerPubkeyHex: peer.ownerPubkeyHex,
                pictureURL: peer.pictureUrl,
                profileEventID: peer.profileEventId,
                bluetoothRSSI: nil,
                lastSeen: Date(timeIntervalSince1970: TimeInterval(peer.lastSeenSecs))
            )
        }
        screenshotFixtureBluetoothPeerIDs = Set(bluetoothPeerIds)
        screenshotFixtureLanPeerIDs = Set(lanPeerIds)
    }

    func applyScreenshotFixturePeers(
        peers: [IrisNearbyPeer],
        bluetoothPeerIDs: [String],
        lanPeerIDs: [String]
    ) {
        self.peers = peers
        screenshotFixtureBluetoothPeerIDs = Set(bluetoothPeerIDs)
        screenshotFixtureLanPeerIDs = Set(lanPeerIDs)
        if !bluetoothPeerIDs.isEmpty { setFipsBluetoothVisible(true) }
        if !lanPeerIDs.isEmpty { setFipsLanVisible(true) }
    }
}
