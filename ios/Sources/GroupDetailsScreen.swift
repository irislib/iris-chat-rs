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

struct GroupDetailsScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let groupId: String

    @State private var groupName = ""
    @State private var groupAbout = ""
    @State private var memberInput = ""
    @State private var selectedAddMemberOwners = Set<String>()
    @State private var addMemberSuggestionsVisible = true
    @State private var showingScanner = false
    @State private var showingGroupPicturePicker = false
    @State private var showingGroupPictureSourceMenu = false
    @State private var groupPictureViewerItem: IrisProfilePictureViewerItem?
    #if os(iOS)
    @State private var showingGroupPictureCamera = false
    #endif
    #if canImport(PhotosUI)
    @State private var showingGroupPicturePhotoPicker = false
    @State private var pickedGroupPicturePhotos: [PhotosPickerItem] = []
    #endif

    private var normalizedMemberInput: String {
        normalizePeerInput(input: memberInput)
    }

    private var addMemberInputBinding: Binding<String> {
        Binding(
            get: { memberInput },
            set: { value in
                memberInput = value
                addMemberSuggestionsVisible = true
            }
        )
    }

    var body: some View {
        IrisScrollScreen {
            Color.clear
                .frame(height: 0)
                .accessibilityIdentifier("groupDetailsScreen")

            if let details = manager.state.groupDetails {
                IrisSectionCard(accent: true) {
                    CardHeader(
                        title: "Group settings",
                        subtitle: "Created by \(details.createdByDisplayName). Revision \(details.revision)."
                    )

                    HStack(spacing: 14) {
                        groupAvatar(details)
                        if details.canManage {
                            Button(manager.state.busy.uploadingAttachment ? "Uploading…" : "Change photo") {
                                presentGroupPictureSource()
                            }
                            .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                            .disabled(manager.state.busy.uploadingAttachment)
                            .accessibilityIdentifier("groupDetailsChangePhotoButton")
                        }
                    }

                    TextField("Name", text: Binding(
                        get: { groupName.isEmpty ? details.name : groupName },
                        set: { groupName = $0 }
                    ))
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .accessibilityIdentifier("groupDetailsNameInput")

                    if details.canManage {
                        Button(manager.state.busy.updatingGroup ? "Renaming…" : "Rename") {
                            let nextName = groupName.trimmingCharacters(in: .whitespacesAndNewlines)
                            manager.dispatch(.updateGroupName(groupId: groupId, name: nextName.isEmpty ? details.name : nextName))
                        }
                        .buttonStyle(IrisPrimaryButtonStyle())
                        .disabled(manager.state.busy.updatingGroup)
                        .accessibilityIdentifier("groupDetailsRenameButton")
                    }

                    if details.canManage {
                        TextField("Add a description", text: Binding(
                            get: { groupAbout.isEmpty ? (details.about ?? "") : groupAbout },
                            set: { groupAbout = $0 }
                        ), axis: .vertical)
                        .lineLimit(2...5)
                        .textFieldStyle(.plain)
                        .irisInputField()
                        .accessibilityIdentifier("groupDetailsAboutInput")

                        Button("Save description") {
                            let trimmed = groupAbout.trimmingCharacters(in: .whitespacesAndNewlines)
                            manager.dispatch(.updateGroupAbout(
                                groupId: groupId,
                                about: trimmed.isEmpty ? nil : trimmed
                            ))
                        }
                        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                        .disabled(manager.state.busy.updatingGroup)
                        .accessibilityIdentifier("groupDetailsAboutSaveButton")
                    } else if let about = details.about, !about.isEmpty {
                        Text(about)
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.textPrimary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Disappearing messages",
                        subtitle: "Messages auto-delete after the chosen interval."
                    )
                    let chatId = "group:\(groupId)"
                    let currentTtl = manager.state.currentChat?.chatId == chatId
                        ? manager.state.currentChat?.messageTtlSeconds
                        : nil
                    VStack(spacing: 0) {
                        ForEach(disappearingMessageOptions, id: \.0) { label, ttlSeconds in
                            Button {
                                manager.dispatch(.setChatMessageTtl(chatId: chatId, ttlSeconds: ttlSeconds))
                            } label: {
                                HStack {
                                    Text(label)
                                        .foregroundStyle(palette.textPrimary)
                                    Spacer()
                                    if currentTtl == ttlSeconds {
                                        Image(systemName: "checkmark")
                                            .font(.system(size: 14, weight: .semibold))
                                            .foregroundStyle(palette.textPrimary)
                                    }
                                }
                                .padding(.vertical, 10)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.irisPlain)
                        }
                    }
                }

                IrisSectionCard {
                    Button {
                        manager.dispatch(.setChatMuted(chatId: "group:\(groupId)", muted: !details.isMuted))
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: details.isMuted ? "bell.fill" : "bell.slash.fill")
                            Text(details.isMuted ? "Unmute chat" : "Mute chat")
                            Spacer()
                        }
                        .foregroundStyle(palette.textPrimary)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.irisPlain)
                    .accessibilityIdentifier("groupDetailsMuteButton")
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Members",
                        subtitle: "\(details.members.count) people in this conversation."
                    )

                    ForEach(Array(details.members.enumerated()), id: \.element.ownerPubkeyHex) { index, member in
                        let primary = primaryDisplayName(displayName: member.displayName, fallback: member.npub)
                        VStack(alignment: .leading, spacing: 10) {
                            let memberHeader = HStack(alignment: .top, spacing: 12) {
                                IrisAvatar(
                                    label: primary,
                                    size: 38,
                                    emphasize: member.isLocalOwner,
                                    pictureUrl: member.pictureUrl,
                                    preferences: manager.state.preferences,
                                    manager: manager
                                )

                                VStack(alignment: .leading, spacing: 6) {
                                    Text(primary)
                                        .font(.system(.headline, design: .rounded, weight: .semibold))
                                        .foregroundStyle(palette.textPrimary)
                                    HStack(spacing: 6) {
                                        if member.isLocalOwner {
                                            IrisInfoPill("You")
                                        }
                                        if member.isCreator {
                                            IrisInfoPill("Creator")
                                        } else if member.isAdmin {
                                            IrisInfoPill("Admin")
                                        }
                                    }
                                }

                                Spacer()
                            }
                            if member.isLocalOwner {
                                memberHeader
                            } else {
                                Button {
                                    manager.dispatch(.createChat(peerInput: member.ownerPubkeyHex))
                                } label: {
                                    memberHeader
                                        .contentShape(Rectangle())
                                }
                                .buttonStyle(.irisPlain)
                                .accessibilityIdentifier("groupDetailsOpenMember-\(String(member.ownerPubkeyHex.prefix(12)))")
                            }

                            if details.canManage && !member.isLocalOwner {
                                ViewThatFits(in: .horizontal) {
                                    HStack(spacing: 8) {
                                        memberAdminButton(member)
                                        removeMemberButton(member)
                                    }
                                    VStack(spacing: 8) {
                                        memberAdminButton(member)
                                        removeMemberButton(member)
                                    }
                                }
                            }
                        }

                        if index < details.members.count - 1 {
                            Divider().overlay(palette.border)
                        }
                    }
                }

                if details.canManage {
                    IrisSectionCard {
                        CardHeader(
                            title: "Add members",
                            subtitle: "Search, paste, or scan a user ID."
                        )

                        TextField("Search or paste user ID", text: addMemberInputBinding)
                            .irisIdentifierInputModifiers()
                            .textFieldStyle(.plain)
                            .irisInputField()
                            .accessibilityIdentifier("groupDetailsAddMemberInput")

                        selectedAddMemberChips(details: details)

                        VStack(spacing: 10) {
                            if irisSupportsQrScanning {
                                Button("Scan code") { showingScanner = true }
                                    .buttonStyle(IrisSecondaryButtonStyle())
                                    .accessibilityIdentifier("groupDetailsScanQrButton")
                            }

                            let pendingInputs = pendingAddMemberInputs(details: details)
                            Button(addMembersButtonTitle(inputCount: pendingInputs.count)) {
                                manager.dispatch(.addGroupMembers(groupId: groupId, memberInputs: pendingInputs))
                                selectedAddMemberOwners.removeAll()
                                memberInput = ""
                                addMemberSuggestionsVisible = false
                            }
                            .buttonStyle(IrisPrimaryButtonStyle())
                            .disabled(pendingInputs.isEmpty || manager.state.busy.updatingGroup)
                            .accessibilityIdentifier("groupDetailsAddMembersButton")
                        }
                    }

                    let candidateChats = knownUsersForAdding(details: details)
                    let visibleCandidateChats = Array(candidateChats.prefix(groupDetailsMemberCandidateLimit))
                    if addMemberSuggestionsVisible && !visibleCandidateChats.isEmpty {
                        IrisSectionCard {
                            HStack(alignment: .firstTextBaseline, spacing: 10) {
                                CardHeader(
                                    title: memberInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Known users" : "Search results"
                                )
                                Spacer()
                                Button {
                                    addMemberSuggestionsVisible = false
                                } label: {
                                    Image(systemName: "xmark.circle.fill")
                                        .font(.system(size: 18, weight: .semibold))
                                }
                                .buttonStyle(.irisPlain)
                                .foregroundStyle(palette.muted)
                                .accessibilityLabel("Close search results")
                                .accessibilityIdentifier("groupDetailsCloseMemberResultsButton")
                            }

                            ForEach(Array(visibleCandidateChats.enumerated()), id: \.element.chatId) { index, chat in
                                let selected = selectedAddMemberOwners.contains(chat.chatId)
                                Button {
                                    toggleSelectedAddMember(chat.chatId)
                                } label: {
                                    HStack(spacing: 12) {
                                        IrisAvatar(label: chat.displayName, size: 38, emphasize: selected)
                                        VStack(alignment: .leading, spacing: 4) {
                                            Text(chat.displayName)
                                                .font(.system(.headline, design: .rounded, weight: .semibold))
                                                .foregroundStyle(palette.textPrimary)
                                            if let subtitle = secondaryDisplayName(chat.subtitle, primary: chat.displayName) {
                                                Text(subtitle)
                                                    .font(.system(.footnote, design: .rounded))
                                                    .foregroundStyle(palette.muted)
                                            }
                                        }
                                        Spacer()
                                        Image(systemName: selected ? "checkmark.square.fill" : "square")
                                            .font(.system(size: 22, weight: .semibold))
                                            .foregroundStyle(selected ? palette.textPrimary : palette.muted)
                                    }
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.irisPlain)
                                .accessibilityIdentifier("groupDetailsKnownUser-\(String(chat.chatId.prefix(12)))")
                                .accessibilityValue(selected ? "Selected" : "Not selected")
                                .disabled(manager.state.busy.updatingGroup)

                                if index < visibleCandidateChats.count - 1 {
                                    Divider().overlay(palette.border)
                                }
                            }
                        }
                    }
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Delete chat",
                        subtitle: "Removes this group from your chat list and forgets local messages."
                    )
                    Button(role: .destructive) {
                        manager.dispatch(.deleteChat(chatId: "group:\(groupId)"))
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: "trash")
                            Text("Delete chat")
                        }
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("groupDetailsDeleteChatButton")
                }
            }
        }
        .irisProfilePictureViewer(
            item: $groupPictureViewerItem,
            preferences: manager.state.preferences,
            manager: manager
        )
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                memberInput = normalizePeerInput(input: code)
                addMemberSuggestionsVisible = true
                showingScanner = false
            }
            .irisModalSurface()
            .irisDismissOnMacOutsideClick { showingScanner = false }
        }
        .fileImporter(
            isPresented: $showingGroupPicturePicker,
            allowedContentTypes: [.image],
            allowsMultipleSelection: false
        ) { result in
            if case let .success(urls) = result, let url = urls.first {
                manager.updateGroupPicture(groupId: groupId, fileURL: url)
            }
        }
        .confirmationDialog(
            "Choose a group photo",
            isPresented: $showingGroupPictureSourceMenu,
            titleVisibility: .hidden
        ) {
            #if os(iOS)
            if UIImagePickerController.isSourceTypeAvailable(.camera) {
                Button("Take Photo") { showingGroupPictureCamera = true }
            }
            #endif
            #if canImport(PhotosUI)
            Button("Photo Library") { showingGroupPicturePhotoPicker = true }
            #endif
            Button("Files") { showingGroupPicturePicker = true }
            Button("Cancel", role: .cancel) {}
        }
        #if os(iOS)
        .sheet(isPresented: $showingGroupPictureCamera) {
            IrisCameraImagePicker { url in
                manager.updateGroupPicture(groupId: groupId, fileURL: url)
            }
            .ignoresSafeArea()
        }
        #endif
        #if canImport(PhotosUI)
        .photosPicker(
            isPresented: $showingGroupPicturePhotoPicker,
            selection: $pickedGroupPicturePhotos,
            maxSelectionCount: 1,
            matching: .images
        )
        .irisOnChange(of: pickedGroupPicturePhotos) { items in
            handlePickedGroupPicturePhotos(items)
        }
        #endif
    }

    private func presentGroupPictureSource() {
        #if canImport(PhotosUI)
        showingGroupPictureSourceMenu = true
        #else
        showingGroupPicturePicker = true
        #endif
    }

    #if canImport(PhotosUI)
    private func handlePickedGroupPicturePhotos(_ items: [PhotosPickerItem]) {
        guard let item = items.first else { return }
        pickedGroupPicturePhotos = []
        Task {
            guard let url = await loadPickedPhotoItem(item, directoryName: "iris-group-picks") else { return }
            await MainActor.run {
                manager.updateGroupPicture(groupId: groupId, fileURL: url)
            }
        }
    }
    #endif

    @ViewBuilder
    private func groupAvatar(_ details: GroupDetailsSnapshot) -> some View {
        if let item = IrisProfilePictureViewerItem(
            label: details.name,
            pictureUrl: details.pictureUrl,
            accessibilityIdentifier: "groupDetailsProfilePictureViewer"
        ) {
            Button {
                groupPictureViewerItem = item
            } label: {
                groupAvatarImage(details)
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel("Open group photo")
            .accessibilityIdentifier("groupDetailsProfilePictureButton")
        } else {
            groupAvatarImage(details)
        }
    }

    private func groupAvatarImage(_ details: GroupDetailsSnapshot) -> some View {
        IrisAvatar(
            label: details.name,
            size: 56,
            emphasize: true,
            pictureUrl: details.pictureUrl,
            preferences: manager.state.preferences,
            manager: manager,
            loadedImageIdentifier: "groupDetailsAvatarImage"
        )
    }

    private func knownUsersForAdding(details: GroupDetailsSnapshot) -> [ChatThreadSnapshot] {
        let localOwnerHex = manager.state.account?.publicKeyHex
        let memberHexes = Set(details.members.map { $0.ownerPubkeyHex })
        return manager.state.chatList
            .filter { chat in
                chat.kind == .direct
                    && chat.chatId != localOwnerHex
                    && !memberHexes.contains(chat.chatId)
            }
            .filteredByQuery(memberInput)
    }

    private func pendingAddMemberInputs(details: GroupDetailsSnapshot) -> [String] {
        let memberHexes = Set(details.members.map { $0.ownerPubkeyHex })
        let localOwnerHex = manager.state.account?.publicKeyHex
        var inputs = Set(
            selectedAddMemberOwners
                .filter { $0 != localOwnerHex && !memberHexes.contains($0) }
        )

        if isValidPeerInput(input: normalizedMemberInput),
           normalizedMemberInput != localOwnerHex,
           !memberHexes.contains(normalizedMemberInput)
        {
            inputs.insert(normalizedMemberInput)
        }

        return inputs.sorted()
    }

    private func addMembersButtonTitle(inputCount: Int) -> String {
        if manager.state.busy.updatingGroup {
            return "Adding…"
        }
        return inputCount > 1 ? "Add \(inputCount) members" : "Add member"
    }

    @ViewBuilder
    private func selectedAddMemberChips(details: GroupDetailsSnapshot) -> some View {
        let pendingOwners = selectedAddMemberOwners
            .filter { owner in details.members.allSatisfy { $0.ownerPubkeyHex != owner } }
            .sorted()

        if !pendingOwners.isEmpty {
            FlowWrap(spacing: 8, lineSpacing: 8) {
                ForEach(pendingOwners, id: \.self) { owner in
                    let presentation = addMemberPresentation(for: owner)
                    SelectedMemberChip(
                        title: presentation.primary,
                        subtitle: presentation.secondary,
                        onRemove: { selectedAddMemberOwners.remove(owner) }
                    )
                }
            }
        }
    }

    private func toggleSelectedAddMember(_ owner: String) {
        if selectedAddMemberOwners.contains(owner) {
            selectedAddMemberOwners.remove(owner)
        } else {
            selectedAddMemberOwners.insert(owner)
        }
    }

    private func addMemberPresentation(for owner: String) -> OwnerPresentation {
        if let chat = manager.state.chatList.first(where: { sameOwner(owner, hex: $0.chatId, npub: $0.subtitle) }) {
            let primary = primaryDisplayName(displayName: chat.displayName, fallback: normalizePeerInput(input: owner))
            return OwnerPresentation(
                primary: primary,
                secondary: secondaryDisplayName(chat.subtitle, primary: primary)
            )
        }

        let normalized = normalizePeerInput(input: owner)
        return OwnerPresentation(primary: fallbackProfileNameForIdentity(normalized), secondary: nil)
    }

    private func memberAdminButton(_ member: GroupMemberSnapshot) -> some View {
        Button(member.isAdmin ? "Dismiss admin" : "Make admin") {
            manager.setGroupAdmin(
                groupId: groupId,
                ownerPubkeyHex: member.ownerPubkeyHex,
                isAdmin: !member.isAdmin
            )
        }
        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
        .disabled(manager.state.busy.updatingGroup || member.isCreator)
        .accessibilityIdentifier("groupDetailsAdminMember-\(String(member.ownerPubkeyHex.prefix(12)))")
    }

    private func removeMemberButton(_ member: GroupMemberSnapshot) -> some View {
        Button("Remove", role: .destructive) {
            manager.dispatch(.removeGroupMember(groupId: groupId, ownerPubkeyHex: member.ownerPubkeyHex))
        }
        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
        .disabled(manager.state.busy.updatingGroup || member.isCreator)
        .accessibilityIdentifier("groupDetailsRemoveMember-\(String(member.ownerPubkeyHex.prefix(12)))")
    }
}
