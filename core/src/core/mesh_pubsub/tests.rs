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

#[test]
fn identified_mesh_never_attaches_retained_private_group_replay() {
    for change in ["block", "remove", "delete", "reject"] {
        let owner = Keys::generate();
        let peer = Keys::generate();
        let (mut core, _, _dir) = crate::core::tests::logged_in_test_core_with_updates(
            "group-replay-permissions",
            &owner,
            &Keys::generate(),
        );
        core.create_group("replay permissions", &[peer.public_key().to_hex()]);
        let group = core.groups.keys().next().unwrap().clone();
        let chat = group_chat_id(&group);
        let event = EventBuilder::new(
            Kind::Custom(MESSAGE_EVENT_KIND as u16),
            "opaque signed group envelope",
        )
        .tags([
            nostr::Tag::parse(["v", "2"]).unwrap(),
            nostr::Tag::public_key(owner.public_key()),
        ])
        .sign_with_keys(&Keys::generate())
        .unwrap();
        core.apply_runtime_text_message(
            peer.public_key(),
            Some(chat.clone()),
            "decrypted message".into(),
            unix_now().get(),
            None,
            Some("message".into()),
            Some(event.id.to_hex()),
        );
        core.persist_app_state().unwrap();
        core.retain_mesh_group_envelope(&event, &chat);
        let (endpoint, client) = core.runtime.block_on(fixture());
        let client = Arc::new(client);
        let (tcp, _) = DeviceSyncTcpSender::test_channel(1, 1024);
        core.install_device_sync_sender_for_test(endpoint.clone(), tcp, Vec::new());
        core.device_sync.as_mut().unwrap().pubsub = Some(client.clone());
        core.reconcile_mesh_protocol_subscriptions();
        assert!(core
            .device_sync
            .as_ref()
            .unwrap()
            .protocol_subscriptions
            .replay_source
            .is_none());
        let attached = core.mesh_group_replay_source().unwrap();
        let query =
            |core: &AppCore, source: &Arc<storage::mesh_group_envelopes::GroupEnvelopeReplay>| {
                core.runtime
                    .block_on(source.query(
                        vec![Filter::new().id(event.id)],
                        nostr_pubsub::QueryOptions { limit: Some(1) },
                    ))
                    .unwrap()
                    .events
            };
        assert_eq!(query(&core, &attached).len(), 1);
        // A routine save/upsert must preserve the envelope. A changed source
        // body or a deletion must not preserve it.
        for _ in 0..3 {
            core.persist_app_state().unwrap();
        }
        assert_eq!(query(&core, &attached).len(), 1);
        core.reconcile_mesh_protocol_subscriptions();
        assert!(
            core.device_sync
                .as_ref()
                .unwrap()
                .protocol_subscriptions
                .replay_source
                .is_none(),
            "normal saves must not attach private replay to an identified provider client"
        );
        match change {
            "block" => core.set_user_blocked(&peer.public_key().to_hex(), true),
            "remove" => core.remove_group_member(&group, &peer.public_key().to_hex()),
            "delete" => core.delete_chat(&chat),
            "reject" => {
                let rejection = nostr_identity::build_private_setting(
                    owner.public_key(), "requestDecision", &chat,
                    &serde_json::json!({"chatId":chat,"decision":"rejected","updatedAtMs":unix_now_ms().saturating_add(1)}),
                ).unwrap();
                assert!(core.merge_private_setting(&rejection));
            }
            _ => unreachable!(),
        }
        // Some operations can stop the runtime completely; either outcome
        // revokes its durable replay source.
        if let Some(source) = core
            .device_sync
            .as_ref()
            .and_then(|mesh| mesh.protocol_subscriptions.replay_source.as_ref())
        {
            assert!(
                query(&core, source).is_empty(),
                "{change} must revoke peer replay"
            );
        }
        core.stop_device_sync_now();
        core.runtime.block_on(client.shutdown_shared());
        core.runtime.block_on(endpoint.shutdown()).unwrap();
    }
}
