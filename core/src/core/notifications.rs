//! Single source of truth for "should this chat raise a desktop / shell
//! notification?". All non-iOS-APNS callers (foreground macOS, Linux GTK,
//! Windows WPF, Android in-app) compare two consecutive AppState snapshots
//! and call [`decide_notifications`] to get the resulting candidate list.
//!
//! iOS APNS keeps its own minimal suppression path
//! (`mobile_push::decrypt_mobile_push_notification`) because background
//! Notification Service Extensions cannot reach the live AppState and,
//! more importantly, until we ship the
//! `com.apple.developer.usernotifications.filtering` entitlement, Apple
//! requires every delivered push to surface to the user. Do not route
//! iOS APNS through this module.

use std::collections::HashMap;

use crate::state::{ChatThreadSnapshot, PreferencesSnapshot, Router, Screen};

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct NotificationCandidate {
    pub chat_id: String,
    pub title: String,
    pub body: String,
}

/// Decide which chats should raise a notification given the previous and
/// next snapshots of the chat list. Suppression rules — all must pass:
///
/// 1. `preferences.desktop_notifications_enabled` is true.
/// 2. The chat is not muted (`chat.is_muted`).
/// 3. The last message is known-incoming (`last_message_is_outgoing == Some(false)`).
///    Unknown direction (`None`) is treated as suppressed: we'd rather miss
///    a banner than show one for our own outgoing message.
/// 4. The chat is not currently open with the app foregrounded.
/// 5. `unread_count` strictly increased relative to the previous snapshot
///    (chats absent from the previous snapshot count as previous = 0).
pub fn decide_notifications(
    previous_chats: &[ChatThreadSnapshot],
    next_chats: &[ChatThreadSnapshot],
    preferences: &PreferencesSnapshot,
    app_foreground: bool,
    open_chat_id: Option<&str>,
) -> Vec<NotificationCandidate> {
    if !preferences.desktop_notifications_enabled {
        return Vec::new();
    }

    let previous_unread: HashMap<&str, u64> = previous_chats
        .iter()
        .map(|c| (c.chat_id.as_str(), c.unread_count))
        .collect();

    let suppressing_open_chat = if app_foreground {
        open_chat_id
    } else {
        None
    };

    let mut out = Vec::new();
    for chat in next_chats {
        if chat.is_muted {
            continue;
        }
        if chat.last_message_is_outgoing != Some(false) {
            continue;
        }
        if suppressing_open_chat == Some(chat.chat_id.as_str()) {
            continue;
        }
        let previous = previous_unread.get(chat.chat_id.as_str()).copied().unwrap_or(0);
        if chat.unread_count <= previous {
            continue;
        }
        let body = chat
            .last_message_preview
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "New message".to_string());
        out.push(NotificationCandidate {
            chat_id: chat.chat_id.clone(),
            title: chat.display_name.clone(),
            body,
        });
    }
    out
}

/// Pull the topmost chat id out of a router stack, falling back to the
/// default screen. Mirrors what every shell does inline so they don't
/// each have to reach into `Screen::Chat` themselves.
pub fn active_chat_id(router: &Router) -> Option<String> {
    if let Some(id) = router
        .screen_stack
        .iter()
        .rev()
        .find_map(screen_chat_id)
    {
        return Some(id);
    }
    screen_chat_id(&router.default_screen)
}

fn screen_chat_id(screen: &Screen) -> Option<String> {
    match screen {
        Screen::Chat { chat_id } => Some(chat_id.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ChatKind;

    fn chat(
        id: &str,
        unread: u64,
        last_is_outgoing: Option<bool>,
        muted: bool,
        preview: Option<&str>,
    ) -> ChatThreadSnapshot {
        ChatThreadSnapshot {
            chat_id: id.to_string(),
            kind: ChatKind::Direct,
            display_name: format!("name-{id}"),
            nickname: None,
            profile_name: None,
            subtitle: None,
            picture_url: None,
            about: None,
            member_count: 0,
            last_message_preview: preview.map(str::to_string),
            last_message_at_secs: Some(100),
            last_message_is_outgoing: last_is_outgoing,
            last_message_delivery: None,
            unread_count: unread,
            is_typing: false,
            is_muted: muted,
            is_pinned: false,
            draft: String::new(),
            is_request: false,
        }
    }

    fn prefs(enabled: bool) -> PreferencesSnapshot {
        let mut p = PreferencesSnapshot::default();
        p.desktop_notifications_enabled = enabled;
        p
    }

    #[test]
    fn fires_when_unread_strictly_increases() {
        let prev = vec![chat("a", 0, Some(false), false, Some("hi"))];
        let next = vec![chat("a", 1, Some(false), false, Some("hi"))];
        let out = decide_notifications(&prev, &next, &prefs(true), false, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].chat_id, "a");
        assert_eq!(out[0].body, "hi");
    }

    #[test]
    fn falls_back_to_new_message_when_preview_empty() {
        let prev = vec![chat("a", 0, Some(false), false, None)];
        let next = vec![chat("a", 1, Some(false), false, Some("   "))];
        let out = decide_notifications(&prev, &next, &prefs(true), false, None);
        assert_eq!(out[0].body, "New message");
    }

    #[test]
    fn suppresses_muted_chat() {
        let prev = vec![chat("a", 0, Some(false), true, Some("hi"))];
        let next = vec![chat("a", 1, Some(false), true, Some("hi"))];
        let out = decide_notifications(&prev, &next, &prefs(true), false, None);
        assert!(out.is_empty());
    }

    #[test]
    fn suppresses_outgoing_message() {
        let prev = vec![chat("a", 0, Some(true), false, Some("hi"))];
        let next = vec![chat("a", 1, Some(true), false, Some("hi"))];
        let out = decide_notifications(&prev, &next, &prefs(true), false, None);
        assert!(out.is_empty());
    }

    #[test]
    fn suppresses_unknown_direction_message() {
        let prev = vec![chat("a", 0, None, false, Some("hi"))];
        let next = vec![chat("a", 1, None, false, Some("hi"))];
        let out = decide_notifications(&prev, &next, &prefs(true), false, None);
        assert!(out.is_empty());
    }

    #[test]
    fn suppresses_when_chat_open_and_app_foreground() {
        let prev = vec![chat("a", 0, Some(false), false, Some("hi"))];
        let next = vec![chat("a", 1, Some(false), false, Some("hi"))];
        let out = decide_notifications(&prev, &next, &prefs(true), true, Some("a"));
        assert!(out.is_empty());
    }

    #[test]
    fn fires_when_chat_open_but_app_backgrounded() {
        let prev = vec![chat("a", 0, Some(false), false, Some("hi"))];
        let next = vec![chat("a", 1, Some(false), false, Some("hi"))];
        let out = decide_notifications(&prev, &next, &prefs(true), false, Some("a"));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn fires_when_app_foreground_but_a_different_chat_open() {
        let prev = vec![chat("a", 0, Some(false), false, Some("hi"))];
        let next = vec![chat("a", 1, Some(false), false, Some("hi"))];
        let out = decide_notifications(&prev, &next, &prefs(true), true, Some("b"));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn suppresses_when_unread_unchanged_or_decreased() {
        let prev = vec![chat("a", 2, Some(false), false, Some("hi"))];
        let next = vec![chat("a", 1, Some(false), false, Some("hi"))];
        let out = decide_notifications(&prev, &next, &prefs(true), false, None);
        assert!(out.is_empty());
    }

    #[test]
    fn returns_empty_when_preference_disabled() {
        let prev = vec![chat("a", 0, Some(false), false, Some("hi"))];
        let next = vec![chat("a", 1, Some(false), false, Some("hi"))];
        let out = decide_notifications(&prev, &next, &prefs(false), false, None);
        assert!(out.is_empty());
    }

    #[test]
    fn treats_new_chat_as_previous_unread_zero() {
        let prev: Vec<ChatThreadSnapshot> = Vec::new();
        let next = vec![chat("a", 1, Some(false), false, Some("hello"))];
        let out = decide_notifications(&prev, &next, &prefs(true), false, None);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn active_chat_id_returns_topmost_chat_screen() {
        let router = Router {
            default_screen: Screen::ChatList,
            screen_stack: vec![Screen::Chat {
                chat_id: "x".to_string(),
            }],
        };
        assert_eq!(active_chat_id(&router), Some("x".to_string()));
    }

    #[test]
    fn active_chat_id_falls_back_to_default_screen() {
        let router = Router {
            default_screen: Screen::Chat {
                chat_id: "default-x".to_string(),
            },
            screen_stack: vec![],
        };
        assert_eq!(active_chat_id(&router), Some("default-x".to_string()));
    }

    #[test]
    fn active_chat_id_none_when_not_on_a_chat_screen() {
        let router = Router {
            default_screen: Screen::ChatList,
            screen_stack: vec![Screen::Settings],
        };
        assert_eq!(active_chat_id(&router), None);
    }
}
