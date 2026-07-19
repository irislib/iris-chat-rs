use std::sync::Arc;
use std::time::Duration;

use fips_core::FipsEndpoint;
use nostr_pubsub::{NostrEventSubscriber, QueryEvent};
use nostr_pubsub_fips::FipsPubsubClient;
use nostr_pubsub_relay::RelayEventBus;
use tokio::time::sleep;

pub(super) async fn run_update_announcement_subscription(
    endpoint: Arc<FipsEndpoint>,
    pubsub: Arc<FipsPubsubClient>,
    filter: nostr_pubsub::Filter,
) {
    loop {
        let Ok(subscribed_links) = connected_fips_links(&endpoint).await else {
            sleep(Duration::from_secs(1)).await;
            continue;
        };
        if subscribed_links.is_empty() {
            sleep(Duration::from_secs(1)).await;
            continue;
        }
        let _ = crate::update_announcements::refresh_update_announcements(pubsub.as_ref()).await;
        let Ok(mut subscription) = pubsub.subscribe(vec![filter.clone()]).await else {
            sleep(Duration::from_secs(1)).await;
            continue;
        };

        loop {
            tokio::select! {
                event = subscription.recv() => {
                    let Some(event) = event else {
                        break;
                    };
                    let _ = crate::update_announcements::ingest_update_announcement(
                        event.event.as_event().clone(),
                    );
                }
                _ = sleep(Duration::from_secs(1)) => {
                    if connected_fips_links(&endpoint)
                        .await
                        .is_ok_and(|links| links != subscribed_links)
                    {
                        break;
                    }
                }
            }
        }
    }
}

pub(super) async fn run_relay_update_announcement_subscription(
    pubsub: Arc<RelayEventBus>,
    filter: nostr_pubsub::Filter,
) {
    loop {
        let _ = crate::update_announcements::refresh_update_announcements(pubsub.as_ref()).await;
        let handler = Arc::new(|event: QueryEvent| {
            let _ = crate::update_announcements::ingest_update_announcement(
                event.event.as_event().clone(),
            );
        });
        let Ok(_subscription) =
            NostrEventSubscriber::subscribe(pubsub.as_ref(), vec![filter.clone()], handler).await
        else {
            sleep(Duration::from_secs(5)).await;
            continue;
        };

        loop {
            sleep(Duration::from_secs(60)).await;
            let _ =
                crate::update_announcements::refresh_update_announcements(pubsub.as_ref()).await;
        }
    }
}

async fn connected_fips_links(endpoint: &FipsEndpoint) -> Result<Vec<(String, u64)>, ()> {
    let mut links = endpoint
        .peers()
        .await
        .map_err(|_| ())?
        .into_iter()
        .filter(|peer| peer.connected)
        .map(|peer| (peer.npub, peer.link_id))
        .collect::<Vec<_>>();
    links.sort_unstable();
    Ok(links)
}
