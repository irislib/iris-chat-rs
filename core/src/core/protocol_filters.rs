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
    message_authors: impl IntoIterator<Item = PublicKey>,
    now: UnixSeconds,
) -> Vec<Filter> {
    let owners = dedupe_pubkeys(owners);
    let invite_authors = dedupe_pubkeys(invite_authors);
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
