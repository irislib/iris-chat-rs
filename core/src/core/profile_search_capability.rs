use super::direct_chat_capability::{resolve_current_app_keys, CurrentAppKeysResolution};
use super::protocol::fetch_events_for_filters;
use super::*;

pub(super) const MAX_SEARCH_CAPABILITY_CANDIDATES: usize = 64;
const SEARCH_CAPABILITY_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve a bounded batch with the same signature, timestamp and conflict
/// checks used when opening a direct chat. Empty snapshots must reach the
/// cache too, so a revoked device list removes a previously visible person.
pub(super) async fn fetch_search_app_keys(client: &Client, owners: Vec<PublicKey>) -> Vec<Event> {
    let owners = owners
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if owners.is_empty() {
        return Vec::new();
    }
    let filters = owners
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .chunks(32)
        .map(|chunk| {
            Filter::new()
                .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
                .authors(chunk.iter().copied())
                .limit(32 * chunk.len())
        })
        .collect();
    let events = fetch_events_for_filters(client, filters, SEARCH_CAPABILITY_TIMEOUT)
        .await
        .unwrap_or_default();
    resolve_search_app_keys(events, &owners, unix_now().get())
}

fn resolve_search_app_keys(
    events: Vec<Event>,
    owners: &std::collections::BTreeSet<PublicKey>,
    now: u64,
) -> Vec<Event> {
    let mut by_owner = BTreeMap::<PublicKey, Vec<Event>>::new();
    for event in events {
        if owners.contains(&event.pubkey) {
            by_owner.entry(event.pubkey).or_default().push(event);
        }
    }
    by_owner
        .into_iter()
        .filter_map(
            |(owner, events)| match resolve_current_app_keys(events, owner, now) {
                CurrentAppKeysResolution::Found { event, .. } => Some(*event),
                CurrentAppKeysResolution::Missing | CurrentAppKeysResolution::Ambiguous => None,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_keys(owner: &Keys, device: Option<&Keys>, time: u64) -> Event {
        AppKeys::new(
            device
                .map(|key| DeviceEntry::new(key.public_key(), time))
                .into_iter()
                .collect(),
        )
        .get_event_at(owner.public_key(), time)
        .sign_with_keys(owner)
        .unwrap()
    }

    #[test]
    fn search_capability_rejects_forged_future_conflicting_and_unrequested_events() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let other = Keys::generate();
        let requested = [owner.public_key()].into_iter().collect();
        let now = unix_now().get();
        let valid = app_keys(&owner, Some(&device), now);
        let mut forged = valid.clone();
        forged.sig = app_keys(&other, Some(&device), now).sig;
        assert!(resolve_search_app_keys(
            vec![
                forged,
                app_keys(&owner, Some(&device), now + 3600),
                app_keys(&other, Some(&device), now)
            ],
            &requested,
            now
        )
        .is_empty());
        assert!(resolve_search_app_keys(
            vec![valid.clone(), app_keys(&owner, Some(&other), now)],
            &requested,
            now
        )
        .is_empty());
        let revoked = app_keys(&owner, None, now + 1);
        assert_eq!(
            resolve_search_app_keys(vec![valid, revoked.clone()], &requested, now + 1),
            vec![revoked]
        );
    }

    #[test]
    fn search_capability_fetches_verified_devices_and_revocations_from_relay() {
        let relay = crate::local_relay::TestRelay::start();
        let owner = Keys::generate();
        let device = Keys::generate();
        let unknown = Keys::generate();
        let client = Client::new(Keys::generate());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let urls = relay_urls_from_strings(&[relay.url().to_string()]);
            ensure_session_relays_configured(&client, &urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(2)).await;
            let now = unix_now().get();
            let supported = app_keys(&owner, Some(&device), now);
            client.send_event(&supported).await.unwrap();
            assert_eq!(
                fetch_search_app_keys(&client, vec![owner.public_key(), unknown.public_key()])
                    .await,
                vec![supported]
            );
            let revoked = app_keys(&owner, None, now + 1);
            client.send_event(&revoked).await.unwrap();
            assert_eq!(
                fetch_search_app_keys(&client, vec![owner.public_key()]).await,
                vec![revoked]
            );
            client.disconnect().await;
        });
    }
}
