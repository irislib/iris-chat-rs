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
struct ShareTargetSheet: View {
    @ObservedObject var manager: AppManager
    let share: PendingShare
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.irisPalette) private var palette
    @State private var searchText = ""
    @State private var selectedChatIds = Set<String>()

    private var filteredChats: [ChatThreadSnapshot] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else {
            return manager.state.chatList
        }
        return manager.state.chatList.filter { chat in
            chat.displayName.lowercased().contains(query)
                || (chat.subtitle?.lowercased().contains(query) ?? false)
                || (chat.lastMessagePreview?.lowercased().contains(query) ?? false)
        }
    }

    private var filteredNearbyPeers: [IrisNearbyPeer] {
        let peers = sortedNearbyPeers(
            manager.nearbyIris.peers.filter { $0.ownerPubkeyHex != nil },
            manager: manager,
            bluetoothPeerIDs: Set(manager.nearbyIris.bluetoothPeers.map(\.id)),
            lanPeerIDs: Set(manager.nearbyIris.lanPeers.map(\.id))
        )
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else {
            return peers
        }
        return peers.filter { peer in
            nearbyPeerResolvedName(peer, manager: manager).lowercased().contains(query)
                || (peer.ownerPubkeyHex?.lowercased().contains(query) ?? false)
        }
    }

    private var hasShareTargets: Bool {
        !manager.state.chatList.isEmpty || !manager.nearbyIris.peers.filter { $0.ownerPubkeyHex != nil }.isEmpty
    }

    private var selectedChats: [ChatThreadSnapshot] {
        manager.state.chatList.filter { selectedChatIds.contains($0.chatId) }
    }

    private var selectedNamesText: String {
        let chatNames = selectedChats.map(\.displayName)
        let nearbyNames = manager.nearbyIris.peers.compactMap { peer -> String? in
            guard let owner = peer.ownerPubkeyHex, selectedChatIds.contains(owner) else {
                return nil
            }
            return nearbyPeerDisplayName(peer, manager: manager)
        }
        return (chatNames + nearbyNames).joined(separator: ", ")
    }

    var body: some View {
        NavigationStack {
            Group {
                if !hasShareTargets {
                    emptyState
                } else {
                    chatList
                }
            }
            .navigationTitle(share.isForwarding ? "Forward" : "Choose recipients")
#if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search")
#elseif os(macOS)
            .searchable(text: $searchText, prompt: "Search")
#endif
#if os(iOS)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if hasShareTargets {
                    approvalFooter
                }
            }
#endif
            .toolbar {
#if os(iOS)
                ToolbarItem(placement: .confirmationAction) {
                    IrisModalCloseButton(accessibilityIdentifier: "shareTargetCloseButton") {
                        manager.clearPendingShare()
                        dismiss()
                    }
                }
#elseif os(macOS)
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        manager.clearPendingShare()
                        dismiss()
                    }
                }
                if hasShareTargets {
                    ToolbarItem(placement: .confirmationAction) {
                        Button {
                            sendSelectedAndDismiss()
                        } label: {
                            Text(sendButtonTitle)
                        }
                        .disabled(selectedChatIds.isEmpty)
                    }
                }
#endif
            }
            .background(palette.background)
            .onAppear(perform: preselectSuggestedChat)
            .irisOnChange(of: share.id) { _ in
                selectedChatIds.removeAll()
                preselectSuggestedChat()
            }
        }
    }

    private var emptyState: some View {
        VStack(spacing: 18) {
            Text("Start a chat first")
                .font(.headline)
                .foregroundStyle(palette.textPrimary)
            Button("New chat") {
                manager.clearPendingShare()
                manager.dispatch(.pushScreen(screen: .newChat))
                dismiss()
            }
            .buttonStyle(.borderedProminent)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(palette.background)
    }

    private var chatList: some View {
        List {
            if filteredChats.isEmpty && filteredNearbyPeers.isEmpty {
                Text("No matches")
                    .font(.system(size: 17, weight: .regular))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity, minHeight: 180, alignment: .center)
                    .listRowInsets(EdgeInsets())
                    .listRowBackground(palette.background)
                    .listRowSeparator(.hidden)
            } else {
                if !filteredNearbyPeers.isEmpty {
                    Section {
                        ForEach(filteredNearbyPeers) { peer in
                            shareNearbyTargetRow(peer)
                        }
                    } header: {
                        Text("Nearby")
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(palette.muted)
                            .textCase(nil)
                    }
                }

                if !filteredChats.isEmpty {
                    Section {
                        ForEach(filteredChats, id: \.chatId) { chat in
                            shareTargetRow(chat)
                        }
                    } header: {
                        Text(searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Recent chats" : "Chats")
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(palette.muted)
                            .textCase(nil)
                    }
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(palette.background)
    }

    private var sendButtonTitle: String {
        selectedChatIds.count > 1 ? "Send (\(selectedChatIds.count))" : "Send"
    }

#if os(iOS)
    private var approvalFooter: some View {
        VStack(spacing: 0) {
            Rectangle()
                .fill(palette.border)
                .frame(height: 0.5)

            HStack(spacing: 12) {
                ScrollView(.horizontal, showsIndicators: false) {
                    Text(selectedNamesText.isEmpty ? " " : selectedNamesText)
                        .font(.system(size: 15, weight: .regular))
                        .foregroundStyle(selectedChatIds.isEmpty ? palette.muted : palette.textPrimary)
                        .lineLimit(1)
                        .fixedSize(horizontal: true, vertical: false)
                        .padding(.horizontal, 2)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityHidden(selectedChatIds.isEmpty)

                Button {
                    sendSelectedAndDismiss()
                } label: {
                    Image(systemName: "arrow.up")
                        .font(.system(size: 21, weight: .bold))
                        .foregroundStyle(selectedChatIds.isEmpty ? palette.textPrimary.opacity(0.42) : Color.white)
                        .frame(width: 48, height: 48)
                        .background(
                            Circle()
                                .fill(selectedChatIds.isEmpty ? palette.muted.opacity(0.24) : palette.action)
                        )
                        .contentShape(Circle())
                }
                .buttonStyle(.irisPlain)
                .disabled(selectedChatIds.isEmpty)
                .accessibilityLabel(sendButtonTitle)
                .accessibilityIdentifier("shareTargetSendButton")
            }
            .padding(.horizontal, 16)
            .padding(.top, 9)
            .padding(.bottom, 9)
        }
        .background(shareFooterBackground)
    }

    private var shareFooterBackground: Color {
        if colorScheme == .dark {
            return Color(.sRGB, red: 27.0 / 255.0, green: 27.0 / 255.0, blue: 27.0 / 255.0, opacity: 1)
        }
        return Color(.sRGB, red: 246.0 / 255.0, green: 246.0 / 255.0, blue: 246.0 / 255.0, opacity: 1)
    }
#endif

    private func sendSelectedAndDismiss() {
        manager.sendPendingShare(
            to: Array(selectedChatIds).sorted()
        )
        dismiss()
    }

    private func shareTargetRow(_ chat: ChatThreadSnapshot) -> some View {
        let selected = selectedChatIds.contains(chat.chatId)
        return Button {
            if selected {
                selectedChatIds.remove(chat.chatId)
            } else {
                selectedChatIds.insert(chat.chatId)
            }
        } label: {
            HStack(spacing: 12) {
                IrisAvatar(
                    label: chat.displayName,
                    size: 40,
                    emphasize: false,
                    pictureUrl: chat.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager
                )
                VStack(alignment: .leading, spacing: 2) {
                    Text(chat.displayName)
                        .font(.system(size: 17, weight: .regular))
                        .foregroundStyle(palette.textPrimary)
                        .lineLimit(1)
                    if let subtitle = chat.subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.system(size: 13, weight: .regular))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Spacer()
                ShareTargetSelectionBadge(isSelected: selected)
            }
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .listRowInsets(EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16))
        .listRowBackground(palette.panel)
        .listRowSeparator(.hidden)
        .accessibilityLabel("\(chat.displayName), \(selected ? "selected" : "not selected")")
        .accessibilityValue(selected ? "Selected" : "Not selected")
    }

    private func shareNearbyTargetRow(_ peer: IrisNearbyPeer) -> some View {
        guard let owner = peer.ownerPubkeyHex else {
            return AnyView(EmptyView())
        }
        let selected = selectedChatIds.contains(owner)
        let name = nearbyPeerResolvedName(peer, manager: manager, fallback: "Nearby user")
        return AnyView(
            Button {
                if selected {
                    selectedChatIds.remove(owner)
                } else {
                    selectedChatIds.insert(owner)
                }
            } label: {
                HStack(spacing: 12) {
                    IrisAvatar(
                        label: name,
                        size: 40,
                        emphasize: false,
                        pictureUrl: peer.pictureURL,
                        preferences: manager.state.preferences,
                        manager: manager
                    )
                    VStack(alignment: .leading, spacing: 2) {
                        Text(nearbyPeerDisplayName(name))
                            .font(.system(size: 17, weight: .regular))
                            .foregroundStyle(palette.textPrimary)
                            .lineLimit(1)
                        Text("Nearby")
                            .font(.system(size: 13, weight: .regular))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Spacer()
                    ShareTargetSelectionBadge(isSelected: selected)
                }
                .padding(.vertical, 7)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .listRowInsets(EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16))
            .listRowBackground(palette.panel)
            .listRowSeparator(.hidden)
            .accessibilityLabel("\(name), \(selected ? "selected" : "not selected")")
            .accessibilityValue(selected ? "Selected" : "Not selected")
        )
    }

    private func preselectSuggestedChat() {
        guard selectedChatIds.isEmpty else {
            return
        }
        let suggested = Set(share.suggestedTargetChatIds)
        let available = manager.state.chatList
            .map(\.chatId)
            .filter { suggested.contains($0) }
        selectedChatIds.formUnion(available)
    }
}

struct ShareTargetSelectionBadge: View {
    @Environment(\.irisPalette) private var palette
    let isSelected: Bool

    var body: some View {
        Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
            .font(.system(size: 24, weight: .semibold))
            .symbolRenderingMode(.monochrome)
            .foregroundStyle(isSelected ? palette.action : palette.muted.opacity(0.55))
            .frame(width: 32, height: 40, alignment: .trailing)
            .accessibilityHidden(true)
    }
}
#endif
