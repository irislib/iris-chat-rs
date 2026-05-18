#if os(iOS)
import Foundation

/// Curated state used to paint App Store screenshots. Activated by the
/// `IRIS_UI_TEST_SCREENSHOT_FIXTURE=1` launch environment variable.
///
/// The fixture replaces account display name / picture, the chat list,
/// and per-chat message timelines with deterministic demo content. Tapping
/// a fixture chat row is intercepted in AppManager so it never reaches the
/// Rust core (the chat IDs intentionally don't correspond to anything in
/// the real database — they're just keys into this dictionary).
struct ScreenshotFixture {
    /// Fixture chat IDs are short to stay unique within the 12-character
    /// prefix the chat-row accessibility identifier truncates to.
    static let chatIdPrefix = "fx-chat-"

    /// Owner profile shown in chat list header and settings.
    let ownerDisplayName: String
    /// Ordered chat list. First entry is rendered as the "active" chat on
    /// iPad layouts and used by the welcome-into-chat UI test path.
    let threads: [Thread]
    /// Per-chat message timelines, keyed by `Thread.chatId`.
    let timelines: [String: [Message]]
    /// Synthetic peers shown in the Nearby modal.
    let nearbyPeers: [NearbyPeer]

    struct NearbyPeer {
        let id: String
        let name: String
        let transport: Transport
        enum Transport { case bluetooth, lan }
    }

    struct Thread {
        let chatId: String
        let kind: ChatKind
        let displayName: String
        let subtitle: String?
        let lastMessagePreview: String
        let lastMessageAgeSecs: TimeInterval
        let lastMessageIsOutgoing: Bool
        let unreadCount: UInt64
        let memberCount: UInt64
        let isPinned: Bool
        let isMuted: Bool
    }

    struct Message {
        let body: String
        let isOutgoing: Bool
        let ageSecs: TimeInterval
        let delivery: DeliveryState
        let reactions: [MessageReactionSnapshot]
        /// Group sender name. The chat bubble renders `message.author`
        /// verbatim as the sender name above the bubble — production
        /// chats resolve a hex pubkey to a display name elsewhere; the
        /// fixture short-circuits that by writing the name straight
        /// into the author slot. Ignored for direct chats / outgoing
        /// bubbles.
        let groupAuthorName: String?

        init(
            body: String,
            isOutgoing: Bool,
            ageSecs: TimeInterval,
            delivery: DeliveryState,
            reactions: [MessageReactionSnapshot] = [],
            groupAuthorName: String? = nil
        ) {
            self.body = body
            self.isOutgoing = isOutgoing
            self.ageSecs = ageSecs
            self.delivery = delivery
            self.reactions = reactions
            self.groupAuthorName = groupAuthorName
        }
    }

    static let `default` = ScreenshotFixture(
        ownerDisplayName: "Alex Rivera",
        threads: [
            Thread(
                chatId: "\(chatIdPrefix)1",
                kind: .direct,
                displayName: "Maya Chen",
                subtitle: nil,
                lastMessagePreview: "perfect, see you at 7",
                lastMessageAgeSecs: 60 * 4,
                lastMessageIsOutgoing: false,
                unreadCount: 2,
                memberCount: 2,
                isPinned: true,
                isMuted: false
            ),
            Thread(
                chatId: "\(chatIdPrefix)2",
                kind: .group,
                displayName: "Trip crew ✈️",
                subtitle: "5 members",
                lastMessagePreview: "Lena: booked the cabin",
                lastMessageAgeSecs: 60 * 22,
                lastMessageIsOutgoing: false,
                unreadCount: 4,
                memberCount: 5,
                isPinned: true,
                isMuted: false
            ),
            Thread(
                chatId: "\(chatIdPrefix)3",
                kind: .direct,
                displayName: "Sam Park",
                subtitle: nil,
                lastMessagePreview: "i think i fixed it",
                lastMessageAgeSecs: 60 * 58,
                lastMessageIsOutgoing: true,
                unreadCount: 0,
                memberCount: 2,
                isPinned: false,
                isMuted: false
            ),
            Thread(
                chatId: "\(chatIdPrefix)4",
                kind: .direct,
                displayName: "Priya Anand",
                subtitle: nil,
                lastMessagePreview: "ha, good one 😂",
                lastMessageAgeSecs: 60 * 60 * 3,
                lastMessageIsOutgoing: false,
                unreadCount: 0,
                memberCount: 2,
                isPinned: false,
                isMuted: false
            ),
            Thread(
                chatId: "\(chatIdPrefix)5",
                kind: .group,
                displayName: "Book club",
                subtitle: "8 members",
                lastMessagePreview: "Theo: chapter 4 was wild",
                lastMessageAgeSecs: 60 * 60 * 7,
                lastMessageIsOutgoing: false,
                unreadCount: 0,
                memberCount: 8,
                isPinned: false,
                isMuted: true
            ),
            Thread(
                chatId: "\(chatIdPrefix)6",
                kind: .direct,
                displayName: "Jamie Olufemi",
                subtitle: nil,
                lastMessagePreview: "thanks again ❤️",
                lastMessageAgeSecs: 60 * 60 * 28,
                lastMessageIsOutgoing: true,
                unreadCount: 0,
                memberCount: 2,
                isPinned: false,
                isMuted: false
            ),
            Thread(
                chatId: "\(chatIdPrefix)7",
                kind: .direct,
                displayName: "Mom",
                subtitle: nil,
                lastMessagePreview: "love you, sweetie",
                lastMessageAgeSecs: 60 * 60 * 32,
                lastMessageIsOutgoing: false,
                unreadCount: 0,
                memberCount: 2,
                isPinned: false,
                isMuted: false
            ),
        ],
        timelines: [
            "\(chatIdPrefix)1": [
                Message(body: "hey, dinner still on tonight?", isOutgoing: true, ageSecs: 60 * 60 * 5, delivery: .seen, reactions: []),
                Message(body: "yes! the new ramen place in town", isOutgoing: false, ageSecs: 60 * 60 * 5 - 90, delivery: .seen, reactions: []),
                Message(body: "i hear the spicy miso is incredible", isOutgoing: false, ageSecs: 60 * 60 * 5 - 150, delivery: .seen, reactions: []),
                Message(body: "perfect. 7?", isOutgoing: true, ageSecs: 60 * 13, delivery: .seen, reactions: [
                    MessageReactionSnapshot(emoji: "❤️", count: 1, reactedByMe: false),
                ]),
                Message(body: "perfect, see you at 7", isOutgoing: false, ageSecs: 60 * 4, delivery: .seen, reactions: []),
            ],
            "\(chatIdPrefix)2": [
                Message(body: "ok the booking links are in", isOutgoing: false, ageSecs: 60 * 60 * 4, delivery: .seen, groupAuthorName: "Theo"),
                Message(body: "huge", isOutgoing: true, ageSecs: 60 * 60 * 4 - 30, delivery: .seen),
                Message(
                    body: "booked the cabin",
                    isOutgoing: false,
                    ageSecs: 60 * 22,
                    delivery: .seen,
                    reactions: [
                        MessageReactionSnapshot(emoji: "🔥", count: 3, reactedByMe: true),
                    ],
                    groupAuthorName: "Lena"
                ),
            ],
        ],
        nearbyPeers: [
            NearbyPeer(id: "fx-near-1", name: "Lena Park", transport: .bluetooth),
            NearbyPeer(id: "fx-near-2", name: "Theo Asante", transport: .bluetooth),
            NearbyPeer(id: "fx-near-3", name: "Noa Klein", transport: .bluetooth),
            NearbyPeer(id: "fx-near-4", name: "Iris Cafe TV", transport: .lan),
            NearbyPeer(id: "fx-near-5", name: "Kai Wong", transport: .lan),
        ]
    )
}

extension ScreenshotFixture {
    static func enabled(environment: [String: String]) -> Bool {
        environment["IRIS_UI_TEST_SCREENSHOT_FIXTURE"] == "1"
    }

    func chatIsFixture(_ chatId: String) -> Bool {
        chatId.hasPrefix(Self.chatIdPrefix)
    }

    /// Apply the fixture overrides on top of a state snapshot. Called from
    /// AppManager.apply(update:) on the iOS dispatch path.
    func applyTo(state: AppState, referenceDate: Date) -> AppState {
        guard let account = state.account else {
            return state
        }
        var next = state

        // Override account display name / picture for the chrome avatar.
        var overriddenAccount = account
        overriddenAccount.displayName = ownerDisplayName
        overriddenAccount.pictureUrl = nil
        next.account = overriddenAccount

        // Force the Nearby row visible in the chat list — the row only
        // renders when `preferences.nearbyEnabled` is true.
        next.preferences.nearbyEnabled = true

        // Replace the chat list with our curated threads.
        let ownerHex = account.publicKeyHex
        next.chatList = threads.map { thread in
            self.threadSnapshot(thread, referenceDate: referenceDate)
        }

        // If the active screen is a fixture chat, inject the timeline.
        if case let .chat(chatId) = state.router.screenStack.last,
           chatIsFixture(chatId),
           let thread = threads.first(where: { $0.chatId == chatId }) {
            next.currentChat = currentChatSnapshot(
                thread: thread,
                ownerHex: ownerHex,
                referenceDate: referenceDate
            )
        } else if next.currentChat?.chatId.hasPrefix(Self.chatIdPrefix) == true {
            // Clear when we navigate away.
            next.currentChat = nil
        }

        return next
    }

    private func threadSnapshot(
        _ thread: Thread,
        referenceDate: Date
    ) -> ChatThreadSnapshot {
        let timestamp = referenceDate.addingTimeInterval(-thread.lastMessageAgeSecs)
        return ChatThreadSnapshot(
            chatId: thread.chatId,
            kind: thread.kind,
            displayName: thread.displayName,
            nickname: nil,
            profileName: nil,
            subtitle: thread.subtitle,
            pictureUrl: nil,
            memberCount: thread.memberCount,
            lastMessagePreview: thread.lastMessagePreview,
            lastMessageAtSecs: UInt64(max(0, timestamp.timeIntervalSince1970)),
            lastMessageIsOutgoing: thread.lastMessageIsOutgoing,
            lastMessageDelivery: thread.lastMessageIsOutgoing ? .seen : nil,
            unreadCount: thread.unreadCount,
            isTyping: false,
            isMuted: thread.isMuted,
            isPinned: thread.isPinned,
            draft: "",
            isRequest: false
        )
    }

    private func currentChatSnapshot(
        thread: Thread,
        ownerHex: String,
        referenceDate: Date
    ) -> CurrentChatSnapshot {
        let timeline = timelines[thread.chatId] ?? defaultTimeline(for: thread)
        let messages = timeline.enumerated().map { index, message in
            messageSnapshot(
                message,
                index: index,
                chat: thread,
                ownerHex: ownerHex,
                referenceDate: referenceDate
            )
        }
        return CurrentChatSnapshot(
            chatId: thread.chatId,
            kind: thread.kind,
            displayName: thread.displayName,
            nickname: nil,
            profileName: nil,
            subtitle: thread.subtitle,
            pictureUrl: nil,
            groupId: thread.kind == .group ? thread.chatId : nil,
            memberCount: thread.memberCount,
            messageTtlSeconds: nil,
            isMuted: thread.isMuted,
            participants: [],
            messages: messages,
            typingIndicators: [],
            draft: "",
            isRequest: false
        )
    }

    private func defaultTimeline(for thread: Thread) -> [Message] {
        [
            Message(
                body: thread.lastMessagePreview,
                isOutgoing: thread.lastMessageIsOutgoing,
                ageSecs: thread.lastMessageAgeSecs,
                delivery: thread.lastMessageIsOutgoing ? .seen : .seen,
                reactions: []
            ),
        ]
    }

    private func messageSnapshot(
        _ message: Message,
        index: Int,
        chat: Thread,
        ownerHex: String,
        referenceDate: Date
    ) -> ChatMessageSnapshot {
        let timestamp = referenceDate.addingTimeInterval(-message.ageSecs)
        let createdAt = UInt64(max(0, timestamp.timeIntervalSince1970))
        let author: String
        if message.isOutgoing {
            author = ownerHex
        } else if let name = message.groupAuthorName {
            author = name
        } else {
            author = Self.syntheticPeerHex(for: chat.chatId, index: index)
        }
        let authorOwnerPubkeyHex: String?
        if message.isOutgoing {
            authorOwnerPubkeyHex = ownerHex
        } else if message.groupAuthorName == nil {
            authorOwnerPubkeyHex = author
        } else {
            authorOwnerPubkeyHex = nil
        }
        let id = "\(chat.chatId)-msg-\(index)"
        return ChatMessageSnapshot(
            id: id,
            chatId: chat.chatId,
            kind: .user,
            author: author,
            authorOwnerPubkeyHex: authorOwnerPubkeyHex,
            authorPictureUrl: nil,
            body: message.body,
            attachments: [],
            reactions: message.reactions,
            reactors: [],
            isOutgoing: message.isOutgoing,
            createdAtSecs: createdAt,
            expiresAtSecs: nil,
            delivery: message.delivery,
            recipientDeliveries: [],
            deliveryTrace: MessageDeliveryTraceSnapshot(
                outerEventIds: [],
                pendingRelayEventIds: [],
                queuedProtocolTargets: [],
                targetDeviceIds: [],
                transportChannels: [],
                lastTransportError: nil
            ),
            sourceEventId: nil
        )
    }

    /// IrisNearbyPeer values for the synthetic nearby fixture peers.
    /// `firstPeerOwnerHex` lets the nearby→chat e2e test wire the first
    /// peer's tap target to a fixture chat id without disturbing the
    /// screenshot-test output (where peers stay disabled).
    func nearbyPeerSnapshots(firstPeerOwnerHex: String? = nil) -> [IrisNearbyPeer] {
        let now = Date()
        return nearbyPeers.enumerated().map { index, peer in
            IrisNearbyPeer(
                id: peer.id,
                name: peer.name,
                ownerPubkeyHex: index == 0 ? firstPeerOwnerHex : nil,
                pictureURL: nil,
                profileEventID: nil,
                lastSeen: now
            )
        }
    }

    func nearbyBluetoothPeerIDs() -> [String] {
        nearbyPeers.filter { $0.transport == .bluetooth }.map(\.id)
    }

    func nearbyLanPeerIDs() -> [String] {
        nearbyPeers.filter { $0.transport == .lan }.map(\.id)
    }

    private static func syntheticPeerHex(for chatId: String, index: Int) -> String {
        // 64-hex pubkey-shaped string. Doesn't need to be a real pubkey
        // because nothing decrypts it — the chat UI just hashes it for
        // grouping author bubbles in groups.
        let seed = "fixture-\(chatId)-\(index)"
        var hash: UInt64 = 1469598103934665603
        for byte in seed.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 1099511628211
        }
        let hex = String(hash, radix: 16, uppercase: false)
        return String(String(repeating: hex, count: 8).prefix(64))
    }
}
#endif
