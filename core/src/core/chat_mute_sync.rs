use super::*;

// Private inner event kind, carried only by our linked-device ratchets.
pub(super) const CHAT_MUTE_KIND: u32 = 10449;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatMuteState {
    pub chat_id: String,
    // None is an explicit unmute, zero means forever, otherwise Unix seconds.
    pub until_secs: Option<u64>,
    pub updated_at_ms: u64,
}

impl ChatMuteState {
    fn version(&self) -> (u64, Option<u64>) {
        (self.updated_at_ms, self.until_secs)
    }
}

impl AppCore {
    // Legacy preferences have no clock. Give them the lowest revision so they
    // can seed a new device without overwriting a newer mute or unmute.
    pub(super) fn chat_mute_snapshot(&self) -> Vec<ChatMuteState> {
        let mut states = self.chat_mute_states.clone();
        for (chat_id, until_secs) in self
            .preferences
            .muted_chat_ids
            .iter()
            .map(|id| (id, 0))
            .chain(
                self.preferences
                    .timed_chat_mutes
                    .iter()
                    .map(|m| (&m.chat_id, m.until_secs)),
            )
        {
            states
                .entry(chat_id.clone())
                .or_insert_with(|| ChatMuteState {
                    chat_id: chat_id.clone(),
                    until_secs: Some(until_secs),
                    updated_at_ms: 1,
                });
        }
        states.into_values().collect()
    }

    pub(super) fn restore_chat_mute_projection(&mut self) {
        self.chat_mute_states = self
            .chat_mute_snapshot()
            .into_iter()
            .map(|state| (state.chat_id.clone(), state))
            .collect();
        for state in self.chat_mute_states.values().cloned().collect::<Vec<_>>() {
            self.project_chat_mute(&state);
        }
    }

    fn project_chat_mute(&mut self, state: &ChatMuteState) {
        self.preferences
            .muted_chat_ids
            .retain(|id| id != &state.chat_id);
        self.preferences
            .timed_chat_mutes
            .retain(|mute| mute.chat_id != state.chat_id);
        match state.until_secs {
            Some(0) => self.preferences.muted_chat_ids.push(state.chat_id.clone()),
            Some(until_secs) if until_secs > unix_now().get() => {
                self.preferences.timed_chat_mutes.push(ChatMuteDeadline {
                    chat_id: state.chat_id.clone(),
                    until_secs,
                });
            }
            _ => {}
        }
        self.preferences.muted_chat_ids.sort();
        self.preferences
            .timed_chat_mutes
            .sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
    }

    pub(super) fn merge_chat_mute(&mut self, state: ChatMuteState) -> anyhow::Result<bool> {
        if self
            .normalize_local_chat_setting_id(&state.chat_id)
            .as_deref()
            != Some(&state.chat_id)
            || state.chat_id.len() > 134
            || state
                .chat_id
                .chars()
                .any(|c| c.is_ascii_control() || c == ' ')
            || state.updated_at_ms == 0
            || state.updated_at_ms > unix_now_ms().saturating_add(300_000)
            || state
                .until_secs
                .is_some_and(|until| until > 253_402_300_799)
            || self
                .chat_mute_states
                .get(&state.chat_id)
                .is_some_and(|old| old.version() >= state.version())
        {
            return Ok(false);
        }
        // Persist the authoritative value first. Login replays this projection
        // even if the preferences snapshot was interrupted by a crash.
        self.app_store.save_chat_mute_state(&state)?;
        self.project_chat_mute(&state);
        self.chat_mute_states.insert(state.chat_id.clone(), state);
        self.schedule_chat_mute_expiry();
        self.mark_mobile_push_dirty();
        Ok(true)
    }

    pub(super) fn set_synced_chat_mute(&mut self, chat_id: &str, until_secs: Option<u64>) {
        let Some(chat_id) = self.normalize_local_chat_setting_id(chat_id) else {
            return;
        };
        let updated_at_ms = unix_now_ms().max(
            self.chat_mute_states
                .get(&chat_id)
                .map_or(1, |old| old.updated_at_ms.saturating_add(1)),
        );
        let state = ChatMuteState {
            chat_id,
            until_secs,
            updated_at_ms,
        };
        match self.merge_chat_mute(state.clone()) {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                self.push_debug_log("chat_mute.save_failed", error.to_string());
                self.state.toast = Some("Could not save notification setting. Try again.".into());
                self.emit_state();
                return;
            }
        }
        if let Some(owner) = self.logged_in.as_ref().map(|account| account.owner_pubkey) {
            let content =
                serde_json::json!({ "type": "chat-mute", "v": 1, "mute": state }).to_string();
            let unsigned =
                nostr::EventBuilder::new(nostr::Kind::Custom(CHAT_MUTE_KIND as u16), content)
                    .tag(nostr::Tag::public_key(owner))
                    .allow_self_tagging()
                    .build(owner);
            self.send_protocol_engine_unsigned_event_to_local_siblings(
                owner,
                &owner.to_hex(),
                unsigned,
                "chat_mute.self_sync",
            );
        }
        self.broadcast_device_sync_snapshot();
        self.rebuild_persist_and_emit_state();
    }

    pub(super) fn receive_chat_mute_control(
        &mut self,
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        content: &str,
    ) -> bool {
        let Some(account) = self.logged_in.as_ref() else {
            return true;
        };
        if sender_owner != account.owner_pubkey
            || !sender_device
                .is_some_and(|device| self.device_sync_peer_is_authorized(&device.to_hex()))
        {
            return true;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
            return true;
        };
        if value.get("type").and_then(serde_json::Value::as_str) != Some("chat-mute")
            || value.get("v").and_then(serde_json::Value::as_u64) != Some(1)
        {
            return true;
        }
        let Some(payload) = value.get("mute") else {
            return true;
        };
        let Ok(state) = serde_json::from_value::<ChatMuteState>(payload.clone()) else {
            return true;
        };
        match self.merge_chat_mute(state) {
            Ok(true) => {
                self.rebuild_persist_and_emit_state();
                true
            }
            Ok(false) => true,
            Err(error) => {
                self.push_debug_log("chat_mute.save_failed", error.to_string());
                false // Keep the decrypted delivery journal for retry.
            }
        }
    }
}
