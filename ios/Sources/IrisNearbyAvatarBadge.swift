import SwiftUI

func irisNearbyAvatarOwners(
    peers: [IrisNearbyPeer],
    isActive: Bool,
    enabled: Bool,
    localOwner: String?
) -> Set<String> {
    guard enabled, isActive, let localOwner, !localOwner.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return [] }
    return Set(peers.compactMap { peer in
        guard let owner = peer.ownerPubkeyHex, !owner.isEmpty, owner != localOwner else { return nil }
        return owner
    })
}

struct IrisNearbyAvatarBadge: View {
    @ObservedObject private var manager: AppManager
    @ObservedObject private var service: IrisNearbyService
    let ownerPubkeyHex: String
    let size: CGFloat

    init(ownerPubkeyHex: String, manager: AppManager, size: CGFloat) {
        self.ownerPubkeyHex = ownerPubkeyHex
        self.manager = manager
        service = manager.nearbyIris
        self.size = size
    }

    var body: some View {
        if irisNearbyAvatarOwners(
            peers: service.peers,
            isActive: service.isNearbyActive,
            enabled: manager.state.preferences.nearbyEnabled,
            localOwner: manager.state.account?.publicKeyHex
        ).contains(ownerPubkeyHex) {
            IrisNearbyBadge(size: size)
        }
    }
}

struct IrisNearbyBadge: View {
    @Environment(\.irisPalette) private var palette
    let size: CGFloat

    var body: some View {
        Image(systemName: "dot.radiowaves.left.and.right")
            .font(.system(size: size * 0.6, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(palette.accent, in: Circle())
            .overlay(Circle().strokeBorder(palette.panel, lineWidth: 1.5))
            .accessibilityLabel("Nearby")
            .accessibilityIdentifier("nearbyAvatarBadge")
            .help("Nearby")
            .allowsHitTesting(false)
    }
}
