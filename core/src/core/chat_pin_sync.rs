use super::*;

pub(super) const CHAT_PIN_KIND: u32 = 10450;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatPinState {
    pub chat_id: String,
    pub pinned: bool,
    pub updated_at_ms: u64,
}

impl ChatPinState {
    fn version(&self) -> (u64, bool) {
        (self.updated_at_ms, self.pinned)
    }
}

impl AppCore {
    pub(super) fn chat_pin_snapshot(&self) -> Vec<ChatPinState> {
        let mut states = self.chat_pin_states.clone();
        for chat_id in &self.preferences.pinned_chat_ids {
            states
                .entry(chat_id.clone())
                .or_insert_with(|| ChatPinState {
                    chat_id: chat_id.clone(),
                    pinned: true,
                    updated_at_ms: 1,
                });
        }
        states.into_values().collect()
    }

    pub(super) fn restore_chat_pin_projection(&mut self) {
        self.chat_pin_states = self
            .chat_pin_snapshot()
            .into_iter()
            .map(|state| (state.chat_id.clone(), state))
            .collect();
        for state in self.chat_pin_states.values().cloned().collect::<Vec<_>>() {
            self.project_chat_pin(&state);
        }
    }

    fn project_chat_pin(&mut self, state: &ChatPinState) {
        self.preferences
            .pinned_chat_ids
            .retain(|id| id != &state.chat_id);
        if state.pinned {
            self.preferences.pinned_chat_ids.push(state.chat_id.clone());
        }
        self.preferences.pinned_chat_ids.sort();
        self.preferences.pinned_chat_ids.dedup();
    }

    pub(super) fn merge_chat_pin(&mut self, state: ChatPinState) -> anyhow::Result<bool> {
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
            || self
                .chat_pin_states
                .get(&state.chat_id)
                .is_some_and(|old| old.version() >= state.version())
        {
            return Ok(false);
        }
        self.app_store.save_chat_pin_state(&state)?;
        self.project_chat_pin(&state);
        self.chat_pin_states.insert(state.chat_id.clone(), state);
        Ok(true)
    }

    pub(super) fn set_chat_pinned(&mut self, chat_id: &str, pinned: bool) {
        let Some(chat_id) = self.normalize_local_chat_setting_id(chat_id) else {
            return;
        };
        let updated_at_ms = unix_now_ms().max(
            self.chat_pin_states
                .get(&chat_id)
                .map_or(1, |old| old.updated_at_ms.saturating_add(1)),
        );
        let state = ChatPinState {
            chat_id,
            pinned,
            updated_at_ms,
        };
        match self.merge_chat_pin(state.clone()) {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                self.push_debug_log("chat_pin.save_failed", error.to_string());
                self.state.toast = Some("Could not save pinned chat. Try again.".into());
                self.emit_state();
                return;
            }
        }
        if let Some(owner) = self.logged_in.as_ref().map(|account| account.owner_pubkey) {
            let content =
                serde_json::json!({ "type": "chat-pin", "v": 1, "pin": state }).to_string();
            let unsigned =
                nostr::EventBuilder::new(nostr::Kind::Custom(CHAT_PIN_KIND as u16), content)
                    .tag(nostr::Tag::public_key(owner))
                    .allow_self_tagging()
                    .build(owner);
            self.send_protocol_engine_unsigned_event_to_local_siblings(
                owner,
                &owner.to_hex(),
                unsigned,
                "chat_pin.self_sync",
            );
        }
        self.broadcast_device_sync_snapshot();
        self.rebuild_persist_and_emit_state();
    }

    pub(super) fn receive_chat_pin_control(
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
        if value.get("type").and_then(serde_json::Value::as_str) != Some("chat-pin")
            || value.get("v").and_then(serde_json::Value::as_u64) != Some(1)
        {
            return true;
        }
        let Some(payload) = value.get("pin") else {
            return true;
        };
        let Ok(state) = serde_json::from_value::<ChatPinState>(payload.clone()) else {
            return true;
        };
        match self.merge_chat_pin(state) {
            Ok(true) => {
                self.rebuild_persist_and_emit_state();
                true
            }
            Ok(false) => true,
            Err(error) => {
                self.push_debug_log("chat_pin.save_failed", error.to_string());
                false
            }
        }
    }
}
