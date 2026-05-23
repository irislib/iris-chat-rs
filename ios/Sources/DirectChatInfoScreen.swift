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

struct IdentifiedString: Identifiable, Hashable {
    let value: String
    var id: String { value }
}

struct IrisProfilePictureViewerItem: Identifiable, Equatable {
    let label: String
    let pictureUrl: String
    let accessibilityIdentifier: String

    var id: String { "\(accessibilityIdentifier)|\(pictureUrl)" }

    init?(
        label: String,
        pictureUrl: String?,
        accessibilityIdentifier: String
    ) {
        let trimmed = pictureUrl?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard irisCanOpenProfilePicture(trimmed) else { return nil }
        self.label = label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Profile" : label
        self.pictureUrl = trimmed
        self.accessibilityIdentifier = accessibilityIdentifier
    }
}

struct DirectChatInfoScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let chatId: String
    let onClose: () -> Void
    var showMessageAction = false
    var onMessage: () -> Void = {}
    @State private var advancedExpanded = false
    @State private var profileDebug: PeerProfileDebugSnapshot?
    @State private var loadedProfileDebugFor: String?
    @State private var profilePictureViewerItem: IrisProfilePictureViewerItem?
    @State private var commonGroups: [ChatThreadSnapshot] = []
    @State private var commonGroupsLoadedFor: String?
    @State private var showingBlockConfirmation = false
    @State private var showingUnblockConfirmation = false
    @State private var showingReportConfirmation = false
    @State private var nicknameDraft = ""
    @State private var nicknameDraftLoadedKey: String?
    @State private var editingNickname = false

    private var chat: CurrentChatSnapshot? {
        manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if let chat {
                    VStack(spacing: 10) {
                        directChatAvatar(chat)
                        Text(chat.displayName)
                            .font(.system(.title2, design: .rounded, weight: .bold))
                            .foregroundStyle(palette.textPrimary)
                            .multilineTextAlignment(.center)
                        if let subtitle = chat.subtitle, !subtitle.isEmpty {
                            Text(subtitle)
                                .font(.system(.subheadline, design: .rounded))
                                .foregroundStyle(palette.muted)
                                .multilineTextAlignment(.center)
                        }
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.top, 22)

                    if let about = trimmedText(chat.about) {
                        profileAboutRow(about)
                    }

                    nicknameCard(chat)

                    if showMessageAction {
                        IrisSectionCard {
                            Button {
                                onMessage()
                            } label: {
                                HStack(spacing: 12) {
                                    Image(systemName: "bubble.left.and.bubble.right.fill")
                                        .frame(width: 24)
                                    Text("Message")
                                        .font(.system(.body, design: .rounded, weight: .semibold))
                                    Spacer(minLength: 0)
                                }
                                .foregroundStyle(palette.textPrimary)
                                .padding(.vertical, 2)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.irisPlain)
                            .accessibilityIdentifier("directChatMessageButton")
                        }
                    }

                    if !commonGroups.isEmpty {
                        IrisSectionCard {
                            Text("Groups in common")
                                .font(.system(.headline, design: .rounded, weight: .semibold))
                                .foregroundStyle(palette.textPrimary)

                            VStack(spacing: 0) {
                                ForEach(Array(commonGroups.enumerated()), id: \.element.chatId) { index, group in
                                    commonGroupRow(group)
                                    if index < commonGroups.count - 1 {
                                        Divider().overlay(palette.border)
                                            .padding(.leading, 50)
                                    }
                                }
                            }
                        }
                    }

                    IrisSectionCard {
                        Button {
                            manager.dispatch(.setChatMuted(chatId: chatId, muted: !chat.isMuted))
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: chat.isMuted ? "bell.fill" : "bell.slash.fill")
                                    .frame(width: 24)
                                Text(chat.isMuted ? "Unmute chat" : "Mute chat")
                                    .font(.system(.body, design: .rounded, weight: .semibold))
                                Spacer(minLength: 0)
                            }
                            .foregroundStyle(palette.textPrimary)
                            .padding(.vertical, 2)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("directChatMuteButton")

                        Divider().overlay(palette.border)

                        IrisCopyButton(
                            label: "Copy user ID",
                            value: peerInputToNpub(input: chatId),
                            style: .menuRow
                        )
                        .accessibilityIdentifier("directChatCopyUserIdButton")
                    }

                    IrisSectionCard {
                        CardHeader(
                            title: "Disappearing messages",
                            subtitle: nil
                        )
                        VStack(spacing: 0) {
                            ForEach(disappearingMessageOptions, id: \.0) { label, ttlSeconds in
                                Button {
                                    manager.dispatch(.setChatMessageTtl(chatId: chatId, ttlSeconds: ttlSeconds))
                                } label: {
                                    HStack {
                                        Text(label)
                                            .foregroundStyle(palette.textPrimary)
                                        Spacer()
                                        if chat.messageTtlSeconds == ttlSeconds {
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

                    DirectChatAdvancedCard(
                        debug: profileDebug,
                        isExpanded: $advancedExpanded
                    )
                    .accessibilityIdentifier("directChatAdvancedCard")
                    .onAppear(perform: loadProfileDebugIfNeeded)
                    .irisOnChange(of: advancedExpanded) { _ in
                        loadProfileDebugIfNeeded()
                    }

                    IrisSectionCard {
                        Button(role: manager.isUserBlocked(chatId) ? nil : .destructive) {
                            if manager.isUserBlocked(chatId) {
                                showingUnblockConfirmation = true
                            } else {
                                showingBlockConfirmation = true
                            }
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: manager.isUserBlocked(chatId) ? "checkmark.shield.fill" : "nosign")
                                    .frame(width: 24)
                                Text(manager.isUserBlocked(chatId) ? "Unblock user" : "Block user")
                                    .font(.system(.body, design: .rounded, weight: .semibold))
                                Spacer(minLength: 0)
                            }
                            .foregroundStyle(manager.isUserBlocked(chatId) ? palette.textPrimary : .red)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("directChatBlockButton")

                        Divider().overlay(palette.border)

#if os(iOS)
                        // The mailto: hand-off to irismessenger@pm.me is
                        // wired specifically for the iOS App Store
                        // user-generated-content review process — Apple
                        // requires every UGC-bearing iOS app to expose
                        // a way to flag abusive content. Other platforms
                        // route abuse handling through their own surfaces
                        // (Zapstore listing, GitHub issues, the irischat.org
                        // contact), so the in-app Report button is
                        // intentionally iOS-only.
                        Button(role: .destructive) {
                            showingReportConfirmation = true
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: "flag.fill")
                                    .frame(width: 24)
                                Text("Report user")
                                    .font(.system(.body, design: .rounded, weight: .semibold))
                                Spacer(minLength: 0)
                            }
                            .foregroundStyle(.red)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("directChatReportButton")

                        Divider().overlay(palette.border)
#endif

                        Button(role: .destructive) {
                            manager.dispatch(.deleteChat(chatId: chatId))
                            onClose()
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: "trash")
                                    .frame(width: 24)
                                Text("Delete chat")
                                    .font(.system(.body, design: .rounded, weight: .semibold))
                                Spacer(minLength: 0)
                            }
                            .foregroundStyle(.red)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("directChatDeleteButton")
                    }
                } else {
                    ProgressView()
                        .padding(.top, 40)
                }
            }
            .padding(.horizontal, 18)
            .padding(.bottom, 24)
            .frame(maxWidth: .infinity, alignment: .leading)
            .textSelection(.enabled)
        }
        .background(palette.background)
        .irisProfilePictureViewer(
            item: $profilePictureViewerItem,
            preferences: manager.state.preferences,
            manager: manager
        )
        .task(id: chatId) {
            loadCommonGroups()
        }
        .confirmationDialog(
            "Block user?",
            isPresented: $showingBlockConfirmation,
            titleVisibility: .visible
        ) {
            Button("Block user", role: .destructive) {
                manager.setUserBlocked(chatId, blocked: true)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("You will not send messages to this user.")
        }
        .confirmationDialog(
            "Unblock user?",
            isPresented: $showingUnblockConfirmation,
            titleVisibility: .visible
        ) {
            Button("Unblock user") {
                manager.setUserBlocked(chatId, blocked: false)
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog(
            "Report user?",
            isPresented: $showingReportConfirmation,
            titleVisibility: .visible
        ) {
            Button("Report and block", role: .destructive) {
                if let chat {
                    reportUser(chat, block: true)
                }
            }
            Button("Report only") {
                if let chat {
                    reportUser(chat, block: false)
                }
            }
            Button("Cancel", role: .cancel) {}
        }
    }

    @ViewBuilder
    private func directChatAvatar(_ chat: CurrentChatSnapshot) -> some View {
        if let item = IrisProfilePictureViewerItem(
            label: chat.displayName,
            pictureUrl: chat.pictureUrl,
            accessibilityIdentifier: "directChatProfilePictureViewer"
        ) {
            Button {
                profilePictureViewerItem = item
            } label: {
                directChatAvatarImage(chat)
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel("Open profile picture")
            .accessibilityIdentifier("directChatProfilePictureButton")
        } else {
            directChatAvatarImage(chat)
        }
    }

    private func directChatAvatarImage(_ chat: CurrentChatSnapshot) -> some View {
        IrisAvatar(
            label: chat.displayName,
            size: 96,
            emphasize: true,
            pictureUrl: chat.pictureUrl,
            preferences: manager.state.preferences,
            manager: manager,
            loadedImageIdentifier: "directChatProfileAvatarImage"
        )
    }

    @ViewBuilder
    private func commonGroupRow(_ group: ChatThreadSnapshot) -> some View {
        Button {
            if let groupId = groupId(from: group.chatId) {
                onClose()
                manager.dispatch(.pushScreen(screen: .groupDetails(groupId: groupId)))
            }
        } label: {
            HStack(spacing: 12) {
                IrisAvatar(
                    label: group.displayName,
                    size: 38,
                    emphasize: false,
                    pictureUrl: group.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager
                )
                VStack(alignment: .leading, spacing: 3) {
                    Text(group.displayName)
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                        .lineLimit(1)
                    Text("\(group.memberCount) people")
                        .font(.system(.footnote, design: .rounded))
                        .foregroundStyle(palette.muted)
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 9)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier("directChatCommonGroup-\(String(group.chatId.prefix(12)))")
    }

    private func loadProfileDebugIfNeeded() {
        guard advancedExpanded else { return }
        if loadedProfileDebugFor != chatId {
            profileDebug = nil
            loadedProfileDebugFor = chatId
        }
        guard profileDebug == nil else { return }
        profileDebug = manager.peerProfileDebug(ownerInput: chatId)
    }

    private func loadCommonGroups() {
        guard commonGroupsLoadedFor != chatId else { return }
        commonGroupsLoadedFor = chatId
        commonGroups = manager.mutualGroups(ownerInput: chatId)
    }

    private func profileAboutRow(_ about: String) -> some View {
        IrisSectionCard {
            HStack(alignment: .top, spacing: 14) {
                Image(systemName: "square.and.pencil")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .frame(width: 24, height: 24)
                    .padding(.top, 1)

                highlightedProfileAboutText(about, linkColor: palette.accent)
                    .font(.system(.body, design: .rounded))
                    .foregroundStyle(palette.textPrimary)
                    .lineLimit(3)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .accessibilityIdentifier("directChatAboutCard")
    }

    @ViewBuilder
    private func nicknameCard(_ chat: CurrentChatSnapshot) -> some View {
        let storedNickname = trimmedText(chat.nickname) ?? ""
        let normalizedDraft = nicknameDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        let profileName = secondaryDisplayName(chat.profileName, primary: storedNickname.isEmpty ? chat.displayName : storedNickname)

        IrisSectionCard {
            Button {
                editingNickname.toggle()
            } label: {
                HStack(spacing: 12) {
                    Text("Nickname")
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    Spacer(minLength: 0)
                    if !storedNickname.isEmpty {
                        Text(storedNickname)
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.textPrimary)
                            .lineLimit(1)
                    }
                    Image(systemName: editingNickname ? "chevron.up" : "chevron.down")
                        .font(.system(.footnote, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .accessibilityIdentifier("directChatNicknameRow")

            if let profileName {
                Divider().overlay(palette.border)
                HStack(spacing: 12) {
                    Text("Profile name")
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    Spacer(minLength: 0)
                    Text(profileName)
                        .font(.system(.body, design: .rounded))
                        .foregroundStyle(palette.muted)
                        .multilineTextAlignment(.trailing)
                }
                .accessibilityIdentifier("directChatProfileNameRow")
            }

            if editingNickname {
                Divider().overlay(palette.border)

                TextField("Nickname", text: $nicknameDraft)
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .submitLabel(.done)
                    .onSubmit(saveNickname)
                    .accessibilityIdentifier("directChatNicknameField")

                HStack(spacing: 10) {
                    Button("Save") {
                        saveNickname()
                    }
                    .buttonStyle(IrisPrimaryButtonStyle(compact: true))
                    .disabled(normalizedDraft == storedNickname)
                    .accessibilityIdentifier("directChatSaveNicknameButton")

                    if !storedNickname.isEmpty {
                        Button("Remove") {
                            nicknameDraft = ""
                            editingNickname = false
                            manager.dispatch(.setContactNickname(ownerPubkeyHex: chatId, nickname: ""))
                        }
                        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                        .accessibilityIdentifier("directChatRemoveNicknameButton")
                    }
                }
            }
        }
        .onAppear {
            syncNicknameDraft(chat)
        }
        .irisOnChange(of: "\(chat.chatId)|\(chat.nickname ?? "")") { _ in
            syncNicknameDraft(chat)
        }
    }

    private func syncNicknameDraft(_ chat: CurrentChatSnapshot) {
        let key = "\(chat.chatId)|\(chat.nickname ?? "")"
        guard nicknameDraftLoadedKey != key else { return }
        nicknameDraft = chat.nickname ?? ""
        nicknameDraftLoadedKey = key
    }

    private func saveNickname() {
        manager.dispatch(.setContactNickname(ownerPubkeyHex: chatId, nickname: nicknameDraft))
        editingNickname = false
    }

    private func reportUser(_ chat: CurrentChatSnapshot, block: Bool) {
        if block {
            manager.setUserBlocked(chatId, blocked: true)
        }

        let userId = peerInputToNpub(input: chatId)
        let body = """
        Reported user: \(chat.displayName)
        User ID: \(userId)
        App: Iris Chat \(manager.buildSummaryText())

        What happened:
        """
        guard let url = irisMailtoURL(
            to: irisSupportEmail,
            subject: "Iris Chat user report",
            body: body
        ) else {
            manager.copyToClipboard("User ID: \(userId)")
            return
        }
        PlatformExternalURL.open(url)
    }

    private func groupId(from chatId: String) -> String? {
        let prefix = "group:"
        guard chatId.lowercased().hasPrefix(prefix) else {
            return nil
        }
        let raw = String(chatId.dropFirst(prefix.count))
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return raw.isEmpty ? nil : raw
    }
}

struct DirectChatAdvancedCard: View {
    @Environment(\.irisPalette) private var palette
    let debug: PeerProfileDebugSnapshot?
    @Binding var isExpanded: Bool

    var body: some View {
        IrisSectionCard {
            DisclosureGroup(isExpanded: $isExpanded) {
                if let debug {
                    VStack(alignment: .leading, spacing: 10) {
                        DirectChatDebugRow(label: "Sessions", value: "\(debug.sessionCount)")
                        DirectChatDebugRow(label: "Active sessions", value: "\(debug.activeSessionCount)")
                        DirectChatDebugRow(label: "Receiving sessions", value: "\(debug.receivingSessionCount)")
                        DirectChatDebugRow(label: "Known devices", value: "\(debug.knownDeviceCount)")
                        DirectChatDebugRow(label: "Device roster", value: "\(debug.rosterDeviceCount)")
                        DirectChatDebugRow(label: "Tracked senders", value: "\(debug.trackedSenderCount)")
                        DirectChatDebugRow(label: "Recent handshakes", value: "\(debug.recentHandshakeDeviceCount)")
                        DirectChatDebugRow(label: "Last handshake", value: lastHandshakeText(debug.lastHandshakeAtSecs))
                        DirectChatDebugRow(label: "Message tracking", value: debug.trackedForMessages ? "On" : "Off")
                        MonoValue(label: "User ID", value: debug.ownerNpub)
                        MonoValue(label: "Public key", value: debug.ownerPubkeyHex)
                    }
                    .padding(.top, 10)
                } else {
                    ProgressView()
                        .padding(.top, 10)
                }
            } label: {
                HStack(spacing: 9) {
                    Image(systemName: "wrench.and.screwdriver.fill")
                        .foregroundStyle(palette.textPrimary)
                    Text("Debug")
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                }
            }
        }
    }
}

struct DirectChatDebugRow: View {
    @Environment(\.irisPalette) private var palette
    let label: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(label)
                .font(.system(.body, design: .rounded))
                .foregroundStyle(palette.muted)
            Spacer(minLength: 12)
            Text(value)
                .font(.system(.body, design: .rounded, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(palette.textPrimary)
                .multilineTextAlignment(.trailing)
        }
    }
}

func lastHandshakeText(_ seconds: UInt64?) -> String {
    guard let seconds else { return "Never" }
    return Date(timeIntervalSince1970: TimeInterval(seconds))
        .formatted(date: .abbreviated, time: .shortened)
}

func relayStatusColor(_ status: NetworkStatusSnapshot?, palette: IrisPalette) -> Color {
    guard let status, !status.relayUrls.isEmpty else {
        return palette.muted.opacity(0.55)
    }
    if status.connectedRelayCount > 0 {
        return Color(red: 34.0 / 255.0, green: 197.0 / 255.0, blue: 94.0 / 255.0)
    }
    if status.syncing || status.pendingOutboundCount > 0 || status.pendingGroupControlCount > 0 {
        return Color(red: 234.0 / 255.0, green: 179.0 / 255.0, blue: 8.0 / 255.0)
    }
    return Color(red: 239.0 / 255.0, green: 68.0 / 255.0, blue: 68.0 / 255.0)
}

struct OwnerPresentation {
    let primary: String
    let secondary: String?
}

func trimmedText(_ value: String?) -> String? {
    guard let value else { return nil }
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
}

func highlightedProfileAboutText(_ text: String, linkColor: Color) -> Text {
    let pattern = #"(?i)(?:^|(?<=[\s(\[{<]))((?:https?://|www\.)[^\s<]+|(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}(?::[0-9]{2,5})?(?:/[^\s<]*)?)"#
    guard let regex = try? NSRegularExpression(pattern: pattern) else {
        return Text(text)
    }
    let nsRange = NSRange(text.startIndex..<text.endIndex, in: text)
    let matches = regex.matches(in: text, range: nsRange)
    guard !matches.isEmpty else {
        return Text(text)
    }

    var result = Text("")
    var cursor = text.startIndex
    for match in matches {
        guard var range = Range(match.range(at: 1), in: text) else { continue }
        let visible = String(text[range]).trimmingCharacters(in: profileAboutURLTrailingPunctuation)
        guard !visible.isEmpty else { continue }
        range = range.lowerBound..<text.index(range.lowerBound, offsetBy: visible.count)
        if cursor < range.lowerBound {
            result = result + Text(String(text[cursor..<range.lowerBound]))
        }
        result = result + Text(visible).foregroundColor(linkColor).underline()
        cursor = range.upperBound
    }
    if cursor < text.endIndex {
        result = result + Text(String(text[cursor..<text.endIndex]))
    }
    return result
}

let profileAboutURLTrailingPunctuation = CharacterSet(charactersIn: ".,;:!?)]")

func primaryDisplayName(displayName: String, fallback: String) -> String {
    trimmedText(displayName) ?? fallbackProfileNameForIdentity(fallback)
}

func secondaryDisplayName(_ secondary: String?, primary: String) -> String? {
    guard let secondary = trimmedText(secondary) else {
        return nil
    }
    return secondary.caseInsensitiveCompare(primary) == .orderedSame ? nil : secondary
}

func sameOwner(_ owner: String, hex: String?, npub: String?) -> Bool {
    let rawOwner = owner.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    let normalizedOwner = normalizePeerInput(input: owner).trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    let candidates = [hex, npub]
        .compactMap(trimmedText)
        .map { $0.lowercased() }
    return candidates.contains(rawOwner) || candidates.contains(normalizedOwner)
}

extension Array where Element == ChatThreadSnapshot {
    func filteredByQuery(_ query: String) -> [ChatThreadSnapshot] {
        let raw = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return self }
        let lower = raw.lowercased()
        let normalized = normalizePeerInput(input: raw).lowercased()
        return filter { chat in
            chat.displayName.lowercased().contains(lower)
                || chat.chatId.lowercased().contains(normalized)
                || (chat.subtitle?.lowercased().contains(lower) ?? false)
        }
    }
}

func fallbackProfileNameForIdentity(_ identity: String) -> String {
    let adjectives = [
        "Amber", "Bright", "Calm", "Clear", "Golden", "Lunar",
        "Nova", "Quiet", "Silver", "Solar", "Velvet", "Wild"
    ]
    let nouns = [
        "Aurora", "Comet", "Echo", "Falcon", "Harbor", "Listener",
        "Otter", "Raven", "Signal", "Sparrow", "Tide", "Voyager"
    ]
    let trimmed = identity.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return "Quiet Listener" }

    let hash = trimmed.utf8.reduce(UInt32(0)) { partial, byte in
        partial &* 31 &+ UInt32(byte)
    }
    let adjective = adjectives[Int(hash) % adjectives.count]
    let noun = nouns[(Int(hash) / adjectives.count) % nouns.count]
    return "\(adjective) \(noun)"
}
