use super::*;

const PROTOCOL_FETCH_MIN_INTERVAL_SECS: u64 = 30;
pub(super) const PROTOCOL_BACKFILL_AUTHOR_BATCH_SIZE: usize = 64;
#[cfg(not(test))]
pub(super) const NEW_MESSAGE_AUTHOR_DELAYED_BACKFILL_MS: [u64; 2] = [2_500, 10_000];
#[cfg(test)]
pub(super) const NEW_MESSAGE_AUTHOR_DELAYED_BACKFILL_MS: [u64; 1] = [50];

fn unique_pubkeys(pubkeys: impl IntoIterator<Item = PublicKey>) -> Vec<PublicKey> {
    let mut seen = HashSet::new();
    pubkeys
        .into_iter()
        .filter(|pubkey| seen.insert(*pubkey))
        .collect()
}

pub(super) fn direct_message_history_filter(
    author_pubkeys: impl IntoIterator<Item = PublicKey>,
) -> Filter {
    Filter::new()
        .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
        .authors(unique_pubkeys(author_pubkeys))
}

pub(super) fn direct_message_recipient_history_filter(
    recipient_pubkeys: impl IntoIterator<Item = PublicKey>,
    now: UnixSeconds,
) -> Filter {
    Filter::new()
        .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
        .pubkeys(unique_pubkeys(recipient_pubkeys))
        .since(Timestamp::from(
            now.get()
                .saturating_sub(DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS),
        ))
        .limit(DEVICE_INVITE_DISCOVERY_LIMIT)
}

pub(super) fn group_sender_key_history_filter(
    author_pubkeys: impl IntoIterator<Item = PublicKey>,
) -> Filter {
    Filter::new()
        .kind(Kind::from(GROUP_SENDER_KEY_MESSAGE_KIND as u16))
        .authors(unique_pubkeys(author_pubkeys))
}

pub(super) fn protocol_event_summary(events: &[Event]) -> String {
    events
        .iter()
        .take(16)
        .map(|event| format!("{}:{}:{}", event.kind.as_u16(), event.pubkey, event.id))
        .collect::<Vec<_>>()
        .join(",")
}

impl AppCore {
    pub(super) fn protocol_fetch_rate_limit_delay(&self) -> Option<Duration> {
        let last_started = self
            .protocol_subscription_runtime
            .protocol_fetch_last_started_at?;
        let min_interval = Duration::from_secs(PROTOCOL_FETCH_MIN_INTERVAL_SECS);
        let elapsed = Instant::now().saturating_duration_since(last_started);
        (elapsed < min_interval).then(|| min_interval - elapsed)
    }
}
