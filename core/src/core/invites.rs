use super::*;

mod input;
pub(super) use input::chat_invite_url;
#[cfg(test)]
pub(super) use input::parse_public_invite_input;
use input::{parse_public_invite_or_direct_chat_input, PublicInviteInput};

const PRIVATE_CHAT_INVITE_KEY_PREFIX: &str = "private-chat-invites/";
pub(super) const PENDING_PRIVATE_INVITE_RESPONSE_KEY_PREFIX: &str =
    "pending-private-invite-responses/";
pub(super) const PENDING_PRIVATE_INVITE_RESPONSE_LIMIT: usize = 32;
pub(super) const PENDING_PRIVATE_INVITE_RESPONSE_PER_INVITE_LIMIT: usize = 4;
pub(super) const PENDING_PRIVATE_INVITE_RESPONSE_MAX_BYTES: usize = 128 * 1024;
pub(super) const PENDING_PRIVATE_INVITE_RESPONSE_TTL_SECS: u64 = 7 * 24 * 60 * 60;
const PENDING_OUTGOING_INVITE_ACCEPTANCE_TTL_SECS: u64 = 5 * 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PendingPrivateInviteResponseV1 {
    pub(super) invite_key: String,
    pub(super) encrypted_event: Event,
    pub(super) claimed_owner: PublicKey,
    pub(super) authenticated_device: PublicKey,
    pub(super) queued_at_secs: u64,
}

#[derive(Clone, Debug)]
pub(super) struct PendingOutgoingInviteAcceptance {
    pub(super) invite: Invite,
    pub(super) record: Option<crate::invite_record::InviteRecord>,
    pub(super) claimed_owner: PublicKey,
    pub(super) queued_at_secs: u64,
}

impl AppCore {
    pub(super) fn create_public_invite(&mut self) {
        if !self.can_use_chats() {
            self.state.toast = Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
            self.emit_state();
            return;
        }

        self.state.busy.creating_invite = true;
        self.emit_state();

        let result = (|| -> anyhow::Result<Invite> {
            let logged_in = self
                .logged_in
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Create or restore a profile first."))?;
            let device_pubkey = logged_in.device_keys.public_key();
            let device_id = device_pubkey.to_hex();
            let mut invite = Invite::create_new(device_pubkey, Some(device_id), Some(1))?;
            invite.owner_public_key = Some(logged_in.owner_pubkey);
            invite.purpose = Some("private".to_string());
            Ok(invite)
        })();

        match result {
            Ok(invite) => {
                self.ensure_local_app_keys_for_public_invite();
                if let Err(error) = self.store_private_chat_invite(&invite) {
                    self.state.toast = Some(error.to_string());
                } else {
                    self.private_chat_invites
                        .insert(private_chat_invite_key(&invite), invite);
                    self.mark_mobile_push_dirty();
                    self.request_protocol_subscription_refresh();
                    self.persist_best_effort();
                }
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
            }
        }

        self.state.busy.creating_invite = false;
        self.rebuild_state();
        self.emit_state();
    }

    fn ensure_local_app_keys_for_public_invite(&mut self) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        if logged_in.owner_keys.is_none() {
            return;
        }

        let owner_pubkey = logged_in.owner_pubkey;
        if self.app_keys.contains_key(&owner_pubkey.to_hex()) {
            return;
        }

        let device_pubkey = logged_in.device_keys.public_key();
        let current_device_labels = self.current_device_labels.clone();
        if self.upsert_local_app_key_device_with_labels(
            owner_pubkey,
            device_pubkey,
            current_device_labels.as_ref(),
            true,
        ) {
            self.defer_owner_app_keys_publish = false;
            self.publish_local_app_keys_snapshot_only("public_invite_app_keys");
        }
    }

    fn private_chat_invite_storage(&self) -> anyhow::Result<SqliteStorageAdapter> {
        let logged_in = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Create or restore a profile first."))?;
        Ok(SqliteStorageAdapter::new(
            self.app_store.shared(),
            logged_in.owner_pubkey.to_hex(),
            logged_in.device_keys.public_key().to_hex(),
        ))
    }

    fn store_private_chat_invite(&self, invite: &Invite) -> anyhow::Result<()> {
        let storage = self.private_chat_invite_storage()?;
        storage.put(&private_chat_invite_key(invite), invite.serialize()?)?;
        Ok(())
    }

    fn consume_private_chat_invite_after_response(
        &mut self,
        invite_key: &str,
    ) -> anyhow::Result<()> {
        let storage = self.private_chat_invite_storage()?;
        // The session import receipt is already durable. Make one-use invite
        // consumption durable before removing retry state or exposing effects.
        storage.del(invite_key)?;
        self.private_chat_invites.remove(invite_key);

        let pending_response_ids = self
            .pending_private_invite_responses
            .iter()
            .filter(|(_, pending)| pending.invite_key == invite_key)
            .map(|(event_id, _)| event_id.clone())
            .collect::<Vec<_>>();
        for event_id in pending_response_ids {
            self.pending_private_invite_responses.remove(&event_id);
            let _ = storage.del(&format!(
                "{PENDING_PRIVATE_INVITE_RESPONSE_KEY_PREFIX}{event_id}"
            ));
        }
        self.mark_mobile_push_dirty();
        self.request_protocol_subscription_refresh();
        Ok(())
    }

    pub(super) fn private_chat_invite_response_pubkeys(&self) -> Vec<PublicKey> {
        let mut pubkeys = self
            .private_chat_invites
            .values()
            .map(|invite| invite.inviter_ephemeral_public_key.to_nostr())
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        pubkeys.sort_by_key(|pubkey| pubkey.to_hex());
        pubkeys.dedup();
        pubkeys
    }

    pub(super) fn handle_private_chat_invite_response(
        &mut self,
        event: &Event,
    ) -> PrivateInviteResponseDisposition {
        if event.kind.as_u16() as u32 != INVITE_RESPONSE_KIND {
            return PrivateInviteResponseDisposition::NotMatched;
        }
        let Some(logged_in) = self.logged_in.as_ref() else {
            return PrivateInviteResponseDisposition::NotMatched;
        };
        let device_secret = logged_in.device_keys.secret_key().to_secret_bytes();
        let event_id = event.id.to_string();
        let mut matched = None;
        let invite_entries = self
            .private_chat_invites
            .iter()
            .map(|(key, invite)| (key.clone(), invite.clone()))
            .collect::<Vec<_>>();
        for (key, invite) in invite_entries {
            match nostr_double_ratchet::process_invite_response_event(&invite, event, device_secret)
            {
                Ok(Some(response)) => {
                    matched = Some((key.clone(), response));
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    self.push_debug_log(
                        "invite.private_response.error",
                        format!("event_id={event_id} invite_key={key} error={error}"),
                    );
                }
            }
        }
        let Some((invite_key, response)) = matched else {
            return PrivateInviteResponseDisposition::NotMatched;
        };

        let (owner_pubkey, owner_claim_needs_roster) =
            match (response.owner_public_key, response.invitee_owner_pubkey) {
                (Some(owner), Some(owner_claim)) if owner.to_bytes() == owner_claim.to_bytes() => {
                    // O = D is self-authenticating: the inner invite-response
                    // event proves possession of D's key. Only a distinct
                    // owner claim needs a signed AppKeys membership proof.
                    let needs_roster = owner != response.invitee_identity;
                    (owner, needs_roster)
                }
                (None, None) => (response.invitee_identity, false),
                (Some(_), Some(_)) => {
                    self.push_debug_log(
                        "invite.private_response.owner_mismatch",
                        format!("event_id={event_id} action=retain_invite"),
                    );
                    return PrivateInviteResponseDisposition::Handled;
                }
                _ => {
                    self.push_debug_log(
                        "invite.private_response.partial_owner",
                        format!("event_id={event_id} action=retain_invite"),
                    );
                    return PrivateInviteResponseDisposition::Handled;
                }
            };
        if let Some(invite) = self.private_chat_invites.get(&invite_key).cloned() {
            if let Some(engine) = self.protocol_engine.as_mut() {
                match engine.ingest_invite_response_owner_proof(
                    &invite,
                    event,
                    owner_pubkey,
                    response.invitee_identity,
                ) {
                    Ok(batch) => self
                        .process_protocol_engine_retry_batch("private_invite_owner_proof", batch),
                    Err(error) => {
                        self.push_debug_log(
                            "invite.private_response.owner_proof",
                            error.to_string(),
                        );
                        return PrivateInviteResponseDisposition::RetryableFailure;
                    }
                }
            }
        }
        let owner_hex = owner_pubkey.to_hex();
        let peer_device_id = response.invitee_identity.to_hex();
        if owner_claim_needs_roster {
            let pending = PendingPrivateInviteResponseV1 {
                invite_key,
                encrypted_event: event.clone(),
                claimed_owner: owner_pubkey,
                authenticated_device: response.invitee_identity,
                queued_at_secs: unix_now().get(),
            };
            if let Err(error) = self.queue_pending_private_invite_response(pending) {
                self.push_debug_log("invite.private_response.queue", error.to_string());
                return PrivateInviteResponseDisposition::RetryableFailure;
            }
            self.push_debug_log(
                "invite.private_response.owner_claim.staged",
                format!(
                    "event_id={event_id} owner={owner_hex} authenticated_device={peer_device_id} action=await_app_keys"
                ),
            );
            self.retry_pending_private_invite_responses(owner_pubkey);
            self.request_protocol_subscription_refresh();
            return PrivateInviteResponseDisposition::Handled;
        }

        let pending = PendingPrivateInviteResponseV1 {
            invite_key,
            encrypted_event: event.clone(),
            claimed_owner: owner_pubkey,
            authenticated_device: response.invitee_identity,
            queued_at_secs: unix_now().get(),
        };
        if self.finalize_private_invite_response_record(&pending) {
            PrivateInviteResponseDisposition::Handled
        } else {
            PrivateInviteResponseDisposition::RetryableFailure
        }
    }

    fn finalize_private_invite_response_record(
        &mut self,
        pending: &PendingPrivateInviteResponseV1,
    ) -> bool {
        let event_id = pending.encrypted_event.id.to_string();
        let Some(invite) = self.private_chat_invites.get(&pending.invite_key).cloned() else {
            self.remove_pending_private_invite_response(&event_id);
            return false;
        };
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        let device_secret = logged_in.device_keys.secret_key().to_secret_bytes();
        let Ok(Some(response)) = nostr_double_ratchet::process_invite_response_event(
            &invite,
            &pending.encrypted_event,
            device_secret,
        ) else {
            self.remove_pending_private_invite_response(&event_id);
            self.push_debug_log(
                "invite.private_response.revalidate",
                format!("event_id={event_id} result=invalid action=retain_invite"),
            );
            return false;
        };
        let revalidated_owner = match (response.owner_public_key, response.invitee_owner_pubkey) {
            (Some(owner), Some(owner_claim)) if owner.to_bytes() == owner_claim.to_bytes() => owner,
            (None, None) => response.invitee_identity,
            _ => {
                self.remove_pending_private_invite_response(&event_id);
                self.push_debug_log(
                    "invite.private_response.revalidate",
                    format!("event_id={event_id} result=owner_mismatch action=retain_invite"),
                );
                return false;
            }
        };
        if revalidated_owner != pending.claimed_owner
            || response.invitee_identity != pending.authenticated_device
        {
            self.remove_pending_private_invite_response(&event_id);
            self.push_debug_log(
                "invite.private_response.revalidate",
                format!("event_id={event_id} result=record_mismatch action=retain_invite"),
            );
            return false;
        }

        let import_result = self.protocol_engine.as_mut().map(|engine| {
            engine.import_private_invite_session_once(
                &event_id,
                pending.claimed_owner,
                pending.authenticated_device,
                response.session.state,
                unix_now(),
            )
        });
        let retry_batch = match import_result {
            Some(Ok(ProtocolInviteSessionImportOutcome::Imported(retry_batch))) => {
                Some(retry_batch)
            }
            Some(Ok(ProtocolInviteSessionImportOutcome::AlreadyImported)) => None,
            Some(Ok(ProtocolInviteSessionImportOutcome::Blocked(block))) => {
                self.push_debug_log(
                    "invite.private_response.import_blocked",
                    format!("event_id={event_id} block={block:?}"),
                );
                return false;
            }
            Some(Err(error)) => {
                self.push_debug_log(
                    "invite.private_response.import",
                    format!(
                        "event_id={event_id} owner={} error={error}",
                        pending.claimed_owner.to_hex()
                    ),
                );
                return false;
            }
            None => return false,
        };

        // Capture the displayed invite before consumption removes it. An older
        // outstanding invite must not redirect a newer invite screen.
        let is_displayed_invite = self
            .build_public_invite_snapshot()
            .is_some_and(|snapshot| chat_invite_url(&invite).is_ok_and(|url| url == snapshot.url));
        if let Err(error) = self.consume_private_chat_invite_after_response(&pending.invite_key) {
            self.pending_private_invite_cleanup_retry = true;
            self.push_debug_log(
                "invite.private_response.consume",
                format!("event_id={event_id} error={error} action=retry_cleanup"),
            );
            return false;
        }
        if let Some(retry_batch) = retry_batch {
            self.process_protocol_engine_retry_batch("private_invite_response", retry_batch);
        }

        self.request_protocol_subscription_refresh_forced_reconnect_if_offline();
        if self.fetch_recent_protocol_state() {
            self.state.busy.syncing_network = true;
        }
        self.fetch_recent_messages_for_tracked_peers();
        self.schedule_tracked_peer_catch_up(Duration::from_secs(2));

        let chat_id = pending.claimed_owner.to_hex();
        self.ensure_thread_record(&chat_id, unix_now().get())
            .unread_count = 0;
        self.remember_recent_handshake_peer(
            chat_id.clone(),
            pending.authenticated_device.to_hex(),
            unix_now().get(),
        );
        // Both the device proof and durable one-use consumption have succeeded.
        // Respect the current screen so delayed handshakes cannot steal focus.
        if is_displayed_invite
            && self.open_invite_chat_on_join
            && !self.is_owner_blocked(&chat_id)
            && matches!(
                self.screen_stack.last(),
                Some(Screen::NewChat | Screen::CreateInvite)
            )
        {
            self.open_chat(&chat_id);
        }
        self.push_debug_log(
            "invite.private_response",
            format!(
                "event_id={event_id} owner={}",
                pending.claimed_owner.to_hex()
            ),
        );
        true
    }

    fn queue_pending_private_invite_response(
        &mut self,
        pending: PendingPrivateInviteResponseV1,
    ) -> anyhow::Result<()> {
        let event_id = pending.encrypted_event.id.to_string();
        if self
            .pending_private_invite_responses
            .contains_key(&event_id)
        {
            return Ok(());
        }
        let serialized = serde_json::to_string(&pending)?;
        if serialized.len() > PENDING_PRIVATE_INVITE_RESPONSE_MAX_BYTES {
            anyhow::bail!("pending private invite response exceeds storage bound");
        }
        let storage = self.private_chat_invite_storage()?;
        storage.put(
            &format!("{PENDING_PRIVATE_INVITE_RESPONSE_KEY_PREFIX}{event_id}"),
            serialized,
        )?;
        self.pending_private_invite_responses
            .insert(event_id, pending);
        self.prune_pending_private_invite_responses();
        Ok(())
    }

    pub(super) fn prune_pending_private_invite_responses(&mut self) {
        let now_secs = unix_now().get();
        let expired = self
            .pending_private_invite_responses
            .iter()
            .filter(|(_, pending)| {
                now_secs.saturating_sub(pending.queued_at_secs)
                    >= PENDING_PRIVATE_INVITE_RESPONSE_TTL_SECS
            })
            .map(|(event_id, _)| event_id.clone())
            .collect::<Vec<_>>();
        for event_id in expired {
            self.remove_pending_private_invite_response(&event_id);
        }

        self.prune_pending_private_invite_response_dimension(
            PENDING_PRIVATE_INVITE_RESPONSE_PER_INVITE_LIMIT,
            |pending| pending.invite_key.clone(),
        );
        self.prune_pending_private_invite_response_dimension(
            PENDING_PRIVATE_INVITE_RESPONSE_LIMIT,
            |_| "all".to_string(),
        );
    }

    pub(super) fn prune_orphaned_pending_private_invite_responses(&mut self) {
        let orphaned = self
            .pending_private_invite_responses
            .iter()
            .filter(|(_, pending)| !self.private_chat_invites.contains_key(&pending.invite_key))
            .map(|(event_id, _)| event_id.clone())
            .collect::<Vec<_>>();
        for event_id in orphaned {
            self.remove_pending_private_invite_response(&event_id);
        }
    }

    fn prune_pending_private_invite_response_dimension(
        &mut self,
        limit: usize,
        key: impl Fn(&PendingPrivateInviteResponseV1) -> String,
    ) {
        let mut groups = BTreeMap::<String, Vec<(u64, String)>>::new();
        for (event_id, pending) in &self.pending_private_invite_responses {
            groups
                .entry(key(pending))
                .or_default()
                .push((pending.queued_at_secs, event_id.clone()));
        }
        let mut remove = Vec::new();
        for pending in groups.values_mut() {
            pending.sort();
            let excess = pending.len().saturating_sub(limit);
            remove.extend(
                pending
                    .iter()
                    .take(excess)
                    .map(|(_, event_id)| event_id.clone()),
            );
        }
        remove.sort();
        remove.dedup();
        for event_id in remove {
            self.remove_pending_private_invite_response(&event_id);
        }
    }

    fn remove_pending_private_invite_response(&mut self, event_id: &str) {
        self.pending_private_invite_responses.remove(event_id);
        if let Ok(storage) = self.private_chat_invite_storage() {
            let _ = storage.del(&format!(
                "{PENDING_PRIVATE_INVITE_RESPONSE_KEY_PREFIX}{event_id}"
            ));
        }
    }

    pub(super) fn retry_pending_private_invite_responses(&mut self, owner: PublicKey) {
        let pending = self
            .pending_private_invite_responses
            .values()
            .filter(|pending| pending.claimed_owner == owner)
            .cloned()
            .collect::<Vec<_>>();
        let mut retrying_invites = HashSet::new();
        for pending in pending {
            if retrying_invites.contains(&pending.invite_key) {
                continue;
            }
            if !self.finalize_private_invite_response_record(&pending) {
                // A failed one-use invite cleanup must finish before a sibling
                // response can import another session from the same invite.
                retrying_invites.insert(pending.invite_key.clone());
            }
        }
    }

    pub(super) fn retry_all_pending_private_invite_responses(&mut self) {
        self.pending_private_invite_cleanup_retry = false;
        let owners = self
            .pending_private_invite_responses
            .values()
            .map(|pending| pending.claimed_owner)
            .collect::<HashSet<_>>();
        for owner in owners {
            self.retry_pending_private_invite_responses(owner);
        }
    }

    pub(super) fn accept_invite(&mut self, invite_input: &str) {
        self.pending_private_invite_lookup = None;
        self.pending_outgoing_invite_acceptance = None;
        if let Some(reference) = crate::invite_record::reference_from_input(invite_input) {
            let Some(owner) = self
                .logged_in
                .as_ref()
                .map(|login| login.owner_pubkey.to_hex())
            else {
                return;
            };
            let request_id = uuid::Uuid::new_v4().to_string();
            self.pending_private_invite_lookup = Some(request_id.clone());
            self.state.busy.accepting_invite = true;
            self.emit_state();
            let reference = reference.to_string();
            let sender = self.core_sender.clone();
            self.runtime.spawn(async move {
                let result = crate::invite_record::load_record(&reference)
                    .await
                    .map_err(|e| e.to_string());
                let _ = sender.send(CoreMsg::Internal(Box::new(
                    InternalEvent::PrivateInviteResolved {
                        request_id,
                        owner,
                        result,
                    },
                )));
            });
            return;
        }
        self.accept_invite_input(invite_input, None);
    }

    pub(super) fn finish_private_invite_lookup(
        &mut self,
        request_id: String,
        owner: String,
        result: Result<crate::invite_record::InviteRecord, String>,
    ) {
        if self.pending_private_invite_lookup.as_ref() != Some(&request_id)
            || self
                .logged_in
                .as_ref()
                .map(|login| login.owner_pubkey.to_hex())
                .as_ref()
                != Some(&owner)
        {
            return;
        }
        self.pending_private_invite_lookup = None;
        match result.and_then(|record| {
            if record
                .expires_at
                .is_some_and(|expiry| expiry <= unix_now().get())
            {
                return Err("This invite has expired.".to_string());
            }
            record
                .legacy_url()
                .map(|url| (url, record))
                .map_err(|e| e.to_string())
        }) {
            Ok((url, record)) => self.accept_invite_input(&url, Some(record)),
            Err(error) => {
                self.state.busy.accepting_invite = false;
                self.state.toast = Some(error);
                self.emit_state();
            }
        }
    }

    fn accept_invite_input(
        &mut self,
        invite_input: &str,
        record: Option<crate::invite_record::InviteRecord>,
    ) {
        if !self.can_use_chats() {
            self.state.toast = Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
            self.emit_state();
            return;
        }

        let trimmed = invite_input.trim();
        if trimmed.is_empty() {
            self.state.toast = Some("Invite link is required.".to_string());
            self.emit_state();
            return;
        }

        self.pending_outgoing_invite_acceptance = None;

        self.state.busy.accepting_invite = true;
        self.emit_state();

        let result = match parse_public_invite_or_direct_chat_input(trimmed) {
            Ok(PublicInviteInput::Invite(invite)) => {
                resolve_invite_owner(&invite, None).and_then(|owner_pubkey| {
                    if owner_pubkey != invite.inviter || self.defer_owner_app_keys_publish {
                        self.pending_outgoing_invite_acceptance =
                            Some(PendingOutgoingInviteAcceptance {
                                invite,
                                record: record.clone(),
                                claimed_owner: owner_pubkey,
                                queued_at_secs: unix_now().get(),
                            });
                        match self.try_pending_outgoing_invite_acceptance(owner_pubkey)? {
                            Some(chat_id) => Ok(AcceptInviteDispatch::Completed(chat_id)),
                            None => {
                                self.request_protocol_subscription_refresh();
                                Ok(AcceptInviteDispatch::Pending)
                            }
                        }
                    } else {
                        self.accept_parsed_invite(invite, owner_pubkey, record.as_ref())?
                            .map(AcceptInviteDispatch::Completed)
                            .ok_or_else(|| anyhow::anyhow!("Invite authorization is unavailable."))
                    }
                })
            }
            Ok(PublicInviteInput::DirectChat) => self
                .open_direct_chat_from_peer_input(trimmed)
                .map(AcceptInviteDispatch::Completed),
            Err(_) => Err(anyhow::anyhow!("Invalid invite link.")),
        };

        match result {
            Ok(AcceptInviteDispatch::Completed(chat_id)) => {
                if chat_id.starts_with("group:") {
                    self.active_chat_id = None;
                    self.screen_stack = vec![Screen::ChatList];
                    self.state.toast = Some("Waiting for the group admin to connect…".to_string());
                } else {
                    self.active_chat_id = Some(chat_id.clone());
                    self.screen_stack = vec![Screen::Chat { chat_id }];
                }
                self.request_protocol_subscription_refresh_forced();
                self.fetch_recent_protocol_state();
                self.persist_best_effort();
                self.state.busy.accepting_invite = false;
            }
            Ok(AcceptInviteDispatch::Pending) => {
                self.state.toast = Some(
                    if self.defer_owner_app_keys_publish {
                        "Setting up this device…"
                    } else {
                        "Verifying the invite owner's device…"
                    }
                    .to_string(),
                );
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
                self.state.busy.accepting_invite = false;
            }
        }

        self.rebuild_state();
        self.emit_state();
    }

    fn accept_parsed_invite(
        &mut self,
        invite: Invite,
        owner_pubkey: PublicKey,
        record: Option<&crate::invite_record::InviteRecord>,
    ) -> anyhow::Result<Option<String>> {
        if self.defer_owner_app_keys_publish {
            self.recover_deferred_owner_registration();
            return Ok(None);
        }
        let outcome = self
            .protocol_engine
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Protocol engine is not ready."))?
            .accept_invite(&invite, Some(owner_pubkey))?;
        let result = match outcome {
            ProtocolAcceptInviteOutcome::Accepted(result) => result,
            ProtocolAcceptInviteOutcome::Blocked(
                ProtocolAcceptInviteBlock::MissingOwnerRoster { .. },
            ) => return Ok(None),
            ProtocolAcceptInviteOutcome::Blocked(
                ProtocolAcceptInviteBlock::UnauthorizedDevice { .. },
            ) => anyhow::bail!("The invite device is not authorized by its claimed owner."),
        };
        let chat_id = result.owner_pubkey.to_hex();

        if let Some(record) = record.filter(|record| record.kind == "group") {
            self.process_protocol_engine_effects(result.effects);
            self.send_group_invite_join_request(record)?;
            return Ok(Some(format!(
                "group:{}",
                record.group_id.as_deref().unwrap_or_default()
            )));
        }
        self.ensure_thread_record(&chat_id, unix_now().get())
            .unread_count = 0;
        self.remember_recent_handshake_peer(
            chat_id.clone(),
            result.inviter_device_pubkey.to_hex(),
            unix_now().get(),
        );
        // Accepting an invite installs a new session — invalidate the
        // cached mobile-push snapshot so the new recipient appears.
        self.mark_mobile_push_dirty();
        self.process_protocol_engine_effects(result.effects);
        Ok(Some(chat_id))
    }

    fn try_pending_outgoing_invite_acceptance(
        &mut self,
        owner: PublicKey,
    ) -> anyhow::Result<Option<String>> {
        let Some(pending) = self
            .pending_outgoing_invite_acceptance
            .as_ref()
            .filter(|pending| pending.claimed_owner == owner)
            .cloned()
        else {
            return Ok(None);
        };
        if unix_now().get().saturating_sub(pending.queued_at_secs)
            >= PENDING_OUTGOING_INVITE_ACCEPTANCE_TTL_SECS
        {
            self.pending_outgoing_invite_acceptance = None;
            anyhow::bail!(
                "Could not verify the invite owner's device list. Reopen the invite to retry."
            );
        }
        let accepted = self.accept_parsed_invite(pending.invite, owner, pending.record.as_ref())?;
        if accepted.is_some() {
            self.pending_outgoing_invite_acceptance = None;
        }
        Ok(accepted)
    }

    pub(super) fn resume_pending_outgoing_invite_acceptance(&mut self, owner: PublicKey) {
        match self.try_pending_outgoing_invite_acceptance(owner) {
            Ok(Some(chat_id)) => {
                self.state.busy.accepting_invite = false;
                self.state.toast = None;
                if chat_id.starts_with("group:") {
                    self.active_chat_id = None;
                    self.screen_stack = vec![Screen::ChatList];
                    self.state.toast = Some("Waiting for the group admin to connect…".to_string());
                } else {
                    self.active_chat_id = Some(chat_id.clone());
                    self.screen_stack = vec![Screen::Chat { chat_id }];
                }
                self.request_protocol_subscription_refresh_forced();
                self.fetch_recent_protocol_state();
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
            }
            Ok(None) => {}
            Err(error) => {
                self.pending_outgoing_invite_acceptance = None;
                self.state.busy.accepting_invite = false;
                self.state.toast = Some(error.to_string());
                self.request_protocol_subscription_refresh();
                self.rebuild_state();
                self.emit_state();
            }
        }
    }

    fn send_group_invite_join_request(
        &mut self,
        record: &crate::invite_record::InviteRecord,
    ) -> anyhow::Result<()> {
        let owner = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not logged in"))?
            .owner_pubkey;
        let peer = PublicKey::from_hex(&record.invite.owner)?;
        let group_id = record
            .group_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Invalid group invite"))?;
        let content =
            serde_json::json!({ "v": 1, "groupId": group_id, "token": record.token }).to_string();
        self.private_chat_invite_storage()?.put(
            &format!("pending-group-invite-v1/{group_id}"),
            serde_json::to_string(record)?,
        )?;
        let unsigned = EventBuilder::new(Kind::Custom(10461), content)
            .tag(nostr::Tag::public_key(peer))
            .build(owner);
        anyhow::ensure!(
            self.send_protocol_engine_unsigned_event_to_peer_only(
                peer,
                &peer.to_hex(),
                unsigned,
                "group_invite.join"
            ),
            "Could not send join request. Reopen the invite to retry."
        );
        self.schedule_fast_protocol_retry_if_pending();
        Ok(())
    }

    pub(super) fn receive_group_invite_result(
        &mut self,
        sender: PublicKey,
        device: Option<PublicKey>,
        content: &str,
    ) -> bool {
        if content.len() > 1024 {
            return true;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
            return true;
        };
        let Some(group_id) = value
            .get("groupId")
            .and_then(|v| v.as_str())
            .filter(|v| v.len() <= 512)
        else {
            return true;
        };
        let pending = self
            .private_chat_invite_storage()
            .and_then(|storage| {
                storage
                    .get(&format!("pending-group-invite-v1/{group_id}"))?
                    .ok_or_else(|| anyhow::anyhow!("No pending invitation"))
            })
            .ok()
            .and_then(|data| {
                serde_json::from_str::<crate::invite_record::InviteRecord>(&data).ok()
            });
        let Some(record) = pending else {
            return true;
        };
        if value.get("v").and_then(|v| v.as_u64()) != Some(1)
            || sender.to_hex() != record.invite.owner
            || device.map(|v| v.to_hex()).as_deref() != Some(record.invite.inviter.as_str())
            || value.get("token").and_then(|v| v.as_str()) != record.token.as_deref()
        {
            return true;
        }
        let message = match value.get("status").and_then(|v| v.as_str()) {
            Some("added" | "already-member") => "Joining group…",
            Some("expired") => "This invite has expired.",
            Some("full") => "This invite has reached its use limit.",
            Some("used") => "This invite has already been used.",
            Some("unavailable") => "This invite is no longer available.",
            _ => return true,
        };
        self.state.toast = Some(message.to_string());
        self.emit_state();
        true
    }

    pub(super) fn reset_pending_invite_acceptance(&mut self) {
        if self.pending_outgoing_invite_acceptance.take().is_some() {
            self.state.toast = None;
        }
        self.pending_private_invite_lookup = None;
        self.state.busy.accepting_invite = false;
    }
}

enum AcceptInviteDispatch {
    Completed(String),
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PrivateInviteResponseDisposition {
    NotMatched,
    Handled,
    RetryableFailure,
}

pub(super) fn private_chat_invite_key(invite: &Invite) -> String {
    format!(
        "{}{}",
        PRIVATE_CHAT_INVITE_KEY_PREFIX, invite.inviter_ephemeral_public_key
    )
}

pub(super) fn load_private_chat_invites(
    storage: &dyn StorageAdapter,
) -> anyhow::Result<BTreeMap<String, Invite>> {
    let mut invites = BTreeMap::new();
    for key in storage.list(PRIVATE_CHAT_INVITE_KEY_PREFIX)? {
        let Some(serialized) = storage.get(&key)? else {
            continue;
        };
        match Invite::deserialize(&serialized) {
            Ok(invite) => {
                invites.insert(key, invite);
            }
            Err(_) => {
                let _ = storage.del(&key);
            }
        }
    }
    Ok(invites)
}

pub(super) fn load_pending_private_invite_responses(
    storage: &dyn StorageAdapter,
) -> anyhow::Result<BTreeMap<String, PendingPrivateInviteResponseV1>> {
    let mut responses = BTreeMap::new();
    let now_secs = unix_now().get();
    for key in storage.list(PENDING_PRIVATE_INVITE_RESPONSE_KEY_PREFIX)? {
        let Some(serialized) = storage.get(&key)? else {
            continue;
        };
        match serde_json::from_str::<PendingPrivateInviteResponseV1>(&serialized) {
            Ok(pending)
                if serialized.len() <= PENDING_PRIVATE_INVITE_RESPONSE_MAX_BYTES
                    && key
                        == format!(
                            "{PENDING_PRIVATE_INVITE_RESPONSE_KEY_PREFIX}{}",
                            pending.encrypted_event.id
                        )
                    && now_secs.saturating_sub(pending.queued_at_secs)
                        < PENDING_PRIVATE_INVITE_RESPONSE_TTL_SECS =>
            {
                responses.insert(pending.encrypted_event.id.to_string(), pending);
            }
            _ => {
                let _ = storage.del(&key);
            }
        }
    }
    Ok(responses)
}
