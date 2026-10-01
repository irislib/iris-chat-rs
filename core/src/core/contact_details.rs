use super::*;

const MAX_NICKNAME_CHARS: usize = 80;
const MAX_NOTE_CHARS: usize = 240;

// Kept separate from public metadata timestamps. Empty values with a timestamp
// are a removal tombstone, so offline devices cannot restore deleted details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContactDetails {
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub updated_at_ms: u64,
}

impl OwnerProfileRecord {
    pub(super) fn contact_details(&self) -> Option<ContactDetails> {
        (self.contact_updated_at_ms > 0 || self.nickname.is_some() || self.contact_note.is_some())
            .then(|| ContactDetails {
                nickname: self.nickname.clone(),
                note: self.contact_note.clone(),
                // Existing local nicknames predate contact-detail sync.
                updated_at_ms: self.contact_updated_at_ms.max(1),
            })
    }
}

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

    pub(super) fn apply_contact_details(
        &mut self,
        owner_hex: &str,
        incoming: ContactDetails,
    ) -> bool {
        if PublicKey::from_hex(owner_hex).is_err()
            || self
                .logged_in
                .as_ref()
                .is_none_or(|login| login.owner_pubkey.to_hex() == owner_hex)
            || incoming.updated_at_ms == 0
            || incoming.updated_at_ms > crate::perflog::now_ms().saturating_add(300_000)
            || incoming
                .nickname
                .as_ref()
                .is_some_and(|text| text.chars().count() > MAX_NICKNAME_CHARS)
            || incoming
                .note
                .as_ref()
                .is_some_and(|text| text.chars().count() > MAX_NOTE_CHARS)
        {
            return false;
        }
        let profile = self
            .owner_profiles
            .entry(owner_hex.to_string())
            .or_default();
        // Deterministic tie-break for simultaneous offline edits, independent
        // of public profile updates and packet arrival order.
        if let Some(previous) = profile.contact_details() {
            if (incoming.updated_at_ms, &incoming.nickname, &incoming.note)
                <= (previous.updated_at_ms, &previous.nickname, &previous.note)
            {
                return false;
            }
        }
        profile.nickname = incoming.nickname;
        profile.contact_note = incoming.note;
        profile.contact_updated_at_ms = incoming.updated_at_ms;
        self.seed_legacy_private_contact(owner_hex);
        self.mark_mobile_push_dirty();
        true
    }
}
