use super::*;

const MAX_NICKNAME_CHARS: usize = 80;
const MAX_NOTE_CHARS: usize = 240;

impl AppCore {
    pub(super) fn set_contact_nickname(&mut self, owner_pubkey_hex: &str, nickname: &str) {
        let note = PublicKey::parse(owner_pubkey_hex.trim())
            .ok()
            .and_then(|owner| self.owner_profiles.get(&owner.to_hex()))
            .and_then(|profile| profile.contact_note.clone())
            .unwrap_or_default();
        self.set_contact_details(owner_pubkey_hex, nickname, &note);
    }

    pub(super) fn set_contact_details(
        &mut self,
        owner_pubkey_hex: &str,
        nickname: &str,
        note: &str,
    ) {
        let Ok(owner) = PublicKey::parse(owner_pubkey_hex.trim()) else {
            self.state.toast = Some("User ID is invalid.".to_string());
            self.emit_state();
            return;
        };
        if self
            .logged_in
            .as_ref()
            .is_none_or(|login| login.owner_pubkey == owner)
        {
            self.state.toast = Some("Choose another person.".to_string());
            self.emit_state();
            return;
        }
        let owner_hex = owner.to_hex();
        if !self.threads.contains_key(&owner_hex) {
            self.state.toast = Some("Chat was not found.".to_string());
            self.emit_state();
            return;
        }
        let nickname = normalize_profile_field(Some(
            nickname.split_whitespace().collect::<Vec<_>>().join(" "),
        ));
        let note = normalize_profile_field(Some(note.replace("\r\n", "\n").replace('\r', "\n")));
        let previous = self.owner_profiles.get(&owner_hex);
        if nickname.as_ref().is_some_and(|text| {
            text.chars().count() > MAX_NICKNAME_CHARS
                && previous.is_none_or(|profile| profile.nickname != nickname)
        }) || note.as_ref().is_some_and(|text| {
            text.chars().count() > MAX_NOTE_CHARS
                && previous.is_none_or(|profile| profile.contact_note != note)
        }) {
            self.state.toast =
                Some("Use up to 80 characters for a nickname and 240 for a note.".to_string());
            self.emit_state();
            return;
        }
        let mut patch = BTreeMap::new();
        if previous.and_then(|profile| profile.nickname.as_ref()) != nickname.as_ref() {
            patch.insert("nickname".into(), serde_json::json!(nickname));
        }
        if previous.and_then(|profile| profile.contact_note.as_ref()) != note.as_ref() {
            patch.insert("note".into(), serde_json::json!(note));
        }
        if !patch.is_empty() {
            if !self.edit_private_contact_fields(&owner_hex, patch) {
                return;
            }
            let profile = self.owner_profiles.entry(owner_hex.clone()).or_default();
            profile.contact_updated_at_ms =
                crate::perflog::now_ms().max(profile.contact_updated_at_ms.saturating_add(1));
            self.persist_best_effort();
            self.mark_mobile_push_dirty();
            self.broadcast_device_sync_snapshot();
            self.rebuild_state();
        }
        self.state.toast = Some("Nickname and note saved".to_string());
        self.emit_state();
    }
}
