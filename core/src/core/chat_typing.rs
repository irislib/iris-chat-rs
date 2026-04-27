use super::*;

const TYPING_INDICATOR_TTL_SECS: u64 = 10;

impl AppCore {
    pub(super) fn send_typing(&mut self, chat_id: &str) {
        if !self.preferences.send_typing_indicators {
            return;
        }
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        if is_group_chat_id(&normalized_chat_id) {
            self.send_group_event(&normalized_chat_id, TYPING_KIND, "typing", Vec::new(), None);
        } else if let Ok((_, peer)) = parse_peer_input(&normalized_chat_id) {
            let _ = logged_in.ndr_runtime.send_typing(peer, None);
            self.process_runtime_events();
        }
    }

    pub(super) fn stop_typing(&mut self, chat_id: &str) {
        if !self.preferences.send_typing_indicators {
            return;
        }
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        if is_group_chat_id(&normalized_chat_id) {
            self.send_group_event(
                &normalized_chat_id,
                TYPING_KIND,
                "typing",
                vec![vec!["expiration".to_string(), "1".to_string()]],
                None,
            );
        } else if let Ok((_, peer)) = parse_peer_input(&normalized_chat_id) {
            let _ = logged_in.ndr_runtime.send_typing(
                peer,
                Some(SendOptions {
                    expires_at: Some(1),
                    ttl_seconds: None,
                }),
            );
            self.process_runtime_events();
        }
    }

    pub(super) fn apply_typing_event(
        &mut self,
        chat_id: String,
        author_owner_hex: String,
        event_secs: u64,
        expires_at_secs: Option<u64>,
    ) {
        if expires_at_secs.is_some_and(|expires_at| expires_at <= event_secs) {
            self.clear_typing_indicator(&chat_id, &author_owner_hex);
            return;
        }
        // Don't re-arm an indicator at or before the chat's typing
        // floor. The floor is bumped to the wire-clock timestamp of
        // every message we add to the thread, so a stray typing
        // rumor that races (or a peer that doesn't send a stop-
        // typing event) can't keep the indicator alive after we've
        // already seen the message.
        if let Some(floor) = self.typing_floor_secs.get(&chat_id).copied() {
            if event_secs <= floor {
                self.clear_typing_indicator(&chat_id, &author_owner_hex);
                return;
            }
        }
        self.set_typing_indicator(chat_id, author_owner_hex, event_secs);
    }

    /// Raise the per-chat typing floor to `ts` if it's higher than
    /// what we already have. Called at every message-add site so the
    /// floor tracks the latest message wire-clock timestamp seen for
    /// the chat, monotonically.
    pub(super) fn bump_typing_floor(&mut self, chat_id: &str, ts: u64) {
        let entry = self
            .typing_floor_secs
            .entry(chat_id.to_string())
            .or_insert(0);
        if *entry < ts {
            *entry = ts;
        }
    }

    pub(super) fn set_typing_indicator(
        &mut self,
        chat_id: String,
        author_owner_hex: String,
        event_secs: u64,
    ) {
        let expires_at_secs = unix_now().get().saturating_add(TYPING_INDICATOR_TTL_SECS);
        let key = typing_indicator_key(&chat_id, &author_owner_hex);
        self.typing_indicators.insert(
            key,
            TypingIndicatorRecord {
                chat_id: chat_id.clone(),
                author_owner_hex: author_owner_hex.clone(),
                expires_at_secs,
                last_event_secs: event_secs,
            },
        );
        self.schedule_typing_indicator_expiry(chat_id, author_owner_hex);
    }

    pub(super) fn clear_typing_indicator(&mut self, chat_id: &str, author_owner_hex: &str) {
        self.typing_indicators
            .remove(&typing_indicator_key(chat_id, author_owner_hex));
    }

    pub(super) fn schedule_typing_indicator_expiry(&self, chat_id: String, author: String) {
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(Duration::from_secs(TYPING_INDICATOR_TTL_SECS)).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::TypingIndicatorExpired { chat_id, author },
            )));
        });
    }
}

fn typing_indicator_key(chat_id: &str, author_owner_hex: &str) -> String {
    format!("{chat_id}\n{author_owner_hex}")
}
