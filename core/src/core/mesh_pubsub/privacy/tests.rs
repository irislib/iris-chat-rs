use super::*;

#[test]
fn live_peer_advert_queries_have_one_anonymous_subject_and_no_own_receive_context() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let own = Keys::generate();
        let peers = [Keys::generate().public_key(), Keys::generate().public_key()];
        let (relay, wire) =
            crate::core::publish_helpers::authenticated_relay(own.public_key(), false).await;
        let own_client = crate::core::publish_helpers::identity_receiver(&own, &relay, &wire).await;
        let advert = EventBuilder::new(Kind::Custom(37195), "own public endpoint advert")
            .sign_with_keys(&own)
            .unwrap();
        own_client.send_event(&advert).await.unwrap();
        let mut config = Config::new();
        config.node.control.enabled = false;
        config.node.discovery.nostr.enabled = false;
        config.node.discovery.local.enabled = false;
        config.node.discovery.lan.enabled = false;
        let endpoint = Arc::new(
            FipsEndpoint::builder()
                .config(config)
                .without_system_tun()
                .bind()
                .await
                .unwrap(),
        );
        let adapter = PrivateMeshProviders::new(Vec::new(), None, Vec::new(), vec![relay]);
        for peer in peers {
            adapter
                .discover(
                    vec![Filter::new().kind(Kind::Custom(37195)).author(peer)],
                    endpoint.clone(),
                    flume::unbounded().0,
                )
                .await;
        }
        let rows = wire.lock().unwrap().clone();
        let identified = rows
            .iter()
            .filter(|(_, event)| event[0] == "EVENT" && event[1]["id"] == advert.id.to_hex())
            .map(|(connection, _)| *connection)
            .collect::<HashSet<_>>();
        assert!(!identified.is_empty());
        let mut subjects = BTreeMap::<usize, HashSet<PublicKey>>::new();
        for (connection, event) in rows {
            if event[0] == "REQ" {
                for peer in peers {
                    if event.to_string().contains(&peer.to_hex()) {
                        assert!(!identified.contains(&connection));
                        assert!(!event.to_string().contains(&own.public_key().to_hex()));
                        subjects.entry(connection).or_default().insert(peer);
                    }
                }
            }
            if event[0] == "AUTH" && !identified.contains(&connection) {
                assert_ne!(event[1]["pubkey"], own.public_key().to_hex());
            }
        }
        assert_eq!(subjects.len(), 2);
        assert!(subjects.values().all(|keys| keys.len() == 1));
        own_client.shutdown().await;
        endpoint.shutdown().await.unwrap();
    });
}
