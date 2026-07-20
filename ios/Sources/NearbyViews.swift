import Foundation
import Combine
import SwiftUI
import UniformTypeIdentifiers
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif
#if canImport(PhotosUI)
import PhotosUI
#endif

#if os(iOS) || os(macOS)
let NearbyChatListRowHeight: CGFloat =
    IrisChatListRowMetrics.avatarSize + IrisChatListRowMetrics.verticalPadding * 2 + 18

func nearbyAccessibilityLabel(nearbyEnabled: Bool, hasPeers: Bool, active: Bool) -> String {
    guard nearbyEnabled else { return "Nearby, Off" }
    if hasPeers { return "Nearby" }
    return active ? "Nearby, No users nearby" : "Nearby, Off"
}

struct NearbyChatListRow: View {
    @ObservedObject var manager: AppManager
    @ObservedObject var service: IrisNearbyService
    let onOpen: () -> Void
    let onOpenPeerProfile: (String) -> Void
    @State private var cachedPeers: [IrisNearbyPeer] = []
    @State private var isTransitioningNearby = false
    @State private var transitionGeneration = 0
    @State private var observedNearbyEnabled: Bool?
    @State private var observedNearbyActive: Bool?

    private var nearbyEnabled: Bool {
        manager.state.preferences.nearbyEnabled
    }

    private var active: Bool {
#if os(iOS)
        nearbyEnabled &&
            (manager.state.preferences.nearbyBluetoothEnabled || service.isLanVisible)
#else
        nearbyEnabled && service.isNearbyActive
#endif
    }

    private var livePeers: [IrisNearbyPeer] {
        guard nearbyEnabled else { return [] }
        return sortedNearbyPeers(
            service.peers,
            manager: manager,
            bluetoothPeerIDs: Set(service.bluetoothPeers.map(\.id)),
            lanPeerIDs: Set(service.lanPeers.map(\.id))
        )
    }

    private var visiblePeers: [IrisNearbyPeer] {
        if !livePeers.isEmpty {
            return livePeers
        }
        if isTransitioningNearby, !cachedPeers.isEmpty {
            return cachedPeers
        }
        return []
    }

    var body: some View {
        Group {
            if visiblePeers.isEmpty {
                NearbyEmptyRow(
                    manager: manager,
                    active: active,
                    isTransitioning: isTransitioningNearby,
                    onOpen: onOpen
                )
            } else {
                NearbyPeerStripRow(
                    manager: manager,
                    peers: visiblePeers,
                    avatarSize: IrisChatListRowMetrics.avatarSize,
                    horizontalPadding: IrisChatListRowMetrics.horizontalPadding,
                    verticalPadding: IrisChatListRowMetrics.verticalPadding,
                    rowHeight: NearbyChatListRowHeight,
                    active: active,
                    onOpenNearby: onOpen,
                    onOpenPeerProfile: onOpenPeerProfile
                )
            }
        }
        .onAppear {
            cachePeersIfNeeded(service.peers)
        }
        .onReceive(service.$peers) { newPeers in
            cachePeersIfNeeded(newPeers)
            if !newPeers.isEmpty {
                endNearbyTransition()
            }
        }
        .onReceive(manager.$state.map { $0.preferences.nearbyEnabled }.removeDuplicates()) { enabled in
            guard let previous = observedNearbyEnabled else {
                observedNearbyEnabled = enabled
                return
            }
            observedNearbyEnabled = enabled
            if previous != enabled {
                beginNearbyTransition()
            }
        }
        .onReceive(service.$isVisible.combineLatest(service.$isLanVisible).map { $0 || $1 }.removeDuplicates()) { active in
            guard let previous = observedNearbyActive else {
                observedNearbyActive = active
                return
            }
            observedNearbyActive = active
            if previous != active {
                beginNearbyTransition()
            }
        }
    }

    private func cachePeersIfNeeded(_ peers: [IrisNearbyPeer]) {
        guard !peers.isEmpty else { return }
        cachedPeers = peers
    }

    private func beginNearbyTransition() {
        transitionGeneration += 1
        let generation = transitionGeneration
        isTransitioningNearby = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.45) {
            guard transitionGeneration == generation else { return }
            isTransitioningNearby = false
        }
    }

    private func endNearbyTransition() {
        transitionGeneration += 1
        isTransitioningNearby = false
    }
}

struct NearbyEmptyRow: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let active: Bool
    let isTransitioning: Bool
    let onOpen: () -> Void
    private var statusText: String {
        if isTransitioning, manager.state.preferences.nearbyEnabled {
            return "Starting"
        }
        if !manager.state.preferences.nearbyEnabled {
            return "Off"
        }
        return active ? "No users nearby" : "Off"
    }

    var body: some View {
        Button(action: onOpen) {
            HStack(spacing: IrisChatListRowMetrics.avatarTextSpacing) {
                NearbyWirelessAvatar(active: active)

                Text(statusText)
                    .font(.subheadline)
                    .foregroundStyle(palette.muted)
                    .lineLimit(1)

                Spacer(minLength: 0)
            }
        }
        .buttonStyle(.irisPlain)
        .padding(.horizontal, IrisChatListRowMetrics.horizontalPadding)
        .padding(.vertical, IrisChatListRowMetrics.verticalPadding)
        .frame(height: NearbyChatListRowHeight, alignment: .top)
        .simultaneousGesture(LongPressGesture(minimumDuration: 0.5).onEnded { _ in
            toggleNearbyMaster(manager: manager)
        })
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("nearbyChatRow")
        .accessibilityLabel(nearbyAccessibilityLabel(
            nearbyEnabled: manager.state.preferences.nearbyEnabled,
            hasPeers: !manager.nearbyIris.peers.isEmpty,
            active: active
        ))
    }
}

struct NearbyWirelessAvatar: View {
    @Environment(\.irisPalette) private var palette
    var size: CGFloat = IrisChatListRowMetrics.avatarSize
    var active: Bool = false

    var body: some View {
        ZStack {
            Circle().fill(active ? palette.action : palette.panelAlt)
            Circle().stroke(palette.border, lineWidth: 1)
            Image(systemName: "dot.radiowaves.left.and.right")
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(active ? palette.onAccent : palette.muted)
        }
        .frame(width: size, height: size)
    }
}

struct NearbyPeerStripRow: View {
    @ObservedObject var manager: AppManager
    let peers: [IrisNearbyPeer]
    let avatarSize: CGFloat
    let horizontalPadding: CGFloat
    let verticalPadding: CGFloat
    let rowHeight: CGFloat?
    let active: Bool
    let onOpenNearby: () -> Void
    let onOpenPeerProfile: (String) -> Void

    var body: some View {
        HStack(alignment: .top, spacing: IrisChatListRowMetrics.avatarTextSpacing) {
            Button(action: onOpenNearby) {
                NearbyWirelessAvatar(size: avatarSize, active: active)
            }
                .buttonStyle(.irisPlain)
                .simultaneousGesture(LongPressGesture(minimumDuration: 0.5).onEnded { _ in
                    toggleNearbyMaster(manager: manager)
                })
                .accessibilityElement(children: .ignore)
                .accessibilityAddTraits(.isButton)
                .accessibilityIdentifier("nearbyChatRow")
                .accessibilityLabel("Nearby")

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(alignment: .top, spacing: 10) {
                    ForEach(peers) { peer in
                        let name = nearbyPeerResolvedName(peer, manager: manager, fallback: "Nearby user")
                        Button {
                            openNearbyPeer(
                                peer,
                                manager: manager,
                                onOpenPeerProfile: onOpenPeerProfile
                            )
                        } label: {
                            VStack(spacing: 4) {
                                IrisAvatar(
                                    label: name,
                                    size: avatarSize,
                                    pictureUrl: peer.pictureURL,
                                    preferences: manager.state.preferences,
                                    manager: manager
                                )
                                Text(nearbyPeerDisplayName(name))
                                    .font(.caption2.weight(.medium))
                                    .foregroundStyle(Color.secondary)
                                    .lineLimit(1)
                                    .truncationMode(.tail)
                                    .frame(width: max(avatarSize + 12, 64), alignment: .center)
                            }
                        }
                        .buttonStyle(.irisPlain)
                        .simultaneousGesture(LongPressGesture(minimumDuration: 0.5).onEnded { _ in
                            openNearbyPeerProfile(peer, onOpenPeerProfile: onOpenPeerProfile)
                        })
                        .accessibilityElement(children: .ignore)
                        .accessibilityAddTraits(.isButton)
                        .accessibilityIdentifier("nearbyPreviewPeer-\(String(peer.id.prefix(12)))")
                        .accessibilityLabel(name)
                    }
                }
                .padding(.trailing, horizontalPadding)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.leading, horizontalPadding)
        .padding(.vertical, verticalPadding)
        .frame(height: rowHeight, alignment: .top)
        .contentShape(Rectangle())
    }
}

func nearbyPeerDisplayName(_ name: String) -> String {
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return "Nearby" }
    if trimmed.count <= 14 { return trimmed }
    return String(trimmed.prefix(13)) + "…"
}

@MainActor
func nearbyPeerDisplayName(_ peer: IrisNearbyPeer, manager: AppManager) -> String {
    nearbyPeerDisplayName(nearbyPeerResolvedName(peer, manager: manager))
}

@MainActor
func nearbyPeerResolvedName(
    _ peer: IrisNearbyPeer,
    manager: AppManager,
    fallback: String = "Nearby"
) -> String {
    if let owner = peer.ownerPubkeyHex,
       let chat = manager.state.chatList.first(where: { chat in
           chat.kind == .direct &&
               chat.chatId.caseInsensitiveCompare(owner) == .orderedSame
       }) {
        let name = chat.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        if !name.isEmpty { return name }
    }
    let name = peer.name.trimmingCharacters(in: .whitespacesAndNewlines)
    return name.isEmpty ? fallback : name
}

@MainActor
func sortedNearbyPeers(
    _ peers: [IrisNearbyPeer],
    manager: AppManager,
    bluetoothPeerIDs: Set<String>,
    lanPeerIDs: Set<String>
) -> [IrisNearbyPeer] {
    peers.sorted { left, right in
        compareNearbyPeers(
            left,
            right,
            manager: manager,
            bluetoothPeerIDs: bluetoothPeerIDs,
            lanPeerIDs: lanPeerIDs
        ) == .orderedAscending
    }
}

@MainActor
func compareNearbyPeers(
    _ left: IrisNearbyPeer,
    _ right: IrisNearbyPeer,
    manager: AppManager,
    bluetoothPeerIDs: Set<String>,
    lanPeerIDs: Set<String>
) -> ComparisonResult {
    let leftHasChat = left.ownerPubkeyHex.map { nearbyPeerHasKnownChat($0, manager: manager) } ?? false
    let rightHasChat = right.ownerPubkeyHex.map { nearbyPeerHasKnownChat($0, manager: manager) } ?? false
    if leftHasChat != rightHasChat {
        return leftHasChat ? .orderedAscending : .orderedDescending
    }

    let leftTransport = nearbyTransportRank(left.id, bluetoothPeerIDs: bluetoothPeerIDs, lanPeerIDs: lanPeerIDs)
    let rightTransport = nearbyTransportRank(right.id, bluetoothPeerIDs: bluetoothPeerIDs, lanPeerIDs: lanPeerIDs)
    if leftTransport != rightTransport {
        return leftTransport < rightTransport ? .orderedAscending : .orderedDescending
    }

    if bluetoothPeerIDs.contains(left.id), bluetoothPeerIDs.contains(right.id) {
        let leftRSSI = left.bluetoothRSSI ?? Int.min
        let rightRSSI = right.bluetoothRSSI ?? Int.min
        if leftRSSI != rightRSSI {
            return leftRSSI > rightRSSI ? .orderedAscending : .orderedDescending
        }
    }

    let leftKey = nearbyDeterministicPeerKey(left)
    let rightKey = nearbyDeterministicPeerKey(right)
    if leftKey != rightKey {
        return leftKey < rightKey ? .orderedAscending : .orderedDescending
    }
    return left.id < right.id ? .orderedAscending : .orderedDescending
}

func nearbyTransportRank(
    _ peerID: String,
    bluetoothPeerIDs: Set<String>,
    lanPeerIDs: Set<String>
) -> Int {
    if bluetoothPeerIDs.contains(peerID) { return 0 }
    if lanPeerIDs.contains(peerID) { return 1 }
    return 2
}

func nearbyDeterministicPeerKey(_ peer: IrisNearbyPeer) -> String {
    let owner = peer.ownerPubkeyHex?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
    return owner.isEmpty ? "peer:\(peer.id.lowercased())" : owner
}

@MainActor
func nearbyPeerHasKnownChat(_ ownerPubkeyHex: String, manager: AppManager) -> Bool {
    manager.state.chatList.contains { chat in
        chat.kind == .direct &&
            chat.chatId.caseInsensitiveCompare(ownerPubkeyHex) == .orderedSame
    }
}

@MainActor
func openNearbyPeer(
    _ peer: IrisNearbyPeer,
    manager: AppManager,
    onOpenPeerProfile: (String) -> Void
) {
    guard let ownerPubkeyHex = peer.ownerPubkeyHex else { return }
    if isKnownDirectNearbyPeer(ownerPubkeyHex, manager: manager) {
        manager.dispatch(.openChat(chatId: ownerPubkeyHex))
    } else {
        onOpenPeerProfile(ownerPubkeyHex)
    }
}

@MainActor
func toggleNearbyMaster(manager: AppManager) {
    irisNearbyLongPressFeedback()
    manager.setNearbyEnabled(!manager.state.preferences.nearbyEnabled)
}

func openNearbyPeerProfile(
    _ peer: IrisNearbyPeer,
    onOpenPeerProfile: (String) -> Void
) {
    guard let ownerPubkeyHex = peer.ownerPubkeyHex else { return }
    irisNearbyLongPressFeedback()
    onOpenPeerProfile(ownerPubkeyHex)
}

@MainActor
func isKnownDirectNearbyPeer(_ ownerPubkeyHex: String, manager: AppManager) -> Bool {
    manager.state.chatList.contains { chat in
        chat.kind == .direct &&
            chat.chatId.caseInsensitiveCompare(ownerPubkeyHex) == .orderedSame
    }
}

func irisNearbyLongPressFeedback() {
#if os(iOS)
    UIImpactFeedbackGenerator(style: .medium).impactOccurred()
#endif
}

struct NearbyIrisScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @ObservedObject var service: IrisNearbyService
    let openPeerProfile: (String) -> Void
    let onClose: () -> Void

    private var nearbyEnabled: Bool {
        manager.state.preferences.nearbyEnabled
    }

    var body: some View {
        VStack(spacing: 0) {
            header

            Rectangle()
                .fill(palette.border)
                .frame(height: 1)

            transportControls

            Spacer(minLength: 0)
        }
        .background(palette.background)
        .irisModalSurface()
    }

    private var header: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Nearby")
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .foregroundStyle(palette.textPrimary)
            }

            Spacer()

            IrisModalCloseButton(action: onClose)
                .accessibilityIdentifier("nearbyCloseButton")
        }
        .padding(.horizontal, 18)
        .frame(height: 58)
        .background(palette.toolbar)
    }

    private var transportControls: some View {
        VStack(spacing: 0) {
            masterRow

            Rectangle()
                .fill(palette.border)
                .frame(height: 1)
                .padding(.leading, 18)

            transportRow(
                title: "Bluetooth",
                subtitle: nearbyEnabled ? service.bluetoothTransportWarning : nil,
                peers: nearbyEnabled ? sortedBluetoothPeers : [],
                isOn: bluetoothBinding,
                isEnabled: nearbyEnabled,
                accessibilityID: "nearbyBluetoothSwitch"
            )

            Rectangle()
                .fill(palette.border)
                .frame(height: 1)
                .padding(.leading, 18)

            transportRow(
                title: "Wi-Fi",
                subtitle: nearbyEnabled ? service.lanTransportWarning : nil,
                peers: nearbyEnabled ? sortedLanPeers : [],
                isOn: lanBinding,
                isEnabled: nearbyEnabled,
                accessibilityID: "nearbyLanSwitch"
            )

            Rectangle()
                .fill(palette.border)
                .frame(height: 1)
                .padding(.leading, 18)

            mailbagRow
        }
        .background(palette.panel)
    }

    private var masterRow: some View {
        HStack(spacing: 12) {
            Text("Nearby")
                .font(.system(.body, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
            Spacer()
            Toggle("", isOn: Binding(
                get: { manager.state.preferences.nearbyEnabled },
                set: { manager.setNearbyEnabled($0) }
            ))
            .labelsHidden()
            .toggleStyle(.switch)
            .irisControlTint()
            .accessibilityIdentifier("nearbyEnabledSwitch")
        }
        .padding(.horizontal, 18)
        .frame(height: 52)
    }

    @ViewBuilder
    private var mailbagRow: some View {
        // Mirrors `transportRow` so Mailbag reads as a peer to
        // Bluetooth and Wi-Fi — same row chrome (title + optional
        // subtitle on the left, switch on the right, footer that
        // appears when the toggle is on), so the user understands
        // it's another transport-layer thing they can pause without
        // losing data.
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Mailbag")
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    if let summary = service.mailbagSummary {
                        Text(summary)
                            .font(.system(.caption, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }
                Spacer()
                Toggle("", isOn: Binding(
                    get: { manager.state.preferences.nearbyMailbagEnabled },
                    set: { enabled in
                        manager.dispatch(.setNearbyMailbagEnabled(enabled: enabled))
                    }
                ))
                .labelsHidden()
                .toggleStyle(.switch)
                .irisControlTint()
                .disabled(!nearbyEnabled)
                .accessibilityIdentifier("nearbyMailbagSwitch")
            }
            .frame(height: 52)

            if nearbyEnabled && manager.state.preferences.nearbyMailbagEnabled {
                Text("Anonymously carries messages by you and others over Bluetooth or Wi-Fi, so they keep moving where there's no internet.")
                    .font(.system(.caption, design: .rounded))
                    .foregroundStyle(palette.muted)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.top, 2)
            }
        }
        .padding(.horizontal, 18)
        .padding(.bottom, 14)
        .opacity(nearbyEnabled ? 1 : 0.48)
        .accessibilityIdentifier("nearbyMailbagSection")
    }

    private func transportRow(
        title: String,
        subtitle: String?,
        peers: [IrisNearbyPeer],
        isOn: Binding<Bool>,
        isEnabled: Bool,
        accessibilityID: String
    ) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    if let subtitle {
                        Text(subtitle)
                            .font(.system(.caption, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }
                Spacer()
                Toggle("", isOn: isOn)
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .irisControlTint()
                    .disabled(!isEnabled)
                    .accessibilityIdentifier(accessibilityID)
            }
            .frame(height: 52)

            if isEnabled && isOn.wrappedValue {
                if peers.isEmpty, subtitle == nil {
                    Text("No users nearby")
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.bottom, 12)
                } else if !peers.isEmpty {
                    peerStrip(peers)
                }
            }
        }
        .padding(.horizontal, 18)
        .opacity(isEnabled ? 1 : 0.48)
    }

    private var bluetoothBinding: Binding<Bool> {
        Binding(
            get: { manager.state.preferences.nearbyBluetoothEnabled },
            set: { manager.setNearbyBluetoothEnabled($0) }
        )
    }

    private var lanBinding: Binding<Bool> {
        Binding(
            get: { manager.state.preferences.nearbyLanEnabled },
            set: { manager.setNearbyLanEnabled($0) }
        )
    }

    private var sortedBluetoothPeers: [IrisNearbyPeer] {
        let peers = service.bluetoothPeers
        return sortedNearbyPeers(
            peers,
            manager: manager,
            bluetoothPeerIDs: Set(peers.map(\.id)),
            lanPeerIDs: []
        )
    }

    private var sortedLanPeers: [IrisNearbyPeer] {
        let peers = service.lanPeers
        return sortedNearbyPeers(
            peers,
            manager: manager,
            bluetoothPeerIDs: [],
            lanPeerIDs: Set(peers.map(\.id))
        )
    }

    @ViewBuilder
    private func peerStrip(_ peers: [IrisNearbyPeer]) -> some View {
        if !peers.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 12) {
                    ForEach(peers) { peer in
                        let name = nearbyPeerResolvedName(peer, manager: manager, fallback: "Nearby user")
                        Button {
                            openPeer(peer)
                        } label: {
                            VStack(spacing: 6) {
                                IrisAvatar(
                                    label: name,
                                    size: 42,
                                    pictureUrl: peer.pictureURL,
                                    preferences: manager.state.preferences,
                                    manager: manager
                                )
                                Text(name)
                                    .font(.system(.caption, design: .rounded, weight: .semibold))
                                    .foregroundStyle(palette.textPrimary)
                                    .lineLimit(1)
                                    .frame(maxWidth: 78)
                            }
                        }
                        .buttonStyle(.irisPlain)
                        .simultaneousGesture(LongPressGesture(minimumDuration: 0.5).onEnded { _ in
                            openNearbyPeerProfile(peer, onOpenPeerProfile: openPeerProfile)
                        })
                        .accessibilityElement(children: .ignore)
                        .accessibilityAddTraits(.isButton)
                        .accessibilityIdentifier("nearbyPeer-\(String(peer.id.prefix(12)))")
                        .accessibilityLabel(name)
                    }
                }
                .padding(.horizontal, 0)
                .padding(.vertical, 10)
            }
        }
    }

    private func openPeer(_ peer: IrisNearbyPeer) {
        guard let ownerPubkeyHex = peer.ownerPubkeyHex else { return }
        if isKnownDirectNearbyPeer(ownerPubkeyHex, manager: manager) {
            manager.dispatch(.openChat(chatId: ownerPubkeyHex))
            onClose()
        } else {
            openPeerProfile(ownerPubkeyHex)
        }
    }
}
#endif
