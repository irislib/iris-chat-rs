use super::*;
use crate::state::ContactIdentitySnapshot;

impl AppCore {
    fn is_contact(&self, owner: &str) -> bool {
        PublicKey::from_hex(owner).is_ok()
            && self
                .logged_in
                .as_ref()
                .is_some_and(|login| login.owner_pubkey.to_hex() != owner)
            && self.threads.contains_key(owner)
    }

    pub(super) fn remember_contact_name(&mut self, owner: &str) {
        if !self.is_contact(owner) {
            return;
        }
        if let Some(profile) = self.owner_profiles.get_mut(owner) {
            if let Some(name) = profile.profile_label() {
                profile.contact_memory.observe_name(&name);
            }
        }
    }

    pub(super) fn contact_identity(&self, owner: &str) -> Option<ContactIdentitySnapshot> {
        if !self.is_contact(owner) {
            return None;
        }
        let profile = self.owner_profiles.get(owner).cloned().unwrap_or_default();
        let pending_name = profile.contact_memory.pending_name(profile.profile_label());
        Some(ContactIdentitySnapshot {
            is_following: self
                .social_graph
                .as_ref()
                .is_some_and(|graph| graph.is_following(graph.get_root(), owner)),
            can_follow: self
                .logged_in
                .as_ref()
                .is_some_and(|login| login.owner_keys.is_some() && !login.relay_urls.is_empty()),
            updating_follow: self.pending_follow.is_some(),
            first_seen_name: profile.contact_memory.first_seen_name,
            saved_name: profile.contact_memory.accepted_name,
            pending_name,
            is_favorite: profile.contact_memory.favorite,
        })
    }

    pub(super) fn approve_contact_name(&mut self, owner: &str, name: &str) {
        let Some(identity) = self.contact_identity(owner) else {
            return;
        };
        // Approval is for the exact name shown. A newer profile update must be reviewed again.
        if identity.pending_name.as_deref() != Some(name) {
            return;
        }
        let now = unix_now().get();
        let Some(change) = self.owner_profiles.get_mut(owner).and_then(|profile| {
            let latest = profile.profile_label()?;
            profile.contact_memory.approve_name(name, &latest, now)
        }) else {
            return;
        };
        self.push_system_notice(
            owner,
            format!(
                "Name change approved: {} → {}",
                change.previous_name, change.accepted_name
            ),
            now,
        );
        self.mark_mobile_push_dirty();
        self.bump_user_discovery_revision();
        self.rebuild_persist_and_emit_state();
    }

    pub(super) fn set_contact_favorite(&mut self, owner: &str, favorite: bool) {
        if !self.is_contact(owner) {
            return;
        }
        let profile = self.owner_profiles.entry(owner.to_string()).or_default();
        if profile.contact_memory.favorite == favorite {
            return;
        }
        profile.contact_memory.favorite = favorite;
        self.rebuild_persist_and_emit_state();
    }
}

// Read-only route/CLI snapshots must expose contact memory without opening a chat
// (which would clear unread messages). The shared database is scoped to the account.
pub(super) fn contact_identity_from_db(
    state: &AppState,
    shared: Option<&SharedConnection>,
    thread: Option<&ChatThreadSnapshot>,
) -> Option<ContactIdentitySnapshot> {
    use rusqlite::OptionalExtension;
    let thread = thread.filter(|thread| matches!(thread.kind, ChatKind::Direct))?;
    let account = state.account.as_ref()?;
    let owner = &thread.chat_id;
    if account.public_key_hex == *owner || PublicKey::from_hex(owner).is_err() {
        return None;
    }
    let conn = shared?.try_lock().ok()?;
    let raw = conn
        .query_row(
            "SELECT contact_memory_json FROM owner_profiles WHERE owner_pubkey_hex = ?1",
            [owner],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()?
        .unwrap_or_else(|| "{}".into());
    let memory: crate::contact_memory::ContactMemory = serde_json::from_str(&raw).ok()?;
    let is_following = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_discovery_users u JOIN user_discovery_state s ON s.id = 1
         WHERE u.owner_pubkey_hex = ?1 AND s.owner_pubkey_hex = ?2)",
        rusqlite::params![owner, account.public_key_hex], |row| row.get(0),
    ).ok()?;
    Some(ContactIdentitySnapshot {
        is_following,
        can_follow: account.has_owner_signing_authority
            && !state.preferences.nostr_relay_urls.is_empty(),
        updating_follow: false,
        pending_name: memory.pending_name(thread.profile_name.clone()),
        first_seen_name: memory.first_seen_name,
        saved_name: memory.accepted_name,
        is_favorite: memory.favorite,
    })
}
