use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::core) struct MentionNotificationSetting {
    chat_id: String,
    enabled: bool,
    updated_at_ms: u64,
}

pub(in crate::core) fn state(event: &PrivateEvent) -> Option<MentionNotificationSetting> {
    let (setting, target, value) = parse_private_setting(event, event.pubkey).ok()?;
    let state: MentionNotificationSetting = serde_json::from_value(value).ok()?;
    (setting == "mentionNotifications"
        && state.chat_id == target
        && target.starts_with(GROUP_CHAT_PREFIX)
        && valid_device_sync_chat_id(&target)
        && state.updated_at_ms > 0
        && state.updated_at_ms <= unix_now_ms().saturating_add(300_000))
    .then_some(state)
}

pub(super) fn version(event: &PrivateEvent) -> Option<(u64, u8)> {
    let state = state(event)?;
    // If two devices change the same millisecond, the quieter choice wins.
    Some((state.updated_at_ms, u8::from(!state.enabled)))
}

impl AppCore {
    pub(in crate::core) fn project_mention_notifications(
        &mut self,
        state: &MentionNotificationSetting,
    ) {
        self.preferences
            .silent_mention_chat_ids
            .retain(|id| id != &state.chat_id);
        if !state.enabled {
            self.preferences
                .silent_mention_chat_ids
                .push(state.chat_id.clone());
        }
        self.preferences.silent_mention_chat_ids.sort();
        self.mark_mobile_push_dirty();
    }

    pub(in crate::core) fn set_chat_mention_notifications(&mut self, target: &str, enabled: bool) {
        let Some(target) = self
            .normalize_local_chat_setting_id(target)
            .filter(|id| id.starts_with(GROUP_CHAT_PREFIX))
        else {
            return;
        };
        let Some(owner) = self.logged_in.as_ref().map(|login| login.owner_pubkey) else {
            return;
        };
        let key = serde_json::json!(["privateEvent", "mentionNotifications", target]).to_string();
        let previous = match self.load_sync_record(&records::RecordLocator::Head(key)) {
            Some(DeviceSyncRecord::PrivateEvent { event }) => state(&event),
            _ => None,
        };
        let state = MentionNotificationSetting {
            chat_id: target.clone(),
            enabled,
            updated_at_ms: unix_now_ms()
                .max(previous.map_or(0, |state| state.updated_at_ms.saturating_add(1))),
        };
        let event = serde_json::to_value(state).ok().and_then(|value| {
            build_private_setting(owner, "mentionNotifications", &target, &value).ok()
        });
        if !event
            .as_ref()
            .is_some_and(|event| self.merge_private_setting(event))
        {
            self.state.toast = Some("Could not save notification setting. Try again.".into());
            self.emit_state();
            return;
        }
        self.broadcast_device_sync_snapshot();
        self.rebuild_persist_and_emit_state();
    }
}
