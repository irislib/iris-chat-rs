use super::*;

pub(super) fn sorted_hexes(values: HashSet<String>) -> Vec<String> {
    let mut sorted = values.into_iter().collect::<Vec<_>>();
    sorted.sort();
    sorted.dedup();
    sorted
}

pub(super) fn summarize_protocol_plan(plan: Option<&ProtocolSubscriptionPlan>) -> String {
    let Some(plan) = plan else {
        return "none".to_string();
    };
    format!("runtime={}", plan.runtime_subscriptions.join(","))
}

pub(super) fn recent_protocol_filters(
    owners: impl IntoIterator<Item = PublicKey>,
    invite_authors: impl IntoIterator<Item = PublicKey>,
    invite_response_pubkeys: impl IntoIterator<Item = PublicKey>,
    message_authors: impl IntoIterator<Item = PublicKey>,
    now: UnixSeconds,
) -> Vec<Filter> {
    let owners = dedupe_pubkeys(owners);
    let invite_authors = dedupe_pubkeys(invite_authors);
    let invite_response_pubkeys = dedupe_pubkeys(invite_response_pubkeys);
    let message_authors = dedupe_pubkeys(message_authors);
    let mut filters = Vec::new();

    if !owners.is_empty() {
        filters.push(Filter::new().kind(Kind::Metadata).authors(owners.clone()));
        filters.push(
            Filter::new()
                .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
                .authors(owners.clone()),
        );
    }

    if !invite_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_EVENT_KIND as u16))
                .authors(invite_authors)
                .custom_tag(
                    nostr::SingleLetterTag::lowercase(nostr::Alphabet::L),
                    "double-ratchet/invites",
                )
                .since(Timestamp::from(
                    now.get()
                        .saturating_sub(DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS),
                )),
        );
    }

    if !invite_response_pubkeys.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
                .pubkeys(invite_response_pubkeys)
                .since(Timestamp::from(
                    now.get()
                        .saturating_sub(DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS),
                )),
        );
    }

    if !message_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
                .authors(message_authors)
                .since(Timestamp::from(
                    now.get().saturating_sub(CATCH_UP_LOOKBACK_SECS),
                )),
        );
    }

    filters
}

fn dedupe_pubkeys(values: impl IntoIterator<Item = PublicKey>) -> Vec<PublicKey> {
    let mut seen = HashSet::new();
    let mut output = values
        .into_iter()
        .filter(|pubkey| seen.insert(pubkey.to_hex()))
        .collect::<Vec<_>>();
    output.sort_by_key(PublicKey::to_hex);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[test]
    fn recent_protocol_filters_backfill_current_device_invite_responses() {
        let invite_response_pubkey = Keys::generate().public_key();

        let filters = recent_protocol_filters(
            Vec::<PublicKey>::new(),
            Vec::<PublicKey>::new(),
            [invite_response_pubkey],
            Vec::<PublicKey>::new(),
            UnixSeconds(1_777_159_500),
        );

        let expected_pubkey = invite_response_pubkey.to_hex();
        let response_filter = filters
            .iter()
            .map(|filter| serde_json::to_value(filter).expect("filter json"))
            .find(|filter| {
                filter
                    .get("kinds")
                    .and_then(|kinds| kinds.as_array())
                    .is_some_and(|kinds| {
                        kinds
                            .iter()
                            .any(|kind| kind.as_u64() == Some(INVITE_RESPONSE_KIND as u64))
                    })
            })
            .expect("invite response filter");

        assert_eq!(
            response_filter
                .get("#p")
                .and_then(|pubkeys| pubkeys.as_array())
                .and_then(|pubkeys| pubkeys.first())
                .and_then(|pubkey| pubkey.as_str()),
            Some(expected_pubkey.as_str())
        );
        assert_eq!(
            response_filter
                .get("since")
                .and_then(|since| since.as_u64()),
            Some(1_777_159_500 - DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS)
        );
    }
}
