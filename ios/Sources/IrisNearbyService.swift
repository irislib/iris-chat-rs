import Combine
import Foundation

struct IrisNearbyPeer: Identifiable, Equatable {
    let id: String
    var name: String
    var ownerPubkeyHex: String?
    var pictureURL: String?
    var profileEventID: String?
    var bluetoothRSSI: Int?
}

/// Published UI state for FIPS-owned Nearby transports. This class performs no communication.
final class IrisNearbyService: ObservableObject {
    @Published private(set) var isVisible = false
    @Published private(set) var isLanVisible = false
    @Published private(set) var peers: [IrisNearbyPeer] = []

    private var bluetoothPeerIDs: Set<String> = []
    private var lanPeerIDs: Set<String> = []

    var status: String { isVisible ? "Visible" : "Off" }
    var lanStatus: String { isLanVisible ? "Visible" : "Off" }

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
        peers.filter { bluetoothPeerIDs.contains($0.id) }
    }
    var lanPeers: [IrisNearbyPeer] {
        peers.filter { lanPeerIDs.contains($0.id) }
    }
    var mailbagSummary: String? { nil }

    func setFipsBluetoothVisible(_ visible: Bool) {
        guard isVisible != visible else { return }
        isVisible = visible
    }

    func setFipsLanVisible(_ visible: Bool) {
        guard isLanVisible != visible else { return }
        isLanVisible = visible
    }

    func applyFipsPeerSnapshot(
        _ snapshot: DesktopNearbySnapshot,
        bluetoothPeerIds: [String],
        lanPeerIds: [String]
    ) {
        let peers = snapshot.peers.map { peer in
            IrisNearbyPeer(
                id: peer.id,
                name: peer.name,
                ownerPubkeyHex: peer.ownerPubkeyHex,
                pictureURL: peer.pictureUrl,
                profileEventID: peer.profileEventId,
                bluetoothRSSI: nil
            )
        }
        applyPeers(peers, bluetoothPeerIDs: bluetoothPeerIds, lanPeerIDs: lanPeerIds)
    }

    func applyScreenshotFixturePeers(
        peers: [IrisNearbyPeer],
        bluetoothPeerIDs: [String],
        lanPeerIDs: [String]
    ) {
        applyPeers(peers, bluetoothPeerIDs: bluetoothPeerIDs, lanPeerIDs: lanPeerIDs)
        if !bluetoothPeerIDs.isEmpty { setFipsBluetoothVisible(true) }
        if !lanPeerIDs.isEmpty { setFipsLanVisible(true) }
    }

    private func applyPeers(
        _ peers: [IrisNearbyPeer],
        bluetoothPeerIDs: [String],
        lanPeerIDs: [String]
    ) {
        let bluetooth = Set(bluetoothPeerIDs)
        let lan = Set(lanPeerIDs)
        guard self.peers != peers || self.bluetoothPeerIDs != bluetooth || self.lanPeerIDs != lan else { return }
        // Transport changes also affect the peer UI; make them available to subscribers first.
        self.bluetoothPeerIDs = bluetooth
        self.lanPeerIDs = lan
        self.peers = peers
    }
}
