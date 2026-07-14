use std::sync::Arc;
use std::time::Duration;

use fips_core::FipsEndpoint;
use nostr_pubsub_fips::FipsPubsubClient;
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
