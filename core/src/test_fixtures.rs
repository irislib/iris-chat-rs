use crate::{
    AccountSnapshot, AppState, ChatKind, ChatMessageKind, ChatMessageSnapshot, ChatThreadSnapshot,
    CurrentChatSnapshot, DeliveryState, DeviceAuthorizationState, GroupDetailsSnapshot,
    GroupMemberSnapshot, MessageDeliveryTraceSnapshot, MessageReactionSnapshot, MessageReactor,
    MessageSearchHit, PreferencesSnapshot, ProtocolReadinessSnapshot, Router, Screen,
    SearchResultSnapshot,
};

const MAX_FIXTURE_THREADS: u32 = 2_000;
const MAX_FIXTURE_MESSAGES: u32 = 10_000;
const BASE_TIME_SECS: u64 = 1_800_000_000;

/// Deterministic high-volume shell fixture for list, navigation, and message
/// rendering tests. This is intentionally synthetic: relay-backed tests cover
/// protocol behaviour, while this keeps UI/perf tests fast and reproducible.
#[uniffi::export]
pub fn build_large_test_app_state(
    direct_chat_count: u32,
    group_chat_count: u32,
    messages_in_current_chat: u32,
) -> AppState {
    let direct_chat_count = direct_chat_count.min(MAX_FIXTURE_THREADS);
    let group_chat_count = group_chat_count.min(MAX_FIXTURE_THREADS);
    let messages_in_current_chat = messages_in_current_chat.min(MAX_FIXTURE_MESSAGES);

    let mut chat_list =
        Vec::with_capacity((direct_chat_count as usize) + (group_chat_count as usize));
    for index in 0..direct_chat_count {
        chat_list.push(fixture_thread(ChatKind::Direct, index));
    }
    for index in 0..group_chat_count {
        chat_list.push(fixture_thread(ChatKind::Group, index));
    }

    let current_thread = chat_list
        .first()
        .cloned()
        .unwrap_or_else(|| fixture_thread(ChatKind::Direct, 0));
    let messages = (0..messages_in_current_chat)
        .map(|index| fixture_message(&current_thread.chat_id, index))
        .collect();
    let current_chat = CurrentChatSnapshot {
        chat_id: current_thread.chat_id.clone(),
        kind: current_thread.kind.clone(),
        display_name: current_thread.display_name.clone(),
        nickname: current_thread.nickname.clone(),
        profile_name: current_thread.profile_name.clone(),
        subtitle: current_thread.subtitle.clone(),
        picture_url: current_thread.picture_url.clone(),
        about: current_thread.about.clone(),
        group_id: match &current_thread.kind {
            ChatKind::Direct => None,
            ChatKind::Group => Some(current_thread.chat_id.clone()),
        },
        member_count: current_thread.member_count,
        message_ttl_seconds: Some(86_400),
        is_muted: current_thread.is_muted,
        participants: Vec::new(),
        messages,
        typing_indicators: Vec::new(),
        draft: current_thread.draft.clone(),
        is_request: current_thread.is_request,
        protocol_readiness: current_thread.protocol_readiness.clone(),
    };

    AppState {
        rev: 1,
        router: Router {
            default_screen: Screen::ChatList,
            screen_stack: vec![
                Screen::ChatList,
                Screen::Chat {
                    chat_id: current_thread.chat_id.clone(),
                },
            ],
        },
        account: Some(fixture_account()),
        device_roster: None,
        busy: Default::default(),
        chat_list,
        current_chat: Some(current_chat),
        group_details: Some(fixture_group_details(group_chat_count.max(1))),
        public_invite: None,
        link_device: None,
        network_status: None,
        mobile_push: Default::default(),
        preferences: PreferencesSnapshot {
            send_typing_indicators: true,
            nearby_bluetooth_enabled: true,
            nearby_lan_enabled: true,
            ..PreferencesSnapshot::default()
        },
        toast: None,
    }
}

/// Deterministic grouped search fixture for shell tests that need to verify
/// collapsed initial rendering plus "View more" expansion without writing a
/// large on-disk message index first.
#[uniffi::export]
pub fn build_large_test_search_result(
    query: String,
    contact_count: u32,
    group_count: u32,
    message_count: u32,
) -> SearchResultSnapshot {
    let contact_count = contact_count.min(MAX_FIXTURE_THREADS);
    let group_count = group_count.min(MAX_FIXTURE_THREADS);
    let message_count = message_count.min(MAX_FIXTURE_MESSAGES);
    let query = if query.trim().is_empty() {
        "needle".to_string()
    } else {
        query
    };

    SearchResultSnapshot {
        query: query.clone(),
        scope_chat_id: None,
        contacts: (0..contact_count)
            .map(|index| {
                let mut thread = fixture_thread(ChatKind::Direct, index);
                thread.display_name = format!("{} Contact {:04}", title_token(&query), index + 1);
                thread.last_message_preview =
                    Some(format!("{query} appears in contact preview {}", index + 1));
                thread
            })
            .collect(),
        groups: (0..group_count)
            .map(|index| {
                let mut thread = fixture_thread(ChatKind::Group, index);
                thread.display_name = format!("{} Group {:04}", title_token(&query), index + 1);
                thread.last_message_preview =
                    Some(format!("{query} appears in group preview {}", index + 1));
                thread
            })
            .collect(),
        messages: (0..message_count)
            .map(|index| fixture_search_hit(&query, index))
            .collect(),
        shortcut: None,
    }
}

fn fixture_account() -> AccountSnapshot {
    AccountSnapshot {
        public_key_hex: fixture_hex(1),
        npub: "npub1fixtureowner".to_string(),
        display_name: "Fixture User".to_string(),
        picture_url: None,
        about: None,
        device_public_key_hex: fixture_hex(2),
        device_npub: "npub1fixturedevice".to_string(),
        has_owner_signing_authority: true,
        authorization_state: DeviceAuthorizationState::Authorized,
        protocol_readiness: ProtocolReadinessSnapshot::ready(),
    }
}

fn fixture_thread(kind: ChatKind, index: u32) -> ChatThreadSnapshot {
    let is_group = matches!(kind, ChatKind::Group);
    let prefix = if is_group { "group" } else { "direct" };
    let display_prefix = if is_group { "Project Group" } else { "Contact" };
    let member_count = if is_group {
        3 + u64::from(index % 12)
    } else {
        2
    };

    ChatThreadSnapshot {
        chat_id: format!("{prefix}-{:04}", index + 1),
        kind,
        display_name: format!("{display_prefix} {:04}", index + 1),
        nickname: None,
        profile_name: None,
        subtitle: Some(if is_group {
            format!("{member_count} people")
        } else {
            format!("user {}", index + 1)
        }),
        picture_url: None,
        about: None,
        member_count,
        last_message_preview: Some(format!(
            "Fixture preview {} with searchable token needle",
            index + 1
        )),
        last_message_at_secs: Some(BASE_TIME_SECS.saturating_sub(u64::from(index) * 60)),
        last_message_is_outgoing: Some(index % 3 == 0),
        last_message_delivery: Some(match index % 5 {
            0 => DeliveryState::Seen,
            1 => DeliveryState::Sent,
            2 => DeliveryState::Received,
            3 => DeliveryState::Pending,
            _ => DeliveryState::Failed,
        }),
        unread_count: u64::from(index % 4),
        is_typing: index % 17 == 0,
        is_muted: index % 19 == 0,
        is_pinned: index < 3,
        draft: if index % 23 == 0 {
            format!("draft {}", index + 1)
        } else {
            String::new()
        },
        is_request: false,
        protocol_readiness: ProtocolReadinessSnapshot::ready(),
    }
}

fn fixture_message(chat_id: &str, index: u32) -> ChatMessageSnapshot {
    let outgoing = index % 2 == 0;
    ChatMessageSnapshot {
        id: format!("{chat_id}-message-{:05}", index + 1),
        chat_id: chat_id.to_string(),
        kind: if index % 29 == 0 {
            ChatMessageKind::System
        } else {
            ChatMessageKind::User
        },
        author: if outgoing {
            fixture_hex(1)
        } else {
            fixture_hex(10_000 + index)
        },
        author_owner_pubkey_hex: None,
        author_picture_url: None,
        body: format!(
            "Fixture message {:05} for render and search stress with needle token",
            index + 1
        ),
        attachments: Vec::new(),
        reactions: fixture_reactions(index),
        reactors: fixture_reactors(index),
        is_outgoing: outgoing,
        created_at_secs: BASE_TIME_SECS + u64::from(index),
        expires_at_secs: if index % 31 == 0 {
            Some(BASE_TIME_SECS + u64::from(index) + 86_400)
        } else {
            None
        },
        delivery: if outgoing {
            DeliveryState::Seen
        } else {
            DeliveryState::Received
        },
        recipient_deliveries: Vec::new(),
        delivery_trace: MessageDeliveryTraceSnapshot::default(),
        source_event_id: Some(fixture_hex(50_000 + index)),
    }
}

fn fixture_reactions(index: u32) -> Vec<MessageReactionSnapshot> {
    if index % 4 != 0 {
        return Vec::new();
    }
    vec![
        MessageReactionSnapshot {
            emoji: "\u{1f44d}".to_string(),
            count: 1 + u64::from(index % 5),
            reacted_by_me: index % 8 == 0,
        },
        MessageReactionSnapshot {
            emoji: "\u{2764}\u{fe0f}".to_string(),
            count: 1,
            reacted_by_me: false,
        },
    ]
}

fn fixture_reactors(index: u32) -> Vec<MessageReactor> {
    if index % 4 != 0 {
        return Vec::new();
    }
    vec![MessageReactor {
        author: fixture_hex(20_000 + index),
        display_name: String::new(),
        picture_url: None,
        emoji: "\u{1f44d}".to_string(),
    }]
}

fn fixture_search_hit(query: &str, index: u32) -> MessageSearchHit {
    let kind = if index % 5 == 0 {
        ChatKind::Group
    } else {
        ChatKind::Direct
    };
    let chat_id = match &kind {
        ChatKind::Direct => format!("direct-{:04}", (index % 500) + 1),
        ChatKind::Group => format!("group-{:04}", (index % 200) + 1),
    };
    MessageSearchHit {
        chat_id,
        message_id: format!("search-message-{:05}", index + 1),
        chat_display_name: match &kind {
            ChatKind::Direct => format!("Contact {:04}", (index % 500) + 1),
            ChatKind::Group => format!("Project Group {:04}", (index % 200) + 1),
        },
        chat_picture_url: None,
        chat_kind: kind,
        author_pubkey: fixture_hex(30_000 + index),
        body: format!("{query} search fixture message body {:05}", index + 1),
        is_outgoing: index % 2 == 0,
        created_at_secs: BASE_TIME_SECS.saturating_sub(u64::from(index) * 30),
    }
}

fn fixture_group_details(group_count: u32) -> GroupDetailsSnapshot {
    let member_count = group_count.min(32).max(3);
    GroupDetailsSnapshot {
        group_id: "group-0001".to_string(),
        name: "Project Group 0001".to_string(),
        picture_url: None,
        about: None,
        created_by_display_name: "Fixture User".to_string(),
        created_by_npub: "npub1fixtureowner".to_string(),
        can_manage: true,
        is_muted: false,
        revision: 1,
        members: (0..member_count)
            .map(|index| GroupMemberSnapshot {
                owner_pubkey_hex: fixture_hex(40_000 + index),
                display_name: format!("Member {:02}", index + 1),
                npub: format!("npub1fixturemember{:02}", index + 1),
                picture_url: None,
                is_admin: index < 2,
                is_creator: index == 0,
                is_local_owner: index == 0,
            })
            .collect(),
        protocol_readiness: ProtocolReadinessSnapshot::ready(),
    }
}

fn fixture_hex(seed: u32) -> String {
    format!("{seed:064x}")
}

fn title_token(query: &str) -> String {
    let trimmed = query.trim();
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Needle".to_string(),
    }
}
