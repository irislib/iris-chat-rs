use super::*;

const PENDING_PREFIX: &str = "pending-new-chat-notification/";
const MAX_PENDING: usize = 32;

pub(super) fn load_pending_responses(
    storage: &dyn StorageAdapter,
) -> anyhow::Result<BTreeMap<String, Event>> {
    let mut pending = BTreeMap::new();
    for key in storage.list(PENDING_PREFIX)?.into_iter().take(MAX_PENDING) {
        if let Some(json) = storage.get(&key)? {
            if let Ok(event) = serde_json::from_str::<Event>(&json) {
                pending.insert(event.id.to_hex(), event);
            }
        }
    }
    Ok(pending)
}

impl AppCore {
    // Published device invites can be answered before the account's signed
    // device proof arrives. Retain the encrypted response across restarts and
    // announce it only after the protocol accepts the authenticated session.
    // Return an unqueued sender for immediate authorization checking by the
    // caller; queue capacity or a storage failure must not hide proved arrivals.
    pub(super) fn queue_new_chat_response(
        &mut self,
        event: &Event,
    ) -> Option<(PublicKey, PublicKey)> {
        if serde_json::to_vec(event).is_ok_and(|bytes| bytes.len() > 128 * 1024) {
            return None;
        }
        let sender @ (owner, _) = self.new_chat_response_sender(event, false)?;
        if self.threads.contains_key(&owner.to_hex())
            || self
                .logged_in
                .as_ref()
                .is_some_and(|login| login.owner_pubkey == owner)
        {
            return None;
        }
        // Bound only the waiting queue. The caller also checks this freshly
        // authenticated sender after protocol ingestion, when a bundled proof
        // may authorize it without needing a waiting slot.
        if self.pending_new_chat_responses.len() >= MAX_PENDING {
            return Some(sender);
        }
        let save = || -> anyhow::Result<()> {
            self.private_chat_invite_storage()?.put(
                &format!("{PENDING_PREFIX}{}", event.id),
                serde_json::to_string(event)?,
            )?;
            Ok(())
        };
        if save().is_ok() {
            self.pending_new_chat_responses
                .insert(event.id.to_hex(), event.clone());
            None
        } else {
            Some(sender)
        }
    }

    fn new_chat_response_sender(
        &self,
        event: &Event,
        revalidate: bool,
    ) -> Option<(PublicKey, PublicKey)> {
        if event.verify().is_err() || event.kind.as_u16() as u32 != INVITE_RESPONSE_KIND {
            return None;
        }
        let login = self.logged_in.as_ref()?;
        let mut invite = self.protocol_engine.as_ref()?.local_invite()?;
        if revalidate {
            // The queued response was authenticated before session ingestion.
            // Recheck its ciphertext without replaying/importing the session.
            invite.used_response_contents.clear();
        }
        let response = nostr_double_ratchet::process_invite_response_event(
            &invite,
            event,
            login.device_keys.secret_key().to_secret_bytes(),
        )
        .ok()??;
        let owner = match (response.owner_public_key, response.invitee_owner_pubkey) {
            (Some(owner), Some(claim)) if owner.to_bytes() == claim.to_bytes() => owner,
            (None, None) => response.invitee_identity,
            _ => return None,
        };
        Some((owner, response.invitee_identity))
    }

    pub(super) fn finish_pending_new_chat_notifications(&mut self) -> bool {
        if self.pending_new_chat_responses.is_empty() {
            return false;
        }
        let pending = self
            .pending_new_chat_responses
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut completed = Vec::new();
        for event in pending {
            let id = event.id.to_hex();
            if event.created_at.as_secs().saturating_add(7 * 24 * 60 * 60) < unix_now().get() {
                completed.push(id);
                continue;
            }
            let Some((owner, device)) = self.new_chat_response_sender(&event, true) else {
                completed.push(id);
                continue;
            };
            if !self.record_verified_new_chat_response(&event, owner, device) {
                continue;
            }
            completed.push(id);
        }
        if completed.is_empty() {
            return false;
        }
        // Save the new thread before retiring its pending notice. Restored
        // threads never produce a second notification on response replay.
        if self.persist_app_state().is_err() {
            return false;
        }
        if let Ok(storage) = self.private_chat_invite_storage() {
            for id in completed {
                if storage.del(&format!("{PENDING_PREFIX}{id}")).is_ok() {
                    self.pending_new_chat_responses.remove(&id);
                }
            }
        }
        true
    }

    pub(super) fn record_verified_new_chat_response(
        &mut self,
        event: &Event,
        owner: PublicKey,
        device: PublicKey,
    ) -> bool {
        if !self.protocol_engine.as_ref().is_some_and(|engine| {
            engine.invite_owner_device_is_authorized(owner, device)
                && engine.active_session_count_for_owner(owner) > 0
        }) {
            return false;
        }
        self.cache_invite_acceptance_preview(&event.id.to_hex(), &owner.to_hex());
        self.record_new_chat_notification(&owner.to_hex(), &event.id.to_hex());
        true
    }

    pub(super) fn record_new_chat_notification(&mut self, chat_id: &str, event_id: &str) {
        if self.threads.contains_key(chat_id)
            || self
                .logged_in
                .as_ref()
                .is_none_or(|login| login.owner_pubkey.to_hex() == chat_id)
            || self.is_owner_blocked(chat_id)
        {
            return;
        }
        self.ensure_thread_record(chat_id, unix_now().get());
        if self.preferences.desktop_notifications_enabled
            && self.preferences.invite_acceptance_notifications_enabled
            && !self.is_chat_muted(chat_id)
        {
            self.new_chat_notifications
                .insert(chat_id.to_owned(), event_id.to_owned());
        }
    }
}
