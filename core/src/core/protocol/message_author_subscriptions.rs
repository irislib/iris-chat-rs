//! Live subscriptions to message authors, sized for relay limits.
//!
//! Message events name no recipient, so the sender ratchet keys this device
//! subscribes to are the only way it finds its messages. A relay refuses a
//! request that is too large as a whole, which stops all delivery from that
//! relay. Authors therefore go out in bounded chunks, one request each, and a
//! chunk is requested again only when its own authors change.

use super::*;

/// Authors in one message subscription on a message server. The tightest
/// relays in common use take 131,072-byte frames (about 1,950 hex authors per
/// request) and 2,047 items per filter.
pub(in crate::core) const RELAY_MESSAGE_AUTHOR_CHUNK_SIZE: usize =
    nostr_double_ratchet::MESSAGE_AUTHOR_FILTER_CHUNK_SIZE;
/// A FIPS mesh frame carries at most 65,525 bytes: about 975 hex authors.
pub(in crate::core) const MESH_MESSAGE_AUTHOR_CHUNK_SIZE: usize = 500;

const MESSAGE_AUTHOR_SUBSCRIPTION_PREFIX: &str = "ndr-protocol-messages-";

/// Split message authors, most important first, into filters that each fit
/// one request. The layout is the protocol library's: an author stays in the
/// chunk its key selects, so one ratchet step changes one chunk, and past
/// `MESSAGE_AUTHOR_SUBSCRIPTION_LIMIT` the least important authors (keys that
/// only have missed messages from older ratchet periods) are left out.
pub(in crate::core) fn message_author_subscription_filters(
    authors: impl IntoIterator<Item = PublicKey>,
    chunk_size: usize,
) -> Vec<Filter> {
    nostr_double_ratchet::build_message_author_subscription_filters(
        authors,
        chunk_size,
        nostr_double_ratchet::MESSAGE_AUTHOR_SUBSCRIPTION_LIMIT,
    )
    .filters
}

fn is_message_author_filter(filter: &Filter) -> bool {
    filter.authors.is_some()
        && filter.ids.is_none()
        && filter.generic_tags.is_empty()
        && filter.kinds.as_ref().is_some_and(|kinds| {
            kinds.len() == 1 && kinds.contains(&Kind::from(MESSAGE_EVENT_KIND as u16))
        })
}

/// Filters that travel together, and message-author filters that each need a
/// request of their own because the frame limit applies to a whole request.
pub(in crate::core) fn split_message_author_filters(
    filters: Vec<Filter>,
) -> (Vec<Filter>, Vec<Filter>) {
    filters
        .into_iter()
        .partition(|filter| !is_message_author_filter(filter))
}

pub(super) fn is_protocol_subscription_id(id: &str) -> bool {
    id == PROTOCOL_SUBSCRIPTION_ID || id.starts_with(MESSAGE_AUTHOR_SUBSCRIPTION_PREFIX)
}

/// Forget every protocol subscription cached for this relay, so the next
/// reconcile requests them again on the current socket.
pub(super) async fn evict_protocol_subscriptions(relay: &nostr_sdk::Relay) {
    for id in relay.subscriptions().await.into_keys() {
        if is_protocol_subscription_id(id.as_str()) {
            let _ = relay.unsubscribe(&id).await;
        }
    }
}

/// Apply the live plan: one request for the small shared filters and one per
/// message-author chunk.
pub(super) async fn subscribe_protocol_filters(
    client: &Client,
    filters: Vec<Filter>,
) -> Result<(), String> {
    let (shared, chunks) = split_message_author_filters(filters);
    // Reconcile the chunks even when the shared request failed, so an
    // obsolete chunk never outlives the plan that dropped it.
    let shared = subscribe_protocol_filters_with_id(
        client,
        SubscriptionId::new(PROTOCOL_SUBSCRIPTION_ID),
        shared,
    )
    .await;
    let chunks = subscribe_message_author_chunks(client, chunks).await;
    shared.and(chunks)
}

/// Keep every chunk a relay already has, drop the ones no longer wanted and
/// request only the missing ones. A chunk is recognised by its authors, not
/// by its position, so a changed layout never re-requests unchanged chunks.
async fn subscribe_message_author_chunks(
    client: &Client,
    chunks: Vec<Filter>,
) -> Result<(), String> {
    let mut attempted = 0usize;
    let mut accepted = 0usize;
    let mut last_error = None;
    for relay in client.relays().await.values() {
        let mut missing = chunks.iter().collect::<Vec<_>>();
        let mut used = HashSet::new();
        for (id, existing) in relay.subscriptions().await {
            if !id.as_str().starts_with(MESSAGE_AUTHOR_SUBSCRIPTION_PREFIX) {
                continue;
            }
            let kept = match existing.as_slice() {
                [filter] => missing.iter().position(|chunk| *chunk == filter),
                _ => None,
            };
            match kept {
                Some(index) => {
                    // The status watcher invalidates these on reconnect or
                    // CLOSED. Reissuing a healthy request replays history.
                    missing.swap_remove(index);
                    used.insert(id.to_string());
                }
                // Remove obsolete chunks even on an offline relay, so the SDK
                // cannot restore the old plan when its socket reconnects.
                None => {
                    let _ = relay.unsubscribe(&id).await;
                }
            }
        }
        if chunks.is_empty() || relay.status() != RelayStatus::Connected {
            continue;
        }
        attempted = attempted.saturating_add(1);
        let mut relay_error = None;
        let mut next = 0usize;
        for chunk in missing {
            let id = loop {
                let id = format!("{MESSAGE_AUTHOR_SUBSCRIPTION_PREFIX}{next}");
                next += 1;
                if used.insert(id.clone()) {
                    break id;
                }
            };
            if let Err(error) = relay
                .subscribe_with_id(
                    SubscriptionId::new(id),
                    vec![chunk.clone()],
                    SubscribeOptions::default(),
                )
                .await
            {
                relay_error = Some(error.to_string());
            }
        }
        match relay_error {
            None => accepted = accepted.saturating_add(1),
            Some(error) => last_error = Some(error),
        }
    }
    if chunks.is_empty() || accepted > 0 {
        Ok(())
    } else if attempted == 0 {
        Err("no connected relays".to_string())
    } else {
        Err(last_error.unwrap_or_else(|| "no relay accepted message subscriptions".to_string()))
    }
}

#[cfg(test)]
mod tests;
