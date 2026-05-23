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

struct ChatListScreen: View {
    @Environment(\.irisPalette) private var palette
    @Environment(\.irisNavigationHeaderTopInset) private var navigationHeaderTopInset
    @ObservedObject var manager: AppManager
    let onOpenNearby: () -> Void
    let onOpenNearbyPeerProfile: (String) -> Void
    @State private var searchText: String = ""
    // Cached search response so we only call into the Rust core when
    // the query string itself changes. Previously the body grabbed a
    // fresh `manager.search(...)` snapshot on every SwiftUI body
    // re-eval, which fires on every state push (incoming relay
    // event, typing indicator, message delivery flip, …). On a busy
    // chat that turned into hundreds of FFI calls / SQLite-mutex
    // acquires per second and was visible on iPhone as a warming
    // device. The .task below now refreshes the cache once per
    // searchText edit.
    @State private var cachedSearchResults: SearchResultSnapshot?
    @State private var expandedSearchSections: Set<ChatListSearchSection> = []
    @State private var searchMessageLimit: UInt32 = 50
    @State private var lastExpansionQuery: String = ""
    @State private var relativeNow = Date()

    private static let initialMessageSearchLimit: UInt32 = 50
    private static let messageSearchLimitStep: UInt32 = 50

    init(
        manager: AppManager,
        onOpenNearby: @escaping () -> Void = {},
        onOpenNearbyPeerProfile: @escaping (String) -> Void = { _ in }
    ) {
        self.manager = manager
        self.onOpenNearby = onOpenNearby
        self.onOpenNearbyPeerProfile = onOpenNearbyPeerProfile
    }

    private var trimmedQuery: String {
        searchText.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var searchActive: Bool { !trimmedQuery.isEmpty }

    private var searchRequestToken: String {
        "\(trimmedQuery)|\(searchMessageLimit)"
    }

    var body: some View {
#if os(iOS)
        ChatListTableView(
            searchText: $searchText,
            manager: manager,
            chats: manager.state.chatList,
            preferences: manager.state.preferences,
            relativeNow: relativeNow,
            palette: palette,
            topContentInset: navigationHeaderTopInset,
            isSearchActive: searchActive,
            cachedSearchResults: cachedSearchResults,
            expandedSearchSections: expandedSearchSections,
            messageLimit: searchMessageLimit,
            onOpenNearby: onOpenNearby,
            onOpenNearbyPeerProfile: onOpenNearbyPeerProfile,
            onShortcutNavigate: { searchText = "" },
            onViewMoreSearchResults: viewMoreSearchResults
        )
        .background(palette.background)
        .irisOnChange(of: searchText) { _ in
            resetSearchExpansionIfNeeded()
            autoProceedIfShortcut()
        }
        .onReceive(chatListRelativeTimeTicker) { date in
            relativeNow = date
        }
        .task(id: searchRequestToken) {
            // Refresh the search cache once per query change. Body
            // re-runs on every state push otherwise — the cache turns
            // an O(state-push) FTS5 query into an O(keystroke) one.
            cachedSearchResults = trimmedQuery.isEmpty
                ? nil
                : manager.search(trimmedQuery, limit: searchMessageLimit)
        }
#else
        ScrollView {
            LazyVStack(spacing: 0) {
                ChatListSearchField(text: $searchText)

                if searchActive {
                    if let results = cachedSearchResults {
                        SearchResultsList(
                            manager: manager,
                            results: results,
                            relativeNow: relativeNow,
                            expandedSections: expandedSearchSections,
                            messageLimit: searchMessageLimit,
                            onShortcutNavigate: { searchText = "" },
                            onViewMore: viewMoreSearchResults
                        )
                    }
                } else {
#if os(iOS) || os(macOS)
                    if manager.state.preferences.nearbyShowInChatList {
                        NearbyChatListRow(
                            manager: manager,
                            service: manager.nearbyIris,
                            onOpen: onOpenNearby,
                            onOpenPeerProfile: onOpenNearbyPeerProfile
                        )
                    }
#endif

                    if manager.state.chatList.isEmpty {
                        Text("No chats yet")
                            .font(.system(.body, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.muted)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 20)
                    } else {
                        let preferences = manager.state.preferences
                        ForEach(manager.state.chatList, id: \.chatId) { chat in
                            ChatListRowContainer(
                                manager: manager,
                                chat: chat,
                                timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow),
                                preferences: preferences
                            )
                            .accessibilityIdentifier("chatRow-\(String(chat.chatId.prefix(12)))")
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .top)
        }
        .background(palette.background)
        .irisOnChange(of: searchText) { _ in
            resetSearchExpansionIfNeeded()
            autoProceedIfShortcut()
        }
        .onReceive(chatListRelativeTimeTicker) { date in
            relativeNow = date
        }
        .task(id: searchRequestToken) {
            // Refresh the search cache once per query change. Body
            // re-runs on every state push otherwise — the cache turns
            // an O(state-push) FTS5 query into an O(keystroke) one.
            cachedSearchResults = trimmedQuery.isEmpty
                ? nil
                : manager.search(trimmedQuery, limit: searchMessageLimit)
        }
#endif
    }

    private func resetSearchExpansionIfNeeded() {
        let query = trimmedQuery
        guard query != lastExpansionQuery else {
            return
        }
        lastExpansionQuery = query
        expandedSearchSections.removeAll()
        searchMessageLimit = Self.initialMessageSearchLimit
    }

    private func viewMoreSearchResults(_ section: ChatListSearchSection) {
        if section == .messages, expandedSearchSections.contains(section) {
            let nextLimit = searchMessageLimit.addingReportingOverflow(Self.messageSearchLimitStep)
            searchMessageLimit = nextLimit.overflow ? UInt32.max : nextLimit.partialValue
        } else {
            expandedSearchSections.insert(section)
        }
    }

    /// Mirrors NewChatScreen's auto-proceed: when the user pastes a
    /// full npub or invite URL into the search bar, dispatch the
    /// matching action without making them tap the shortcut row.
    /// Partial input never classifies, so this is safe to call on
    /// every keystroke.
    private func autoProceedIfShortcut() {
        let trimmed = trimmedQuery
        guard !trimmed.isEmpty,
              let shortcut = classifyChatInput(input: trimmed) else { return }
        switch shortcut {
        case let .directPeer(peerInput, _, _, _):
            searchText = ""
            manager.dispatch(.createChat(peerInput: peerInput))
        case let .invite(inviteInput, _):
            searchText = ""
            manager.dispatch(.acceptInvite(inviteInput: inviteInput))
        }
    }
}

enum ChatListSearchSection: String, Hashable {
    case contacts
    case groups
    case messages
}

/// Always-visible search field at the top of the chat list. Drives the
/// grouped Signal-style search results below it. We render the field
/// inline (instead of using `.searchable`) so it composes cleanly with
/// the custom `NavigationShell` we use across iOS/macOS/Linux instead
/// of a stock `NavigationStack`.
struct ChatListSearchField: View {
    @Environment(\.irisPalette) private var palette
    @Binding var text: String
    @FocusState private var isFocused: Bool

    var body: some View {
#if os(iOS)
        IrisChatListSearchBar(text: $text)
            .frame(height: 52)
            .padding(.horizontal, 8)
            .padding(.top, 4)
            .padding(.bottom, 2)
#else
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(palette.muted)
            TextField("Search chats, groups, messages", text: $text)
                .textFieldStyle(.plain)
                .autocorrectionDisabled(true)
#if os(iOS)
                .textInputAutocapitalization(.never)
#endif
                .focused($isFocused)
                .accessibilityIdentifier("chatListSearchField")
            if isFocused {
                Button {
                    isFocused = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(palette.muted)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close search")
                .accessibilityIdentifier("chatListSearchCloseButton")
            } else if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(palette.muted)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(palette.panelAlt)
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .padding(.horizontal, 12)
        .padding(.top, 10)
        .padding(.bottom, 4)
#endif
    }
}

#if os(iOS)
struct IrisChatListSearchBar: UIViewRepresentable {
    @Environment(\.colorScheme) private var colorScheme
    @Binding var text: String

    private var isDark: Bool {
        colorScheme == .dark
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    func makeUIView(context: Context) -> UISearchBar {
        let searchBar = UISearchBar(frame: .zero)
        searchBar.delegate = context.coordinator
        searchBar.placeholder = "Search"
        searchBar.autocapitalizationType = .none
        searchBar.autocorrectionType = .no
        searchBar.returnKeyType = .search
        searchBar.enablesReturnKeyAutomatically = false
        searchBar.searchBarStyle = .minimal
        searchBar.backgroundColor = .clear
        searchBar.backgroundImage = UIImage()
        searchBar.searchTextField.accessibilityIdentifier = "chatListSearchField"
        searchBar.searchTextField.clearButtonMode = .never
        Self.applyAppearance(to: searchBar, isDark: isDark)
        context.coordinator.attach(to: searchBar)
        return searchBar
    }

    func updateUIView(_ searchBar: UISearchBar, context: Context) {
        if searchBar.text != text {
            searchBar.text = text
        }
        Self.applyAppearance(to: searchBar, isDark: isDark)
        context.coordinator.updateCloseButton(for: searchBar, isDark: isDark)
    }

    private static func applyAppearance(to searchBar: UISearchBar, isDark: Bool) {
        let style: UIUserInterfaceStyle = isDark ? .dark : .light
        let field = searchBar.searchTextField

        searchBar.overrideUserInterfaceStyle = style
        searchBar.tintColor = .label
        searchBar.backgroundColor = .clear
        searchBar.barTintColor = .clear
        searchBar.searchBarStyle = .minimal

        field.overrideUserInterfaceStyle = style
        field.textColor = .label
        field.tintColor = .label
        field.leftView?.tintColor = .secondaryLabel
    }

    final class Coordinator: NSObject, UISearchBarDelegate {
        @Binding private var text: String
        private var isFocused = false
        private lazy var closeButton: UIButton = {
            let button = UIButton(type: .system)
            button.setImage(UIImage(systemName: "xmark.circle.fill"), for: .normal)
            button.tintColor = .secondaryLabel
            button.accessibilityLabel = "Close search"
            button.accessibilityIdentifier = "chatListSearchCloseButton"
            button.addTarget(self, action: #selector(closeSearch), for: .touchUpInside)
            button.frame = CGRect(x: 0, y: 0, width: 28, height: 28)
            return button
        }()

        init(text: Binding<String>) {
            self._text = text
        }

        func attach(to searchBar: UISearchBar) {
            updateCloseButton(for: searchBar, isDark: searchBar.traitCollection.userInterfaceStyle == .dark)
        }

        func updateCloseButton(for searchBar: UISearchBar, isDark: Bool) {
            closeButton.overrideUserInterfaceStyle = isDark ? .dark : .light
            closeButton.tintColor = .secondaryLabel
            searchBar.searchTextField.rightView = isFocused ? closeButton : nil
            searchBar.searchTextField.rightViewMode = isFocused ? .always : .never
        }

        func searchBarTextDidBeginEditing(_ searchBar: UISearchBar) {
            isFocused = true
            updateCloseButton(for: searchBar, isDark: searchBar.overrideUserInterfaceStyle == .dark)
        }

        func searchBarTextDidEndEditing(_ searchBar: UISearchBar) {
            isFocused = false
            updateCloseButton(for: searchBar, isDark: searchBar.overrideUserInterfaceStyle == .dark)
        }

        func searchBar(_ searchBar: UISearchBar, textDidChange searchText: String) {
            text = searchText
        }

        func searchBarSearchButtonClicked(_ searchBar: UISearchBar) {
            searchBar.resignFirstResponder()
        }

        @objc private func closeSearch(_ sender: UIButton) {
            sender.window?.endEditing(true)
        }
    }
}
#endif

struct SearchResultsList: View {
    @Environment(\.irisPalette) private var palette
    let manager: AppManager
    let results: SearchResultSnapshot
    let relativeNow: Date
    let expandedSections: Set<ChatListSearchSection>
    let messageLimit: UInt32
    let onShortcutNavigate: () -> Void
    let onViewMore: (ChatListSearchSection) -> Void

    private let initialChatRows = 7
    private let initialMessageRows = 20

    var body: some View {
        let preferences = manager.state.preferences
        let isEmpty = results.contacts.isEmpty
            && results.groups.isEmpty
            && results.messages.isEmpty
            && results.shortcut == nil

        if isEmpty {
            Text("No matches")
                .font(.system(.body, design: .rounded))
                .foregroundStyle(palette.muted)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 28)
        } else {
            LazyVStack(alignment: .leading, spacing: 0) {
                if let shortcut = results.shortcut {
                    ChatInputShortcutRow(
                        manager: manager,
                        shortcut: shortcut,
                        onNavigate: onShortcutNavigate
                    )
                }
                if !results.contacts.isEmpty {
                    SearchSectionHeader(title: "Contacts")
                    let contacts = visibleRows(
                        results.contacts,
                        section: .contacts,
                        initialCount: initialChatRows
                    )
                    ForEach(contacts, id: \.chatId) { chat in
                        ChatListRowContainer(
                            manager: manager,
                            chat: chat,
                            timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow),
                            preferences: preferences
                        )
                    }
                    if shouldShowMore(results.contacts, visibleRows: contacts, section: .contacts) {
                        SearchViewMoreRow {
                            onViewMore(.contacts)
                        }
                    }
                }
                if !results.groups.isEmpty {
                    SearchSectionHeader(title: "Groups")
                    let groups = visibleRows(
                        results.groups,
                        section: .groups,
                        initialCount: initialChatRows
                    )
                    ForEach(groups, id: \.chatId) { chat in
                        ChatListRowContainer(
                            manager: manager,
                            chat: chat,
                            timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow),
                            preferences: preferences
                        )
                    }
                    if shouldShowMore(results.groups, visibleRows: groups, section: .groups) {
                        SearchViewMoreRow {
                            onViewMore(.groups)
                        }
                    }
                }
                if !results.messages.isEmpty {
                    SearchSectionHeader(title: "Messages")
                    let messages = visibleRows(
                        results.messages,
                        section: .messages,
                        initialCount: initialMessageRows
                    )
                    ForEach(messages, id: \.messageId) { hit in
                        MessageSearchHitRow(
                            manager: manager,
                            hit: hit,
                            relativeNow: relativeNow,
                            preferences: preferences
                        )
                    }
                    if shouldShowMoreMessages(visibleRows: messages) {
                        SearchViewMoreRow {
                            onViewMore(.messages)
                        }
                    }
                }
            }
        }
    }

    private func visibleRows<T>(
        _ rows: [T],
        section: ChatListSearchSection,
        initialCount: Int
    ) -> [T] {
        expandedSections.contains(section) ? rows : Array(rows.prefix(initialCount))
    }

    private func shouldShowMore<T>(
        _ rows: [T],
        visibleRows: [T],
        section: ChatListSearchSection
    ) -> Bool {
        !expandedSections.contains(section) && rows.count > visibleRows.count
    }

    private func shouldShowMoreMessages(visibleRows: [MessageSearchHit]) -> Bool {
        let mayHaveMoreFetchedRows = !expandedSections.contains(.messages)
            && results.messages.count > visibleRows.count
        let mayHaveMoreRemoteRows = expandedSections.contains(.messages)
            && results.messages.count >= Int(messageLimit)
            && messageLimit < UInt32.max
        return mayHaveMoreFetchedRows || mayHaveMoreRemoteRows
    }
}

struct ChatInputShortcutRow: View {
    @Environment(\.irisPalette) private var palette
    let manager: AppManager
    let shortcut: ChatInputShortcut
    let onNavigate: () -> Void

    var body: some View {
        let descriptor = describe(shortcut)
        Button {
            onNavigate()
            manager.dispatch(descriptor.action)
        } label: {
            HStack(spacing: 12) {
                Image(systemName: descriptor.systemImage)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .frame(width: 36, height: 36)
                    .background(palette.panelAlt)
                    .clipShape(Circle())
                VStack(alignment: .leading, spacing: 2) {
                    Text(descriptor.title)
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    Text(descriptor.subtitle)
                        .font(.system(.caption, design: .rounded))
                        .foregroundStyle(palette.muted)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier("chatListSearchShortcut")
    }

    private func describe(_ shortcut: ChatInputShortcut) -> Descriptor {
        switch shortcut {
        case let .directPeer(_, display, _, _):
            return Descriptor(
                systemImage: "person.crop.circle.badge.plus",
                title: "Start chat",
                subtitle: display,
                action: peerAction(shortcut)
            )
        case let .invite(_, display):
            return Descriptor(
                systemImage: "envelope.open",
                title: "Accept invite",
                subtitle: display,
                action: peerAction(shortcut)
            )
        }
    }

    private func peerAction(_ shortcut: ChatInputShortcut) -> AppAction {
        switch shortcut {
        case let .directPeer(peerInput, _, _, _):
            return .createChat(peerInput: peerInput)
        case let .invite(inviteInput, _):
            return .acceptInvite(inviteInput: inviteInput)
        }
    }

    private struct Descriptor {
        let systemImage: String
        let title: String
        let subtitle: String
        let action: AppAction
    }
}

struct SearchSectionHeader: View {
    @Environment(\.irisPalette) private var palette
    let title: String

    var body: some View {
        Text(title)
            .font(.system(.caption, design: .rounded, weight: .semibold))
            .foregroundStyle(palette.muted)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.top, 16)
            .padding(.bottom, 4)
    }
}

struct SearchViewMoreRow: View {
    @Environment(\.irisPalette) private var palette
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: "chevron.down.circle.fill")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .frame(width: 36, height: 36)
                Text("View more")
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier("chatListSearchViewMore")
    }
}

struct MessageSearchHitRow: View {
    let manager: AppManager
    let hit: MessageSearchHit
    let relativeNow: Date
    let preferences: PreferencesSnapshot

    var body: some View {
        IrisChatRow(
            title: hit.chatDisplayName,
            isMuted: false,
            isPinned: false,
            preview: hit.body,
            subtitle: nil,
            timeLabel: irisRelativeTime(hit.createdAtSecs, relativeTo: relativeNow),
            unreadCount: 0,
            pictureUrl: hit.chatPictureUrl,
            preferences: preferences,
            manager: manager,
            onTap: {
                manager.openChatAtMessage(chatId: hit.chatId, messageId: hit.messageId)
            }
        )
        .accessibilityIdentifier("messageHit-\(String(hit.messageId.prefix(12)))")
    }
}

struct InChatSearchTarget: Identifiable, Hashable {
    let chatId: String
    let displayName: String

    var id: String { chatId }
}

struct InChatSearchButton: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let target: InChatSearchTarget
    @State private var presentedTarget: InChatSearchTarget?

    var body: some View {
        Button {
            presentedTarget = target
        } label: {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
                .frame(width: 40, height: 40)
                .frame(width: 48, height: 48)
                .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel("Search in this chat")
        .accessibilityIdentifier("chatHeaderSearchButton")
        .padding(.trailing, 4)
        .sheet(item: $presentedTarget) { target in
            InChatSearchSheet(manager: manager, target: target) {
                presentedTarget = nil
            }
            .irisModalSurface()
#if os(iOS)
            .presentationDetents([.large])
            .presentationDragIndicator(.visible)
#elseif os(macOS)
            .frame(minWidth: 420, minHeight: 520)
#endif
            .irisDismissOnMacOutsideClick { presentedTarget = nil }
        }
    }
}

/// Scoped message search bound to a single conversation. Reached from
/// the chat / group-details header magnifying-glass icon. Tapping a
/// hit dismisses the sheet and opens the chat at that conversation.
struct InChatSearchSheet: View {
    @Environment(\.irisPalette) private var palette
    let manager: AppManager
    let target: InChatSearchTarget
    let onClose: () -> Void
    @State private var query: String = ""
    // Same query-keyed cache as ChatListScreen so a state push
    // (e.g. an incoming message) doesn't re-run the FTS5 query.
    @State private var cachedResults: SearchResultSnapshot?
    @State private var messageSearchLimit: UInt32 = Self.initialMessageSearchLimit
    @FocusState private var isFocused: Bool

    private static let initialMessageSearchLimit: UInt32 = 50
    private static let messageSearchLimitStep: UInt32 = 50

    private var trimmedQuery: String {
        query.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var searchRequestToken: String {
        "\(target.chatId)|\(trimmedQuery)|\(messageSearchLimit)"
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(palette.muted)
                TextField("Search in \(target.displayName)", text: $query)
                    .textFieldStyle(.plain)
                    .autocorrectionDisabled(true)
#if os(iOS)
                    .textInputAutocapitalization(.never)
#endif
                    .focused($isFocused)
                    .accessibilityIdentifier("inChatSearchField")
                if !query.isEmpty {
                    Button {
                        query = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundStyle(palette.muted)
                    }
                    .buttonStyle(.plain)
                }
                IrisModalCloseButton(action: onClose)
                    .accessibilityIdentifier("inChatSearchCloseButton")
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)

            Divider()

            let trimmed = trimmedQuery
            ScrollView {
                if trimmed.isEmpty {
                    Text("Type to search messages in this chat.")
                        .font(.system(.body, design: .rounded))
                        .foregroundStyle(palette.muted)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 48)
                } else if let results = cachedResults,
                          results.query.trimmingCharacters(in: .whitespacesAndNewlines) == trimmed,
                          results.scopeChatId == target.chatId {
                    if results.messages.isEmpty {
                        Text("No matches")
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.muted)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 48)
                    } else {
                        let preferences = manager.state.preferences
                        let now = Date()
                        LazyVStack(spacing: 0) {
                            ForEach(results.messages, id: \.messageId) { hit in
                                IrisChatRow(
                                    title: hit.chatDisplayName,
                                    isMuted: false,
                                    isPinned: false,
                                    preview: hit.body,
                                    subtitle: nil,
                                    timeLabel: irisRelativeTime(hit.createdAtSecs, relativeTo: now),
                                    unreadCount: 0,
                                    pictureUrl: hit.chatPictureUrl,
                                    preferences: preferences,
                                    manager: manager,
                                    onTap: {
                                        manager.openChatAtMessage(
                                            chatId: hit.chatId,
                                            messageId: hit.messageId
                                        )
                                        onClose()
                                    }
                                )
                                .accessibilityIdentifier("inChatMessageHit-\(String(hit.messageId.prefix(12)))")
                            }
                            if results.messages.count >= Int(messageSearchLimit) {
                                SearchViewMoreRow {
                                    viewMoreMessages()
                                }
                            }
                        }
                    }
                }
            }
        }
        .background(palette.background)
        .onAppear { isFocused = true }
        .irisOnChange(of: trimmedQuery) { _ in
            messageSearchLimit = Self.initialMessageSearchLimit
        }
        .task(id: searchRequestToken) {
            let trimmed = trimmedQuery
            cachedResults = trimmed.isEmpty
                ? nil
                : manager.search(trimmed, scopeChatId: target.chatId, limit: messageSearchLimit)
        }
    }

    private func viewMoreMessages() {
        let nextLimit = messageSearchLimit.addingReportingOverflow(Self.messageSearchLimitStep)
        messageSearchLimit = nextLimit.overflow ? UInt32.max : nextLimit.partialValue
    }
}

struct NewChatCircleButton: View {
    @Environment(\.irisPalette) private var palette
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            ZStack(alignment: .trailing) {
                Color.clear
                Image(systemName: "square.and.pencil")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .frame(width: 40, height: 40)
                    .irisGlassSurface(in: Circle())
            }
            .frame(width: 60, height: 48, alignment: .trailing)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .frame(width: 60, height: 48, alignment: .trailing)
        .contentShape(Rectangle())
        .accessibilityLabel("New chat")
        .accessibilityIdentifier("chatListNewChatButton")
    }
}
