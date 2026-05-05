use super::*;

const PRIVATE_CHAT_INVITE_KEY_PREFIX: &str = "private-chat-invites/";

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

    pub(super) fn forget_private_chat_invite_keys(&mut self, keys: &[String]) {
        if keys.is_empty() {
            return;
        }
        if let Ok(storage) = self.private_chat_invite_storage() {
            for key in keys {
                let _ = storage.del(key);
            }
        }
        for key in keys {
            self.private_chat_invites.remove(key);
        }
        self.mark_mobile_push_dirty();
        self.request_protocol_subscription_refresh();
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

    pub(super) fn handle_private_chat_invite_response(&mut self, event: &Event) -> bool {
        if event.kind.as_u16() as u32 != INVITE_RESPONSE_KIND {
            return false;
        }
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
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
            match nostr_double_ratchet_nostr::process_invite_response_event(
                &invite,
                event,
                device_secret,
            ) {
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
            return false;
        };

        let owner_pubkey = response
            .owner_public_key
            .unwrap_or(response.invitee_identity);
        let peer_device_id = response
            .device_id
            .clone()
            .unwrap_or_else(|| response.invitee_identity.to_hex());
        let session_state = response.session.state;
        let import_result = self
            .logged_in
            .as_ref()
            .expect("checked logged in")
            .ndr_runtime
            .import_session_state(owner_pubkey, Some(peer_device_id.clone()), session_state);
        let effects = match import_result {
            Ok(effects) => effects,
            Err(error) => {
                self.push_debug_log(
                    "invite.private_response.import",
                    format!(
                        "event_id={event_id} owner={} error={error}",
                        owner_pubkey.to_hex()
                    ),
                );
                return false;
            }
        };
        self.process_runtime_effects(effects);
        self.fetch_recent_messages_for_tracked_peers(unix_now());

        let chat_id = owner_pubkey.to_hex();
        self.ensure_thread_record(&chat_id, unix_now().get())
            .unread_count = 0;
        self.remember_recent_handshake_peer(chat_id, peer_device_id, unix_now().get());
        self.forget_private_chat_invite_keys(&[invite_key]);
        self.push_debug_log(
            "invite.private_response",
            format!("event_id={event_id} owner={}", owner_pubkey.to_hex()),
        );
        true
    }

    pub(super) fn accept_invite(&mut self, invite_input: &str) {
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

        self.state.busy.accepting_invite = true;
        self.emit_state();

        let result = match parse_public_invite_or_direct_chat_input(trimmed) {
            Ok(PublicInviteInput::Invite(invite)) => {
                self.preload_invite_owner_app_keys(&invite);
                self.accept_parsed_invite(invite)
            }
            Ok(PublicInviteInput::DirectChat) => self.open_direct_chat_from_peer_input(trimmed),
            Err(_) => Err(anyhow::anyhow!("Invalid invite link.")),
        };

        match result {
            Ok(chat_id) => {
                self.active_chat_id = Some(chat_id.clone());
                self.screen_stack = vec![Screen::Chat { chat_id }];
                self.request_protocol_subscription_refresh_forced();
                self.fetch_recent_protocol_state();
                self.persist_best_effort();
            }
            Err(error) => self.state.toast = Some(error.to_string()),
        }

        self.state.busy.accepting_invite = false;
        self.rebuild_state();
        self.emit_state();
    }

    fn accept_parsed_invite(&mut self, invite: Invite) -> anyhow::Result<String> {
        let owner_pubkey = invite.owner_public_key.unwrap_or(invite.inviter);
        let chat_id = owner_pubkey.to_hex();
        let result = {
            let logged_in = self
                .logged_in
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Create or restore a profile first."))?;
            logged_in
                .ndr_runtime
                .accept_invite(&invite, Some(owner_pubkey))?
        };

        self.ensure_thread_record(&chat_id, unix_now().get())
            .unread_count = 0;
        self.remember_recent_handshake_peer(
            chat_id.clone(),
            result.outcome.inviter_device_pubkey.to_hex(),
            unix_now().get(),
        );
        // Accepting an invite installs a new session — invalidate the
        // cached mobile-push snapshot so the new recipient appears.
        self.mark_mobile_push_dirty();
        self.process_runtime_effects(result.effects);
        Ok(chat_id)
    }

    fn preload_invite_owner_app_keys(&mut self, invite: &Invite) {
        let Some(owner_pubkey) = invite.owner_public_key else {
            return;
        };
        if owner_pubkey == invite.inviter {
            return;
        }
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .filter(|logged_in| !logged_in.relay_urls.is_empty())
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };

        let filter = Filter::new()
            .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
            .author(owner_pubkey)
            .limit(10);
        let fetched = self.runtime.block_on(async {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(2)).await;
            client.fetch_events(filter, Duration::from_secs(2)).await
        });

        let Ok(events) = fetched else {
            self.push_debug_log(
                "invite.app_keys.preload",
                format!("owner={} result=fetch_failed", owner_pubkey.to_hex()),
            );
            return;
        };

        let latest = events
            .iter()
            .filter(|event| is_app_keys_event(event))
            .max_by_key(|event| (event.created_at.as_secs(), event.id.to_hex()))
            .cloned();
        let Some(event) = latest else {
            self.push_debug_log(
                "invite.app_keys.preload",
                format!("owner={} result=not_found", owner_pubkey.to_hex()),
            );
            return;
        };

        let created_at = event.created_at.as_secs();
        match self.apply_app_keys_event(&event) {
            Ok(_) => self.push_debug_log(
                "invite.app_keys.preload",
                format!(
                    "owner={} result=applied created_at={created_at}",
                    owner_pubkey.to_hex()
                ),
            ),
            Err(error) => self.push_debug_log(
                "invite.app_keys.preload",
                format!(
                    "owner={} result=apply_failed error={error}",
                    owner_pubkey.to_hex()
                ),
            ),
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum PublicInviteInput {
    Invite(Invite),
    DirectChat,
}

fn parse_public_invite_or_direct_chat_input(input: &str) -> anyhow::Result<PublicInviteInput> {
    if let Ok(invite) = parse_public_invite_input(input) {
        return Ok(PublicInviteInput::Invite(invite));
    }
    parse_peer_input(input)?;
    Ok(PublicInviteInput::DirectChat)
}

pub(super) fn parse_public_invite_input(input: &str) -> anyhow::Result<Invite> {
    if let Ok(invite) = nostr_double_ratchet_nostr::parse_invite_url(input) {
        return Ok(invite);
    }

    let Ok(url) = url::Url::parse(input) else {
        return nostr_double_ratchet_nostr::parse_invite_url(input)
            .map_err(|error| anyhow::anyhow!(error.to_string()));
    };

    for (key, value) in url.query_pairs() {
        for candidate in [key.as_ref(), value.as_ref()] {
            if let Ok(invite) = parse_invite_candidate(candidate) {
                return Ok(invite);
            }
        }
    }

    if let Some(fragment) = url.fragment() {
        if let Ok(invite) = parse_invite_candidate(fragment) {
            return Ok(invite);
        }
        for (_, value) in url::form_urlencoded::parse(fragment.as_bytes()) {
            if let Ok(invite) = parse_invite_candidate(&value) {
                return Ok(invite);
            }
        }
        for part in fragment.split(['/', '?', '&', '=']) {
            if let Ok(invite) = parse_invite_candidate(part) {
                return Ok(invite);
            }
        }
    }

    nostr_double_ratchet_nostr::parse_invite_url(input)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
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

fn parse_invite_candidate(candidate: &str) -> anyhow::Result<Invite> {
    let trimmed = candidate.trim().trim_start_matches('/');
    if let Ok(invite) = nostr_double_ratchet_nostr::parse_invite_url(trimmed) {
        return Ok(invite);
    }
    nostr_double_ratchet_nostr::parse_invite_url(&format!("{CHAT_INVITE_ROOT_URL}#{trimmed}"))
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_invite_url() -> String {
        let keys = Keys::generate();
        let mut invite = Invite::create_new(keys.public_key(), Some("public".to_string()), None)
            .expect("invite");
        invite.owner_public_key = Some(keys.public_key());
        nostr_double_ratchet_nostr::invite_url(&invite, CHAT_INVITE_ROOT_URL).expect("invite url")
    }

    #[test]
    fn public_invite_url_uses_chat_iris_root() {
        assert!(sample_invite_url().starts_with("https://chat.iris.to/#"));
    }

    #[test]
    fn parse_public_invite_input_accepts_hash_route_wrapper() {
        let url = sample_invite_url();
        let encoded = url.split('#').nth(1).expect("hash");
        let wrapped = format!("https://chat.iris.to/#/invite/{encoded}");

        let parsed = parse_public_invite_input(&wrapped).expect("parse wrapped invite");

        assert!(parsed.owner_public_key.is_some());
    }

    #[test]
    fn parse_public_invite_input_accepts_user_link_as_direct_chat() {
        let keys = Keys::generate();
        let npub = keys.public_key().to_bech32().expect("npub");
        let wrapped = format!("https://chat.iris.to/#{npub}");

        let parsed =
            parse_public_invite_or_direct_chat_input(&wrapped).expect("parse direct chat link");

        assert!(matches!(parsed, PublicInviteInput::DirectChat));
    }
}
