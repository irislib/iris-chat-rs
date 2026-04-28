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
        self.publish_local_identity_artifacts();
        self.request_protocol_subscription_refresh();
        self.persist_best_effort();
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

        let result = match parse_public_invite_or_direct_chat_input(trimmed) {
            Ok(PublicInviteInput::Invite(invite)) => self.accept_parsed_invite(invite),
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
                .ok_or_else(|| anyhow::anyhow!("Create or restore an account first."))?;
            logged_in
                .ndr_runtime
                .accept_invite(&invite, Some(owner_pubkey))?
        };

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
        self.process_runtime_events();
        Ok(chat_id)
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

fn parse_public_invite_input(input: &str) -> anyhow::Result<Invite> {
    if let Ok(invite) = Invite::from_url(input) {
        return Ok(invite);
    }

    let Ok(url) = url::Url::parse(input) else {
        return Invite::from_url(input).map_err(|error| anyhow::anyhow!(error.to_string()));
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

    Invite::from_url(input).map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn parse_invite_candidate(candidate: &str) -> anyhow::Result<Invite> {
    let trimmed = candidate.trim().trim_start_matches('/');
    if let Ok(invite) = Invite::from_url(trimmed) {
        return Ok(invite);
    }
    Invite::from_url(&format!("{CHAT_INVITE_ROOT_URL}#{trimmed}"))
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
        invite.get_url(CHAT_INVITE_ROOT_URL).expect("invite url")
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
