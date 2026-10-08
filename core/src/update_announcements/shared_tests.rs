use super::*;
use hashtree_resolver::RootResolver;
use nostr::{EventBuilder, Filter, Kind, Tag, TagKind};
use nostr_pubsub::InMemoryEventBus;

#[test]
fn shared_message_servers_work_without_peer_observations() {
    const CHILD: &str = "IRIS_TEST_SHARED_UPDATE_SERVERS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Exercise the app's registered provider path without another core
        // instance in this parallel test process replacing its registration.
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "update_announcements::shared_tests::shared_message_servers_work_without_peer_observations",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env_remove("IRIS_UPDATE_HTREE_REF")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let relay = crate::local_relay::TestRelay::start_with_bind("127.0.0.1:0").unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let keys = Keys::generate();
        let reference = UpdateRef {
            npub: keys.public_key().to_bech32().unwrap(),
            tree_name: "releases/shared-client".into(),
            path: None,
        };
        let client = nostr_sdk::Client::new(Keys::generate());
        let relay_urls = vec![relay.url().to_string(), "ws://127.0.0.1:9".into()];
        let (providers, error) = shared_update_providers(
            Some(Arc::new(InMemoryEventBus::new())), // Started peer transport, no response.
            Some(client.clone()),
            relay_urls.clone(),
        )
        .await;
        assert!(error.is_none());
        client.wait_for_connection(Duration::from_secs(2)).await;
        // Keep an ordinary application subscription alive while update checks
        // request fresh copies through that same connected SDK client.
        client
            .subscribe(Filter::new().author(keys.public_key()), None)
            .await
            .unwrap();
        let event = EventBuilder::new(Kind::Custom(30064), "")
            .tags([
                Tag::identifier(&reference.tree_name),
                Tag::custom(TagKind::Custom("hash".into()), ["42".repeat(32)]),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        let sent = client.send_event(&event).await.unwrap();
        assert_eq!(sent.success.len(), 1);
        register_update_providers(&providers, relay_urls);
        for _ in 0..2 {
            let (_, updater) = build_secure_update_updater().await.unwrap();
            let root = updater
                .resolver()
                .resolve(&reference.resolver_key())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(root.hash, [0x42; 32]);
        }
        client.shutdown().await;
    });
}
