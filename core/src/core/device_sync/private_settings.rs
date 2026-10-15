use super::*;
use nostr_identity::{build_private_setting, parse_private_setting};

impl AppCore {
    pub(super) fn private_setting_events(&self) -> Vec<PrivateEvent> {
        let Some(login) = &self.logged_in else {
            return Vec::new();
        };
        let owner = login.owner_pubkey;
        let snapshot = self.build_device_sync_snapshot(0, false);
        let mut events = Vec::new();
        let mut add = |setting: &str, target: &str, value: serde_json::Value| {
            if let Ok(event) = build_private_setting(owner, setting, target, &value) {
                events.push(event);
            }
        };
        for value in snapshot.chat_pins {
            if let Ok(json) = serde_json::to_value(&value) {
                add("chatPin", &value.chat_id, json);
            }
        }
        for value in snapshot.chat_mutes {
            if let Ok(json) = serde_json::to_value(&value) {
                add("chatMute", &value.chat_id, json);
            }
        }
        for value in snapshot.private_device_labels_v2 {
            let Ok(value) = serde_json::to_value(value) else {
                continue;
            };
            let Some(target) = value
                .get("device")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
            else {
                continue;
            };
            add("deviceLabel", &target, value);
        }
        for value in snapshot.chats {
            add(
                "chat",
                &value.id,
                serde_json::json!({"id":value.id,"updatedAt":value.updated_at}),
            );
        }
        for value in snapshot.deleted_chats {
            if let Ok(json) = serde_json::to_value(&value) {
                add("chatDeletion", &value.id, json);
            }
        }
        for target in &self.preferences.accepted_owner_pubkeys {
            if self.request_decision(target).is_none() {
                add(
                    "requestDecision",
                    target,
                    serde_json::json!({"chatId":target,"decision":"accepted","updatedAtMs":1}),
                );
            }
        }
        let _ = self.visit_sync_records(|record| {
            if let DeviceSyncRecord::PrivateEvent { event } = record {
                events.push(event);
            }
            true
        });
        events
    }

    pub(super) fn private_setting_snapshot(
        &self,
        event: &PrivateEvent,
    ) -> Option<DeviceSyncSnapshot> {
        if serde_json::to_vec(event).ok()?.len() > 8192 {
            return None;
        }
        let (setting, target, value) =
            parse_private_setting(event, self.logged_in.as_ref()?.owner_pubkey).ok()?;
        let mut snapshot = DeviceSyncSnapshot::default();
        let future = unix_now().get().saturating_add(300);
        if setting != "deviceLabel" && !valid_device_sync_chat_id(&target) {
            return None;
        }
        match setting.as_str() {
            "chatPin" => {
                let state: super::super::chat_pin_sync::ChatPinState =
                    serde_json::from_value(value).ok()?;
                if state.chat_id != target
                    || state.updated_at_ms == 0
                    || state.updated_at_ms > unix_now_ms().saturating_add(300_000)
                {
                    return None;
                }
                snapshot.chat_pins.push(state);
            }
            "chatMute" => {
                let state: super::super::chat_mute_sync::ChatMuteState =
                    serde_json::from_value(value).ok()?;
                if state.chat_id != target
                    || state.updated_at_ms == 0
                    || state.updated_at_ms > unix_now_ms().saturating_add(300_000)
                    || state.until_secs.is_some_and(|time| time > 253_402_300_799)
                {
                    return None;
                }
                snapshot.chat_mutes.push(state);
            }
            "deviceLabel" => {
                if value.get("device").and_then(|value| value.as_str()) != Some(target.as_str())
                    || value.get("owner").and_then(|value| value.as_str())
                        != Some(event.pubkey.to_hex().as_str())
                {
                    return None;
                }
                let state: super::super::private_device_labels::PrivateDeviceLabel =
                    serde_json::from_value(value).ok()?;
                if !state.valid_for(&event.pubkey.to_hex()) {
                    return None;
                }
                snapshot.private_device_labels_v2.push(state);
            }
            "chat" => {
                let state: DeviceSyncChat = serde_json::from_value(value).ok()?;
                if state.id != target || state.updated_at > future || state.read_state.is_some() {
                    return None;
                }
                snapshot.chats.push(state);
            }
            "chatDeletion" => {
                let state: DeviceSyncChatDeletion = serde_json::from_value(value).ok()?;
                if state.id != target || state.deleted_at == 0 || state.deleted_at > future {
                    return None;
                }
                snapshot.deleted_chats.push(state);
            }
            "requestDecision" => {
                request_decision_state(event)?;
            }
            _ => return None,
        }
        Some(snapshot)
    }

    pub(in crate::core) fn merge_private_setting(&mut self, event: &PrivateEvent) -> bool {
        let Some(mut snapshot) = self.private_setting_snapshot(event) else {
            return false;
        };
        if let Some(state) = request_decision_state(event) {
            match self.store_sync_record(&DeviceSyncRecord::PrivateEvent {
                event: event.clone(),
            }) {
                Err(_) => return false,
                Ok(false) => return true,
                Ok(true) => self.project_request_decision(&state),
            }
        }
        for state in snapshot.chat_pins.drain(..) {
            if self.merge_chat_pin(state).is_err() {
                return false;
            }
        }
        for state in snapshot.chat_mutes.drain(..) {
            if self.merge_chat_mute(state).is_err() {
                return false;
            }
        }
        for deletion in snapshot.deleted_chats.drain(..) {
            self.apply_chat_deletion(&deletion.id, deletion.deleted_at);
            if self.chat_deletions.get(&deletion.id).copied().unwrap_or(0) < deletion.deleted_at {
                return false;
            }
        }
        self.apply_device_sync_snapshot(snapshot, Some(0));
        self.persist_app_state().is_ok()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::core) struct RequestDecision {
    chat_id: String,
    pub(in crate::core) decision: String,
    updated_at_ms: u64,
}
pub(super) fn request_decision_state(event: &PrivateEvent) -> Option<RequestDecision> {
    let (setting, target, value) = parse_private_setting(event, event.pubkey).ok()?;
    let state: RequestDecision = serde_json::from_value(value).ok()?;
    (setting == "requestDecision"
        && state.chat_id == target
        && valid_device_sync_chat_id(&target)
        && ["none", "accepted", "rejected"].contains(&state.decision.as_str())
        && state.updated_at_ms > 0
        && state.updated_at_ms <= unix_now_ms().saturating_add(300_000))
    .then_some(state)
}
pub(super) fn request_decision_version(event: &PrivateEvent) -> Option<(u64, u8)> {
    let state = request_decision_state(event)?;
    Some((
        state.updated_at_ms,
        match state.decision.as_str() {
            "accepted" => 1,
            "rejected" => 2,
            _ => 0,
        },
    ))
}
impl AppCore {
    pub(in crate::core) fn request_decision(&self, target: &str) -> Option<RequestDecision> {
        let key = serde_json::json!(["privateEvent", "requestDecision", target]).to_string();
        match self.load_sync_record(&records::RecordLocator::Head(key))? {
            DeviceSyncRecord::PrivateEvent { event } => request_decision_state(&event),
            _ => None,
        }
    }
    pub(super) fn project_request_decision(&mut self, state: &RequestDecision) {
        self.preferences
            .accepted_owner_pubkeys
            .retain(|target| target != &state.chat_id);
        if state.decision == "accepted" {
            self.preferences
                .accepted_owner_pubkeys
                .push(state.chat_id.clone());
        }
        self.preferences.accepted_owner_pubkeys.sort();
        self.request_protocol_subscription_refresh();
        self.mark_mobile_push_dirty();
    }
    pub(in crate::core) fn sync_request_acceptance(&mut self, target: &str) -> bool {
        let Some(login) = &self.logged_in else {
            return false;
        };
        let state = RequestDecision {
            chat_id: target.into(),
            decision: "accepted".into(),
            updated_at_ms: unix_now_ms().max(
                self.request_decision(target)
                    .map_or(0, |state| state.updated_at_ms + 1),
            ),
        };
        let Ok(value) = serde_json::to_value(state) else {
            return false;
        };
        let Ok(event) =
            build_private_setting(login.owner_pubkey, "requestDecision", target, &value)
        else {
            return false;
        };
        if !self.merge_private_setting(&event) {
            return false;
        }
        self.broadcast_device_sync_snapshot();
        true
    }
}
