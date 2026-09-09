use std::sync::Arc;
use std::time::Duration;

use nostr_pubsub::{NostrEventSubscriber, QueryEvent};
use nostr_pubsub_fips::FipsPubsubClient;
use nostr_pubsub_relay::RelayEventBus;
use tokio::time::sleep;

pub(super) async fn run_update_announcement_subscription(
    pubsub: Arc<FipsPubsubClient>,
    filter: nostr_pubsub::Filter,
) {
    loop {
        let _ = crate::update_announcements::refresh_update_announcements(pubsub.as_ref()).await;
        let Ok(mut subscription) = pubsub.subscribe(vec![filter.clone()]).await else {
            sleep(Duration::from_secs(1)).await;
            continue;
        };

        while let Some(event) = subscription.recv().await {
            let _ = crate::update_announcements::ingest_update_announcement(
                event.event.as_event().clone(),
            );
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
