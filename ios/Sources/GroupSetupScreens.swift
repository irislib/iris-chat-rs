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

struct NewGroupScreen: View {
    private enum Step: Equatable {
        case members
        case details
    }

    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager

    @State private var step: Step = .members
    @State private var name = ""
    @State private var memberInput = ""
    @State private var selectedOwners = Set<String>()
    @State private var showingGroupPicturePicker = false
    @State private var showingGroupPictureSourceMenu = false
    @State private var groupPhoto: StagedAttachment?
    @State private var groupPictureTask: Task<Void, Never>?
    #if os(iOS)
    @State private var showingGroupPictureCamera = false
    #endif
    #if canImport(PhotosUI)
    @State private var showingGroupPicturePhotoPicker = false
    @State private var pickedGroupPicturePhotos: [PhotosPickerItem] = []
    #endif
    @FocusState private var isNameFocused: Bool

    private var normalizedMemberInput: String {
        normalizePeerInput(input: memberInput)
    }

    private var localOwnerHex: String? {
        manager.state.account?.publicKeyHex
    }

    private var existingDirectChats: [ChatThreadSnapshot] {
        manager.state.chatList.filter { chat in
            chat.kind == .direct && chat.chatId != localOwnerHex
        }
    }

    private var filteredKnownChats: [ChatThreadSnapshot] {
        existingDirectChats.filteredByQuery(memberInput)
    }

    private var canCreate: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
        !manager.state.busy.creatingGroup && groupPictureTask == nil
    }

    private func ownerPresentation(for owner: String) -> OwnerPresentation {
        if let chat = existingDirectChats.first(where: { sameOwner(owner, hex: $0.chatId, npub: $0.subtitle) }) {
            let primary = primaryDisplayName(displayName: chat.displayName, fallback: normalizePeerInput(input: owner))
            return OwnerPresentation(
                primary: primary,
                secondary: secondaryDisplayName(chat.subtitle, primary: primary)
            )
        }

        if let account = manager.state.account, sameOwner(owner, hex: account.publicKeyHex, npub: account.npub) {
            let primary = primaryDisplayName(displayName: account.displayName, fallback: account.npub)
            return OwnerPresentation(primary: primary, secondary: nil)
        }

        let normalized = normalizePeerInput(input: owner)
        return OwnerPresentation(primary: fallbackProfileNameForIdentity(normalized), secondary: nil)
    }

    var body: some View {
        IrisScrollScreen {
            Color.clear
                .frame(height: 0)
                .accessibilityIdentifier("newGroupPrimaryCard")

            if step == .members {
                memberSelectionStep
            } else {
                groupDetailsStep
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            isNameFocused = false
        }
        .irisInteractiveKeyboardDismiss()
        .fileImporter(
            isPresented: $showingGroupPicturePicker,
            allowedContentTypes: [.image],
            allowsMultipleSelection: false
        ) { result in
            guard case let .success(urls) = result, let url = urls.first else {
                return
            }
            selectGroupPicture { url }
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
                selectGroupPicture { url }
            }
            .ignoresSafeArea()
        }
        #endif
        .onDisappear {
            groupPictureTask?.cancel()
            groupPictureTask = nil
        }
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
        .irisOnChange(of: memberInput) { _ in
            addMemberInputIfReady()
        }
        .irisOnChange(of: step) { nextStep in
            if nextStep == .details {
                DispatchQueue.main.async {
                    isNameFocused = true
                }
            }
        }
    }

    private var memberSelectionStep: some View {
        Group {
            IrisSectionCard(accent: true) {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("newGroupMemberStep")

                CardHeader(title: "Select members")

                selectedMembersChips

                TextField("Search or paste user ID", text: $memberInput)
                    .irisIdentifierInputModifiers()
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .accessibilityIdentifier("newGroupMemberInput")
            }

            Button(selectedOwners.isEmpty ? "Next" : "Next (\(selectedOwners.count))") {
                step = .details
            }
            .buttonStyle(IrisPrimaryButtonStyle())
            .accessibilityIdentifier("newGroupNextButton")

            if !filteredKnownChats.isEmpty {
                knownUsersCard
            }
        }
    }

    private var groupDetailsStep: some View {
        Group {
            IrisSectionCard(accent: true) {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("newGroupDetailsStep")

                CardHeader(title: "Group details")

                HStack(spacing: 12) {
                    IrisAvatar(label: name.isEmpty ? "Group" : name, size: 56, emphasize: true)

                    VStack(alignment: .leading, spacing: 8) {
                        Button(groupPictureTask != nil ? "Adding photo…" : (groupPhoto == nil ? "Photo" : "Change photo")) {
                            presentGroupPictureSource()
                        }
                        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                        .accessibilityIdentifier("newGroupPhotoButton")

                        if let groupPhoto {
                            HStack(spacing: 8) {
                                Text(groupPhoto.filename)
                                    .font(.system(.footnote, design: .rounded))
                                    .foregroundStyle(palette.muted)
                                    .lineLimit(1)

                                Button("Remove") {
                                    groupPictureTask?.cancel()
                                    groupPictureTask = nil
                                    self.groupPhoto = nil
                                }
                                .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                                .accessibilityIdentifier("newGroupRemovePhotoButton")
                            }
                        }
                    }
                }

                TextField("Group name", text: $name)
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .focused($isNameFocused)
                    .submitLabel(.done)
                    .onSubmit { isNameFocused = false }
                    .accessibilityIdentifier("newGroupNameInput")

                selectedMembersChips
            }

            HStack(spacing: 10) {
                Button("Back") {
                    step = .members
                }
                .buttonStyle(IrisSecondaryButtonStyle())

                Button(manager.state.busy.creatingGroup ? "Creating…" : "Create group") {
                    manager.createGroup(
                        name: name,
                        memberInputs: selectedOwners.sorted(),
                        picture: groupPhoto
                    )
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(!canCreate)
                .accessibilityIdentifier("newGroupCreateButton")
            }
        }
    }

    private var knownUsersCard: some View {
        IrisSectionCard {
            CardHeader(title: memberInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Known users" : "Search results")

            ForEach(Array(filteredKnownChats.enumerated()), id: \.element.chatId) { index, chat in
                Button {
                    if selectedOwners.contains(chat.chatId) {
                        selectedOwners.remove(chat.chatId)
                    } else {
                        selectedOwners.insert(chat.chatId)
                    }
                    memberInput = ""
                } label: {
                    HStack(spacing: 12) {
                        IrisAvatar(socialConnection: chat.socialConnection, ownerPubkeyHex: chat.chatId, label: chat.displayName, size: 38, emphasize: selectedOwners.contains(chat.chatId), manager: manager)
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
                        Image(systemName: selectedOwners.contains(chat.chatId) ? "checkmark.circle.fill" : "circle")
                            .foregroundStyle(selectedOwners.contains(chat.chatId) ? palette.textPrimary : palette.muted)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.irisPlain)

                if index < filteredKnownChats.count - 1 {
                    Divider().overlay(palette.border)
                }
            }
        }
    }

    private var selectedMembersChips: some View {
        Group {
            if !selectedOwners.isEmpty {
                FlowWrap(spacing: 8, lineSpacing: 8) {
                    ForEach(selectedOwners.sorted(), id: \.self) { owner in
                        let presentation = ownerPresentation(for: owner)
                        SelectedMemberChip(
                            title: presentation.primary,
                            subtitle: presentation.secondary,
                            onRemove: { selectedOwners.remove(owner) }
                        )
                    }
                }
            }
        }
    }

    private func addMemberInputIfReady() {
        let normalized = normalizedMemberInput
        guard !normalized.isEmpty, isValidPeerInput(input: normalized) else {
            return
        }
        guard normalized != localOwnerHex else {
            memberInput = ""
            return
        }
        selectedOwners.insert(normalized)
        memberInput = ""
    }

    private func presentGroupPictureSource() {
        isNameFocused = false
        #if os(iOS)
        showingGroupPictureSourceMenu = true
        #else
        showingGroupPicturePicker = true
        #endif
    }

    private func selectGroupPicture(loadFile: @escaping () async -> URL?) {
        groupPictureTask?.cancel()
        groupPictureTask = Task {
            defer { if !Task.isCancelled { groupPictureTask = nil } }
            guard let staged = await manager.stageGroupPicture(loadFile: loadFile) else { return }
            guard !Task.isCancelled else {
                await manager.discardOutgoingAttachments([staged])
                return
            }
            groupPhoto = staged
        }
    }

    #if canImport(PhotosUI)
    private func handlePickedGroupPicturePhotos(_ items: [PhotosPickerItem]) {
        guard let item = items.first else { return }
        pickedGroupPicturePhotos = []
        selectGroupPicture {
            await loadPickedPhotoItem(item, directoryName: "iris-group-picks")
        }
    }
    #endif
}

let groupDetailsMemberCandidateLimit = 8
