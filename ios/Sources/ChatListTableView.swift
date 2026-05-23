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

struct ChatListRowContainer: View {
    // Plain reference — the parent ChatListScreen already observes `manager`
    // and rebuilds this container with fresh `chat` / `timeLabel` / `preferences`
    // values when state changes. Subscribing here would re-evaluate every row
    // on every manager publish (typing pings, relay events, …), which on a
    // chat list of any size adds up to noticeable CPU + battery drain.
    let manager: AppManager
    let chat: ChatThreadSnapshot
    let timeLabel: String?
    let preferences: PreferencesSnapshot
    @State private var confirmingDelete = false

    @ViewBuilder
    var body: some View {
        let row = chatRow

#if os(macOS)
        row.contextMenu {
            chatListItemContextMenu(manager: manager, chat: chat) {
                confirmingDelete = true
            }
        }
        .confirmationDialog(
            "Delete chat?",
            isPresented: $confirmingDelete,
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                manager.dispatch(.deleteChat(chatId: chat.chatId))
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes messages from this device.")
        }
#else
        row
#endif
    }

    private var chatRow: some View {
        let trimmedDraft = chat.draft.trimmingCharacters(in: .whitespacesAndNewlines)
        let preview: String = {
            if chat.isTyping { return "Typing" }
            if !trimmedDraft.isEmpty { return "Draft: \(trimmedDraft)" }
            return chat.lastMessagePreview ?? chat.subtitle ?? "No messages yet"
        }()
        return IrisChatRow(
            title: chat.displayName,
            isMuted: chat.isMuted,
            isPinned: chat.isPinned,
            preview: preview,
            draftPreview: chat.isTyping || trimmedDraft.isEmpty ? nil : trimmedDraft,
            subtitle: nil,
            timeLabel: timeLabel,
            unreadCount: chat.unreadCount,
            pictureUrl: chat.pictureUrl,
            preferences: preferences,
            manager: manager,
            onTap: {
                manager.dispatch(.openChat(chatId: chat.chatId))
            }
        )
    }
}

#if os(iOS)
struct ChatListTableView: UIViewRepresentable {
    @Binding var searchText: String
    let manager: AppManager
    let chats: [ChatThreadSnapshot]
    let preferences: PreferencesSnapshot
    let relativeNow: Date
    let palette: IrisPalette
    let topContentInset: CGFloat
    let isSearchActive: Bool
    let cachedSearchResults: SearchResultSnapshot?
    let expandedSearchSections: Set<ChatListSearchSection>
    let messageLimit: UInt32
    let onOpenNearby: () -> Void
    let onOpenNearbyPeerProfile: (String) -> Void
    let onShortcutNavigate: () -> Void
    let onViewMoreSearchResults: (ChatListSearchSection) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> UITableView {
        let tableView = ChatListScrollTableView(frame: .zero, style: .grouped)
        tableView.dataSource = context.coordinator
        tableView.delegate = context.coordinator
        tableView.backgroundColor = .clear
        tableView.separatorStyle = .none
        tableView.rowHeight = UITableView.automaticDimension
        tableView.estimatedRowHeight = 80
        tableView.contentInsetAdjustmentBehavior = .never
        tableView.keyboardDismissMode = .interactive
        tableView.sectionHeaderTopPadding = 0
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: Coordinator.cellReuseIdentifier)
        tableView.accessibilityIdentifier = "chatListTable"
        return tableView
    }

    func updateUIView(_ tableView: UITableView, context: Context) {
        let sections = makeSections()
        let fingerprint = makeFingerprint()
        updateContentInset(in: tableView)
        updateSearchHeader(in: tableView, context: context)
        context.coordinator.manager = manager
        context.coordinator.preferences = preferences
        context.coordinator.relativeNow = relativeNow
        context.coordinator.palette = palette
        context.coordinator.expandedSearchSections = expandedSearchSections
        context.coordinator.messageLimit = messageLimit
        context.coordinator.onOpenNearby = onOpenNearby
        context.coordinator.onOpenNearbyPeerProfile = onOpenNearbyPeerProfile
        context.coordinator.onShortcutNavigate = onShortcutNavigate
        context.coordinator.onViewMoreSearchResults = onViewMoreSearchResults
        context.coordinator.sections = sections
        guard context.coordinator.fingerprint != fingerprint else { return }
        context.coordinator.fingerprint = fingerprint
        tableView.reloadData()
    }

    private func updateContentInset(in tableView: UITableView) {
        let previousTopInset = tableView.contentInset.top
        guard abs(previousTopInset - topContentInset) > 0.5 else { return }

        let wasPinnedToTop = tableView.contentOffset.y <= -previousTopInset + 1
            || (previousTopInset == 0 && abs(tableView.contentOffset.y) <= 1)

        var contentInset = tableView.contentInset
        contentInset.top = topContentInset
        tableView.contentInset = contentInset

        var indicatorInsets = tableView.verticalScrollIndicatorInsets
        indicatorInsets.top = topContentInset
        tableView.verticalScrollIndicatorInsets = indicatorInsets

        if wasPinnedToTop {
            tableView.contentOffset.y = -topContentInset
        }
    }

    private func updateSearchHeader(in tableView: UITableView, context: Context) {
        let rootView = AnyView(
            ChatListSearchField(text: $searchText)
                .environment(\.irisPalette, palette)
        )
        let controller: UIHostingController<AnyView>
        if let existing = context.coordinator.searchHeaderController {
            existing.rootView = rootView
            controller = existing
        } else {
            let created = UIHostingController(rootView: rootView)
            created.view.backgroundColor = .clear
            context.coordinator.searchHeaderController = created
            controller = created
        }

        let targetSize = CGSize(
            width: max(tableView.bounds.width, 1),
            height: UIView.layoutFittingCompressedSize.height
        )
        let fittingSize = controller.view.systemLayoutSizeFitting(
            targetSize,
            withHorizontalFittingPriority: .required,
            verticalFittingPriority: .fittingSizeLevel
        )
        let headerHeight = ceil(fittingSize.height)
        var frame = controller.view.frame
        frame.size = CGSize(width: tableView.bounds.width, height: headerHeight)
        controller.view.frame = frame

        if tableView.tableHeaderView !== controller.view ||
            abs((tableView.tableHeaderView?.frame.height ?? 0) - headerHeight) > 0.5 {
            tableView.tableHeaderView = controller.view
        }
    }

    private func makeSections() -> [Section] {
        if isSearchActive {
            return [
                Section(
                    title: nil,
                    items: cachedSearchResults.map { [.searchResults($0)] } ?? []
                )
            ]
        }

        var sections: [Section] = []
        if preferences.nearbyShowInChatList {
            sections.append(Section(title: "Nearby", items: [.nearby]))
        }
        let pinnedChats = chats.filter(\.isPinned)
        let unpinnedChats = chats.filter { !$0.isPinned }
        if !pinnedChats.isEmpty {
            sections.append(Section(title: "Pinned", items: pinnedChats.map(Item.chat)))
        }

        if chats.isEmpty {
            sections.append(Section(title: sections.isEmpty ? nil : "Chats", items: [.empty]))
        } else if !unpinnedChats.isEmpty {
            let title = sections.isEmpty ? nil : "Chats"
            sections.append(Section(title: title, items: unpinnedChats.map(Item.chat)))
        }

        return sections
    }

    private func makeFingerprint() -> [String] {
        let expanded = expandedSearchSections.map(\.rawValue).sorted().joined(separator: ",")
        var values = [
            "time:\(Int(relativeNow.timeIntervalSince1970 / 30))",
            "search:\(isSearchActive):\(searchText):\(messageLimit):\(expanded)",
            "nearby:\(preferences.nearbyShowInChatList):\(preferences.nearbyEnabled):\(manager.nearbyIris.sidebarSubtitle):" +
                "\(manager.nearbyIris.isVisible):\(manager.nearbyIris.isLanVisible):" +
                manager.nearbyIris.peers.map {
                    "\($0.id):\($0.name):\($0.pictureURL ?? ""):\($0.ownerPubkeyHex ?? "")"
                }.joined(separator: "\u{1E}"),
        ]
        if let cachedSearchResults {
            values.append(
                "results:\(cachedSearchResults.query):" +
                    "\(cachedSearchResults.contacts.count):" +
                    "\(cachedSearchResults.groups.count):" +
                    "\(cachedSearchResults.messages.count):" +
                    "\(cachedSearchResults.shortcut == nil)"
            )
        }
        values.append("hasPinned:\(chats.contains { $0.isPinned })")
        if chats.isEmpty {
            values.append("empty")
        } else {
            for chat in chats {
                var chatValues: [String] = []
                chatValues.append(chat.chatId)
                chatValues.append(chat.displayName)
                chatValues.append(chat.lastMessagePreview ?? "")
                chatValues.append(chat.subtitle ?? "")
                chatValues.append(chat.draft)
                chatValues.append(chat.lastMessageAtSecs.map(String.init) ?? "")
                chatValues.append(String(chat.unreadCount))
                chatValues.append(String(chat.isMuted))
                chatValues.append(String(chat.isPinned))
                chatValues.append(String(chat.isTyping))
                chatValues.append(chat.pictureUrl ?? "")
                values.append(chatValues.joined(separator: "\u{1F}"))
            }
        }
        return values
    }

    enum Item {
        case nearby
        case empty
        case searchResults(SearchResultSnapshot)
        case chat(ChatThreadSnapshot)
    }

    struct Section {
        let title: String?
        let items: [Item]
    }

    final class Coordinator: NSObject, UITableViewDataSource, UITableViewDelegate {
        static let cellReuseIdentifier = "ChatListTableCell"

        weak var manager: AppManager?
        var searchHeaderController: UIHostingController<AnyView>?
        var sections: [Section] = []
        var preferences: PreferencesSnapshot?
        var relativeNow = Date()
        var palette = IrisPalette.light
        var expandedSearchSections: Set<ChatListSearchSection> = []
        var messageLimit: UInt32 = 0
        var onOpenNearby: (() -> Void)?
        var onOpenNearbyPeerProfile: ((String) -> Void)?
        var onShortcutNavigate: (() -> Void)?
        var onViewMoreSearchResults: ((ChatListSearchSection) -> Void)?
        var fingerprint: [String] = []

        func numberOfSections(in tableView: UITableView) -> Int {
            sections.count
        }

        func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
            sections[section].items.count
        }

        func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
            let cell = tableView.dequeueReusableCell(withIdentifier: Self.cellReuseIdentifier, for: indexPath)
            cell.backgroundColor = .clear
            cell.selectedBackgroundView = UIView()
            cell.accessoryType = .none
            cell.isAccessibilityElement = true
            cell.accessibilityTraits = []

            switch item(at: indexPath) {
            case .nearby:
                configureNearby(cell)
            case .empty:
                configureEmpty(cell)
            case let .searchResults(results):
                configureSearchResults(cell, results: results)
            case let .chat(chat):
                configureChat(cell, chat: chat)
            }

            return cell
        }

        func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
            tableView.deselectRow(at: indexPath, animated: true)
            switch item(at: indexPath) {
            case .nearby:
                if manager?.nearbyIris.peers.isEmpty != false {
                    onOpenNearby?()
                }
            case .empty, .searchResults:
                break
            case let .chat(chat):
                manager?.dispatch(.openChat(chatId: chat.chatId))
            }
        }

        func tableView(
            _ tableView: UITableView,
            leadingSwipeActionsConfigurationForRowAt indexPath: IndexPath
        ) -> UISwipeActionsConfiguration? {
            guard case let .chat(chat) = item(at: indexPath) else { return nil }
            let configuration = UISwipeActionsConfiguration(actions: [
                contextualAction(
                    accessibilityTitle: chat.unreadCount > 0 ? "Read" : "Unread",
                    systemImage: chat.unreadCount > 0 ? "envelope.open.fill" : "envelope.badge.fill",
                    color: .signalUltramarine
                ) { [weak self] in
                    self?.manager?.dispatch(.setChatUnread(chatId: chat.chatId, unread: chat.unreadCount == 0))
                },
                contextualAction(
                    accessibilityTitle: chat.isPinned ? "Unpin" : "Pin",
                    systemImage: chat.isPinned ? "pin.slash.fill" : "pin.fill",
                    color: .signalPinOrange
                ) { [weak self] in
                    self?.manager?.dispatch(.setChatPinned(chatId: chat.chatId, pinned: !chat.isPinned))
                },
            ])
            configuration.performsFirstActionWithFullSwipe = false
            return configuration
        }

        func tableView(
            _ tableView: UITableView,
            trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath
        ) -> UISwipeActionsConfiguration? {
            guard case let .chat(chat) = item(at: indexPath) else { return nil }
            let configuration = UISwipeActionsConfiguration(actions: [
                deleteAction(chat: chat, presentingFrom: tableView),
                contextualAction(
                    accessibilityTitle: chat.isMuted ? "Unmute" : "Mute",
                    systemImage: chat.isMuted ? "bell.fill" : "bell.slash.fill",
                    color: .signalIndigo
                ) { [weak self] in
                    self?.manager?.dispatch(.setChatMuted(chatId: chat.chatId, muted: !chat.isMuted))
                },
            ])
            configuration.performsFirstActionWithFullSwipe = false
            return configuration
        }

        func tableView(_ tableView: UITableView, canEditRowAt indexPath: IndexPath) -> Bool {
            if case .chat = item(at: indexPath) {
                return true
            }
            return false
        }

        func tableView(_ tableView: UITableView, heightForHeaderInSection section: Int) -> CGFloat {
            sections[section].title == nil ? .leastNormalMagnitude : UITableView.automaticDimension
        }

        func tableView(_ tableView: UITableView, heightForFooterInSection section: Int) -> CGFloat {
            .leastNormalMagnitude
        }

        func tableView(_ tableView: UITableView, viewForHeaderInSection section: Int) -> UIView? {
            guard let title = sections[section].title else { return UIView() }

            let container = UIView()
            container.backgroundColor = .clear
            let topMargin: CGFloat = section == 0 ? 14 : 6
            container.layoutMargins = UIEdgeInsets(top: topMargin, left: 16, bottom: 8, right: 16)

            let label = UILabel()
            label.translatesAutoresizingMaskIntoConstraints = false
            label.font = .preferredFont(forTextStyle: .headline)
            label.textColor = UIColor(palette.textPrimary)
            label.text = title
            label.adjustsFontForContentSizeCategory = true
            label.accessibilityTraits.insert(.header)

            container.addSubview(label)
            NSLayoutConstraint.activate([
                label.leadingAnchor.constraint(equalTo: container.layoutMarginsGuide.leadingAnchor),
                label.trailingAnchor.constraint(equalTo: container.layoutMarginsGuide.trailingAnchor),
                label.topAnchor.constraint(equalTo: container.layoutMarginsGuide.topAnchor),
                label.bottomAnchor.constraint(equalTo: container.layoutMarginsGuide.bottomAnchor),
            ])
            return container
        }

        func tableView(_ tableView: UITableView, viewForFooterInSection section: Int) -> UIView? {
            UIView()
        }

        private func item(at indexPath: IndexPath) -> Item {
            sections[indexPath.section].items[indexPath.row]
        }

        private func configureNearby(_ cell: UITableViewCell) {
            guard let manager else { return }
            let service = manager.nearbyIris
            let nearbyEnabled = preferences?.nearbyEnabled ?? true
            cell.accessibilityIdentifier = nil
            cell.accessibilityLabel = nearbyAccessibilityLabel(
                nearbyEnabled: nearbyEnabled,
                hasPeers: nearbyEnabled && !service.peers.isEmpty,
                active: nearbyEnabled && service.isNearbyActive
            )
            cell.accessibilityTraits = []
            cell.isAccessibilityElement = false
            cell.selectionStyle = .none
            cell.contentConfiguration = UIHostingConfiguration {
                NearbyChatListRow(
                    manager: manager,
                    service: manager.nearbyIris,
                    onOpen: { [weak self] in self?.onOpenNearby?() },
                    onOpenPeerProfile: { [weak self] owner in
                        self?.onOpenNearbyPeerProfile?(owner)
                    }
                )
                .environment(\.irisPalette, palette)
            }
            .margins(.all, 0)
        }

        private func configureEmpty(_ cell: UITableViewCell) {
            cell.accessibilityIdentifier = "chatListEmpty"
            cell.accessibilityLabel = "No chats yet"
            cell.accessibilityTraits = [.staticText]
            cell.selectionStyle = .none
            cell.contentConfiguration = UIHostingConfiguration {
                Text("No chats yet")
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 20)
                    .accessibilityHidden(true)
                    .environment(\.irisPalette, palette)
            }
            .margins(.all, 0)
        }

        private func configureSearchResults(_ cell: UITableViewCell, results: SearchResultSnapshot) {
            guard let manager else { return }
            cell.accessibilityIdentifier = "chatListSearchResults"
            cell.accessibilityLabel = "Search results"
            cell.accessibilityTraits = [.staticText]
            cell.selectionStyle = .none
            cell.contentConfiguration = UIHostingConfiguration {
                SearchResultsList(
                    manager: manager,
                    results: results,
                    relativeNow: relativeNow,
                    expandedSections: expandedSearchSections,
                    messageLimit: messageLimit,
                    onShortcutNavigate: { [weak self] in
                        self?.onShortcutNavigate?()
                    },
                    onViewMore: { [weak self] section in
                        self?.onViewMoreSearchResults?(section)
                    }
                )
                .environment(\.irisPalette, palette)
            }
            .margins(.all, 0)
        }

        private func configureChat(_ cell: UITableViewCell, chat: ChatThreadSnapshot) {
            guard let manager else { return }
            let preview = chatListPreview(for: chat)
            let timeLabel = irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow)
            cell.accessibilityIdentifier = "chatRow-\(String(chat.chatId.prefix(12)))"
            cell.accessibilityLabel = [chat.displayName, preview, timeLabel].compactMap { $0 }.joined(separator: ", ")
            cell.accessibilityTraits = [.button]
            cell.selectionStyle = .default
            cell.contentConfiguration = UIHostingConfiguration {
                ChatListTableRowContent(
                    title: chat.displayName,
                    isMuted: chat.isMuted,
                    isPinned: chat.isPinned,
                    preview: preview,
                    draftPreview: chat.isTyping ? nil : trimmedDraftPreview(for: chat),
                    subtitle: nil,
                    timeLabel: timeLabel,
                    unreadCount: chat.unreadCount,
                    pictureUrl: chat.pictureUrl,
                    preferences: preferences,
                    manager: manager
                )
                .accessibilityHidden(true)
                .environment(\.irisPalette, palette)
            }
            .margins(.all, 0)
        }

        private func contextualAction(
            accessibilityTitle: String,
            systemImage: String,
            style: UIContextualAction.Style = .normal,
            color: UIColor,
            handler: @escaping () -> Void
        ) -> UIContextualAction {
            let action = UIContextualAction(style: style, title: "") { _, _, completion in
                handler()
                completion(true)
            }
            action.backgroundColor = color
            action.image = UIImage(systemName: systemImage)
            action.accessibilityLabel = accessibilityTitle
            return action
        }

        private func deleteAction(chat: ChatThreadSnapshot, presentingFrom tableView: UITableView) -> UIContextualAction {
            let action = UIContextualAction(style: .destructive, title: "") { [weak self, weak tableView] _, _, completion in
                guard let self, let tableView else {
                    completion(false)
                    return
                }
                let alert = UIAlertController(title: "Delete chat?", message: nil, preferredStyle: .alert)
                alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in
                    completion(false)
                })
                alert.addAction(UIAlertAction(title: "Delete", style: .destructive) { _ in
                    self.manager?.dispatch(.deleteChat(chatId: chat.chatId))
                    completion(true)
                })
                guard let presenter = tableView.window?.rootViewController else {
                    completion(false)
                    return
                }
                presenter.present(alert, animated: true)
            }
            action.backgroundColor = .signalRed
            action.image = UIImage(systemName: "trash.fill")
            action.accessibilityLabel = "Delete"
            return action
        }

        private func chatListPreview(for chat: ChatThreadSnapshot) -> String {
            let trimmedDraft = trimmedDraftPreview(for: chat) ?? ""
            if chat.isTyping { return "Typing" }
            if !trimmedDraft.isEmpty { return "Draft: \(trimmedDraft)" }
            return chat.lastMessagePreview ?? chat.subtitle ?? "No messages yet"
        }

        private func trimmedDraftPreview(for chat: ChatThreadSnapshot) -> String? {
            let trimmedDraft = chat.draft.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmedDraft.isEmpty ? nil : trimmedDraft
        }
    }
}

final class ChatListScrollTableView: UITableView {
    private let fillerFooterView = UIView()

    override init(frame: CGRect, style: UITableView.Style) {
        super.init(frame: frame, style: style)
        tableFooterView = fillerFooterView
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        updateFooterHeight()
    }

    override func adjustedContentInsetDidChange() {
        super.adjustedContentInsetDidChange()
        updateFooterHeight()
    }

    private func updateFooterHeight() {
        let headerHeight = tableHeaderView?.frame.height ?? 0
        let visibleHeight = max(0, frame.inset(by: adjustedContentInset).height)
        var availableHeight = max(0, visibleHeight - headerHeight)

        for section in 0..<numberOfSections where availableHeight > 0 {
            availableHeight = max(0, availableHeight - rect(forSection: section).height)
        }

        let displayScale = window?.windowScene?.screen.scale ?? UIScreen.main.scale
        let footerHeight = availableHeight + 1 / displayScale
        guard abs(fillerFooterView.frame.height - footerHeight) > 0.5 else { return }

        var footerFrame = fillerFooterView.frame
        footerFrame.size.height = footerHeight
        fillerFooterView.frame = footerFrame
        tableFooterView = fillerFooterView
    }
}

struct ChatListTableRowContent: View {
    @Environment(\.irisPalette) private var palette

    let title: String
    let isMuted: Bool
    let isPinned: Bool
    let preview: String
    let draftPreview: String?
    let subtitle: String?
    let timeLabel: String?
    let unreadCount: UInt64
    let pictureUrl: String?
    let preferences: PreferencesSnapshot?
    let manager: AppManager?
    let leading: AnyView?
    let previewLeading: AnyView?

    init(
        title: String,
        isMuted: Bool = false,
        isPinned: Bool = false,
        preview: String,
        draftPreview: String? = nil,
        subtitle: String?,
        timeLabel: String?,
        unreadCount: UInt64,
        pictureUrl: String? = nil,
        preferences: PreferencesSnapshot? = nil,
        manager: AppManager? = nil,
        leading: AnyView? = nil,
        previewLeading: AnyView? = nil
    ) {
        self.title = title
        self.isMuted = isMuted
        self.isPinned = isPinned
        self.preview = preview
        self.draftPreview = draftPreview
        self.subtitle = subtitle
        self.timeLabel = timeLabel
        self.unreadCount = unreadCount
        self.pictureUrl = pictureUrl
        self.preferences = preferences
        self.manager = manager
        self.leading = leading
        self.previewLeading = previewLeading
    }

    var body: some View {
        HStack(alignment: .center, spacing: IrisChatListRowMetrics.avatarTextSpacing) {
            if let leading {
                leading
            } else {
                IrisAvatar(
                    label: title,
                    size: IrisChatListRowMetrics.avatarSize,
                    emphasize: unreadCount > 0,
                    pictureUrl: pictureUrl,
                    preferences: preferences,
                    manager: manager
                )
            }

            VStack(alignment: .leading, spacing: IrisChatListRowMetrics.textStackSpacing) {
                HStack(alignment: .firstTextBaseline, spacing: IrisChatListRowMetrics.textRowSpacing) {
                    HStack(alignment: .firstTextBaseline, spacing: IrisChatListRowMetrics.titleAccessorySpacing) {
                        Text(title)
                            .font(.headline)
                            .foregroundStyle(palette.textPrimary)
                            .lineLimit(1)

                        if isMuted {
                            Image(systemName: "bell.slash.fill")
                                .font(.system(size: IrisChatListRowMetrics.muteIconSize, weight: .semibold))
                                .foregroundStyle(palette.muted)
                                .accessibilityLabel("muted")
                        }

                    }
                    .layoutPriority(1)

                    Spacer(minLength: 8)

                    if let timeLabel, !timeLabel.isEmpty {
                        Text(timeLabel)
                            .font(.subheadline)
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }

                HStack(alignment: .center, spacing: IrisChatListRowMetrics.textRowSpacing) {
                    if let previewLeading {
                        previewLeading
                    }
                    previewText
                        .font(.subheadline)
                        .foregroundStyle(palette.muted)
                        .lineLimit(2, reservesSpace: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .layoutPriority(1)

                    unreadBadge
                }

                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(palette.muted)
                        .lineLimit(1)
                }
            }
        }
        .padding(.horizontal, IrisChatListRowMetrics.horizontalPadding)
        .padding(.vertical, IrisChatListRowMetrics.verticalPadding)
    }

    private var previewText: Text {
        if let draftPreview {
            return Text("Draft: ").italic() + Text(draftPreview)
        }
        return Text(preview)
    }

    @ViewBuilder
    private var unreadBadge: some View {
        IrisUnreadBadge(count: unreadCount)
    }
}

private extension UIColor {
    static var signalUltramarine: UIColor {
        UIColor { traits in
            if traits.accessibilityContrast == .high {
                return traits.userInterfaceStyle == .dark
                    ? UIColor(red: 0x5D / 255, green: 0x92 / 255, blue: 0xFF / 255, alpha: 1)
                    : UIColor(red: 0x0A / 255, green: 0x43 / 255, blue: 0xB9 / 255, alpha: 1)
            }
            return traits.userInterfaceStyle == .dark
                ? UIColor(red: 0x2D / 255, green: 0x70 / 255, blue: 0xFA / 255, alpha: 1)
                : UIColor(red: 0x22 / 255, green: 0x67 / 255, blue: 0xF5 / 255, alpha: 1)
        }
    }

    static var signalIndigo: UIColor {
        UIColor { traits in
            if traits.accessibilityContrast == .high {
                return traits.userInterfaceStyle == .dark
                    ? UIColor(red: 0x7D / 255, green: 0x7A / 255, blue: 0xFF / 255, alpha: 1)
                    : UIColor(red: 0x36 / 255, green: 0x34 / 255, blue: 0xA3 / 255, alpha: 1)
            }
            return traits.userInterfaceStyle == .dark
                ? UIColor(red: 0x5E / 255, green: 0x5C / 255, blue: 0xE6 / 255, alpha: 1)
                : UIColor(red: 0x58 / 255, green: 0x56 / 255, blue: 0xD6 / 255, alpha: 1)
        }
    }

    static var signalRed: UIColor {
        UIColor { traits in
            if traits.accessibilityContrast == .high {
                return traits.userInterfaceStyle == .dark
                    ? UIColor(red: 0xFF / 255, green: 0x69 / 255, blue: 0x61 / 255, alpha: 1)
                    : UIColor(red: 0xD7 / 255, green: 0x00 / 255, blue: 0x15 / 255, alpha: 1)
            }
            return traits.userInterfaceStyle == .dark
                ? UIColor(red: 0xFF / 255, green: 0x45 / 255, blue: 0x3A / 255, alpha: 1)
                : UIColor(red: 0xFF / 255, green: 0x3B / 255, blue: 0x30 / 255, alpha: 1)
        }
    }

    static var signalPinOrange: UIColor {
        UIColor(red: 0xFF / 255, green: 0x99 / 255, blue: 0x0A / 255, alpha: 1)
    }
}
#endif
