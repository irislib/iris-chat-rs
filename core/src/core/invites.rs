use super::*;

impl AppCore {
    pub(super) fn create_public_invite(&mut self) {
        if !self.can_use_chats() {
            self.state.toast = Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
            self.emit_state();
            return;
        }

        self.state.busy.creating_invite = true;
        self.emit_state();

        let result = (|| -> anyhow::Result<()> {
            let logged_in = self
                .logged_in
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Create or restore an account first."))?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(unix_now(), &mut rng);
            logged_in.session_manager.ensure_local_invite(&mut ctx)?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.publish_local_identity_artifacts();
                self.request_protocol_subscription_refresh();
                self.persist_best_effort();
            }
            Err(error) => self.state.toast = Some(error.to_string()),
        }

        self.state.busy.creating_invite = false;
        self.rebuild_state();
        self.emit_state();
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

        let result = parse_public_invite_input(trimmed)
            .map_err(|_| anyhow::anyhow!("Invalid invite link."))
            .and_then(|invite| self.accept_parsed_invite(invite));

        match result {
            Ok(chat_id) => {
                self.active_chat_id = Some(chat_id.clone());
                self.screen_stack = vec![Screen::Chat { chat_id }];
                self.request_protocol_subscription_refresh_forced();
                self.fetch_recent_messages_for_tracked_peers(unix_now());
                self.persist_best_effort();
            }
            Err(error) => self.state.toast = Some(error.to_string()),
        }

        self.state.busy.accepting_invite = false;
        self.rebuild_state();
        self.emit_state();
    }

    fn accept_parsed_invite(&mut self, invite: Invite) -> anyhow::Result<String> {
        let owner_pubkey = invite
            .inviter_owner_pubkey
            .unwrap_or_else(|| OwnerPubkey::from_bytes(invite.inviter_device_pubkey.to_bytes()));
        let chat_id = owner_pubkey.to_string();
        let response = {
            let logged_in = self
                .logged_in
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Create or restore an account first."))?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(unix_now(), &mut rng);
            logged_in.session_manager.accept_invite(&mut ctx, &invite)?
        };

        let response_event = codec::invite_response_event(&response)?;
        self.remember_event(response_event.id.to_string());
        self.start_invite_response_publish(response_event);
        self.ensure_thread_record(&chat_id, unix_now().get())
            .unread_count = 0;
        self.remember_recent_handshake_peer(
            chat_id.clone(),
            invite.inviter_device_pubkey.to_string(),
            unix_now().get(),
        );

        Ok(chat_id)
    }

    pub(super) fn start_invite_response_publish(&self, event: Event) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        self.runtime.spawn(async move {
            let _ = publish_event_with_retry(&client, &relay_urls, event, "invite response").await;
        });
    }
}

fn parse_public_invite_input(input: &str) -> codec::Result<Invite> {
    if let Ok(invite) = codec::parse_invite_url(input) {
        return Ok(invite);
    }

    let Ok(url) = url::Url::parse(input) else {
        return codec::parse_invite_url(input);
    };

    for (_, value) in url.query_pairs() {
        if let Ok(invite) = codec::parse_invite_url(&format!("{CHAT_INVITE_ROOT_URL}#{value}")) {
            return Ok(invite);
        }
    }

    if let Some(fragment) = url.fragment() {
        for (_, value) in url::form_urlencoded::parse(fragment.as_bytes()) {
            if let Ok(invite) = codec::parse_invite_url(&format!("{CHAT_INVITE_ROOT_URL}#{value}"))
            {
                return Ok(invite);
            }
        }
    }

    codec::parse_invite_url(input)
}
