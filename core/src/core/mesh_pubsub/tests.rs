use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct ObservedSubscription {
    status: Option<SubscriptionDeliveryStatus>,
    closed: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl NostrEventSubscription for ObservedSubscription {
    fn delivery_status(&self) -> Option<SubscriptionDeliveryStatus> {
        self.status
    }

    async fn close(self: Box<Self>) -> nostr_pubsub::Result<()> {
        tokio::task::yield_now().await;
        self.closed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn unchanged_filters_repair_only_known_terminal_subscriptions_and_await_close() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (endpoint, client) = fixture().await;
        let initial_subscriptions = client.active_subscription_count().unwrap();
        let filters = (1..=4)
            .map(|kind| Filter::new().kind(Kind::Custom(kind)))
            .collect::<Vec<_>>();
        let statuses = [
            None,
            Some(SubscriptionDeliveryStatus::Active),
            Some(SubscriptionDeliveryStatus::Closed),
            Some(SubscriptionDeliveryStatus::Lagged),
        ];
        let closed = statuses.map(|_| Arc::new(AtomicUsize::new(0)));
        let mut state = MeshProtocolSubscriptions {
            filters: filters.clone(),
            subscriptions: statuses
                .into_iter()
                .zip(closed.iter())
                .map(|(status, closed)| {
                    Box::new(ObservedSubscription {
                        status,
                        closed: closed.clone(),
                    }) as Box<dyn NostrEventSubscription>
                })
                .collect(),
            ..Default::default()
        };
        let (sender, _receiver) = flume::unbounded();
        state
            .update(&client, filters.clone(), sender.clone())
            .await
            .unwrap();
        assert_eq!(
            closed
                .each_ref()
                .map(|closed| closed.load(Ordering::SeqCst)),
            [0, 0, 1, 1]
        );
        assert_eq!(
            client.active_subscription_count().unwrap(),
            initial_subscriptions + 2
        );
        assert_eq!(state.subscriptions[0].delivery_status(), None);
        assert!(state.subscriptions[1..].iter().all(|subscription| {
            subscription.delivery_status() == Some(SubscriptionDeliveryStatus::Active)
        }));
        state.update(&client, filters, sender).await.unwrap();
        assert_eq!(
            client.active_subscription_count().unwrap(),
            initial_subscriptions + 2
        );
        drop(state);
        client.shutdown().await;
        endpoint.shutdown().await.unwrap();
    });
}

#[test]
fn failed_terminal_replacement_rebuilds_missing_slots_and_drains_terminal_handles() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (endpoint, client) = fixture().await;
        let filters = vec![Filter::new().kind(Kind::TextNote); 2];
        let closed = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let mut state = MeshProtocolSubscriptions {
            filters: filters.clone(),
            subscriptions: [
                SubscriptionDeliveryStatus::Closed,
                SubscriptionDeliveryStatus::Lagged,
            ]
            .into_iter()
            .zip(closed.iter())
            .map(|(status, closed)| {
                Box::new(ObservedSubscription {
                    status: Some(status),
                    closed: closed.clone(),
                }) as Box<dyn NostrEventSubscription>
            })
            .collect(),
            ..Default::default()
        };
        let (sender, _receiver) = flume::unbounded();
        client.shutdown_shared().await;
        assert!(state
            .update(&client, filters.clone(), sender.clone())
            .await
            .is_err());
        assert!(state.retry_needed);
        assert_eq!(
            state.subscriptions.len(),
            1,
            "failed replacement leaves a missing slot"
        );
        assert_eq!(closed[0].load(Ordering::SeqCst), 1);
        assert_eq!(closed[1].load(Ordering::SeqCst), 0);
        client.shutdown().await;

        let (replacement_endpoint, replacement) = fixture().await;
        let initial_subscriptions = replacement.active_subscription_count().unwrap();
        state
            .update(&replacement, filters.clone(), sender.clone())
            .await
            .unwrap();
        assert_eq!(state.subscriptions.len(), filters.len());
        assert!(!state.retry_needed);
        assert_eq!(
            closed[1].load(Ordering::SeqCst),
            1,
            "retry must await the remaining terminal handle"
        );
        state
            .update(&replacement, filters.clone(), sender.clone())
            .await
            .unwrap();
        assert_eq!(
            replacement.active_subscription_count().unwrap(),
            initial_subscriptions + filters.len()
        );

        let changed_close = Arc::new(AtomicUsize::new(0));
        state.subscriptions[0] = Box::new(ObservedSubscription {
            status: Some(SubscriptionDeliveryStatus::Lagged),
            closed: changed_close.clone(),
        });
        let changed_filters = vec![Filter::new().kind(Kind::ContactList)];
        state
            .update(&replacement, changed_filters.clone(), sender)
            .await
            .unwrap();
        assert_eq!(state.subscriptions.len(), changed_filters.len());
        assert_eq!(
            changed_close.load(Ordering::SeqCst),
            1,
            "changed filters must await terminal backlog too"
        );
        drop(state);
        replacement.shutdown().await;
        endpoint.shutdown().await.unwrap();
        replacement_endpoint.shutdown().await.unwrap();
    });
}

fn client_options() -> nostr_pubsub_fips::FipsPubsubClientOptions {
    nostr_pubsub_fips::FipsPubsubClientOptions {
        max_filters_per_subscription: 1,
        ..Default::default()
    }
}

async fn fixture() -> (Arc<fips_core::FipsEndpoint>, FipsPubsubClient) {
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.lan.enabled = false;
    let endpoint = Arc::new(
        fips_core::FipsEndpoint::builder()
            .config(config)
            .without_system_tun()
            .bind()
            .await
            .unwrap(),
    );
    let client = FipsPubsubClient::start(endpoint.clone(), client_options())
        .await
        .unwrap();
    (endpoint, client)
}
