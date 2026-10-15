use super::*;

pub(in crate::core) fn chat_invite_url(invite: &Invite) -> anyhow::Result<String> {
    let url = nostr_double_ratchet::invite_url(invite, CHAT_INVITE_ROOT_URL)?;
    Ok(route_wrapped_chat_invite_url(&url))
}

#[allow(clippy::large_enum_variant)]
pub(super) enum PublicInviteInput {
    Invite(Invite),
    DirectChat,
}

pub(super) fn parse_public_invite_or_direct_chat_input(
    input: &str,
) -> anyhow::Result<PublicInviteInput> {
    if let Ok(invite) = parse_public_invite_input(input) {
        return Ok(PublicInviteInput::Invite(invite));
    }
    parse_peer_input(input)?;
    Ok(PublicInviteInput::DirectChat)
}

pub(in crate::core) fn parse_public_invite_input(input: &str) -> anyhow::Result<Invite> {
    if let Ok(invite) = nostr_double_ratchet::parse_invite_url(input) {
        return Ok(invite);
    }

    let Ok(url) = url::Url::parse(input) else {
        return nostr_double_ratchet::parse_invite_url(input)
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

    nostr_double_ratchet::parse_invite_url(input)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn parse_invite_candidate(candidate: &str) -> anyhow::Result<Invite> {
    let trimmed = candidate.trim().trim_start_matches('/');
    if let Ok(invite) = nostr_double_ratchet::parse_invite_url(trimmed) {
        return Ok(invite);
    }
    nostr_double_ratchet::parse_invite_url(&format!("{CHAT_INVITE_ROOT_URL}#{trimmed}"))
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn route_wrapped_chat_invite_url(url: &str) -> String {
    let Some((base, fragment)) = url.split_once('#') else {
        return url.to_string();
    };
    let payload = fragment.trim_start_matches('/');
    if payload.is_empty() || payload.starts_with("invite/") {
        return url.to_string();
    }
    format!("{base}#/invite/{payload}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_invite_url() -> String {
        let keys = Keys::generate();
        let mut invite = Invite::create_new(keys.public_key(), Some("public".to_string()), None)
            .expect("invite");
        invite.owner_public_key = Some(keys.public_key());
        chat_invite_url(&invite).expect("invite url")
    }

    #[test]
    fn public_invite_url_uses_chat_iris_root() {
        assert!(sample_invite_url().starts_with("https://chat.iris.to/#/invite/"));
    }

    #[test]
    fn parse_public_invite_input_accepts_hash_route_wrapper() {
        let url = sample_invite_url();
        let encoded = url.split("/invite/").nth(1).expect("payload");
        let wrapped = format!("https://chat.iris.to/#/invite/{encoded}");

        let parsed = parse_public_invite_input(&wrapped).expect("parse wrapped invite");

        assert!(parsed.owner_public_key.is_some());
    }

    #[test]
    fn parse_public_invite_input_accepts_user_link_as_direct_chat() {
        let keys = Keys::generate();
        let npub = keys.public_key().to_bech32().expect("npub");
        let wrapped = format!("https://chat.iris.to/#/{npub}");

        let parsed =
            parse_public_invite_or_direct_chat_input(&wrapped).expect("parse direct chat link");

        assert!(matches!(parsed, PublicInviteInput::DirectChat));
    }

    #[test]
    fn route_wrapped_chat_invite_url_preserves_legacy_payload() {
        let legacy = "https://chat.iris.to/#%7B%22purpose%22%3A%22private%22%7D";

        let wrapped = route_wrapped_chat_invite_url(legacy);

        assert_eq!(
            wrapped,
            "https://chat.iris.to/#/invite/%7B%22purpose%22%3A%22private%22%7D"
        );
    }
}
