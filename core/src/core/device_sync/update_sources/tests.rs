use super::*;
use hashtree_resolver::RootResolver;
use nostr::{EventBuilder, Kind, Tag, TagKind, ToBech32};

struct Fixture {
    core: AppCore,
    _directory: tempfile::TempDir,
    _updates: flume::Receiver<AppUpdate>,
    rendezvous: std::net::SocketAddrV4,
}

impl Fixture {
    fn new(relay: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (sender, updates) = flume::unbounded();
        let mut core = AppCore::new(
            sender,
            flume::unbounded().0,
            directory.path().to_string_lossy().into(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        let owner = Keys::generate();
        let device = Keys::generate();
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner),
            device_keys: device.clone(),
            client: Client::new(device),
            relay_urls: vec![RelayUrl::parse(relay).unwrap()],
            authorization_state: LocalAuthorizationState::Authorized,
        });
        core.preferences.nearby_enabled = false;
        core.preferences.nearby_lan_enabled = false;
        core.preferences.nostr_relay_urls = vec![relay.to_string()];
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(rendezvous) = socket.local_addr().unwrap() else {
            unreachable!()
        };
        Self {
            core,
            _directory: directory,
            _updates: updates,
            rendezvous,
        }
    }

    fn reconcile(&mut self, same_host: bool) {
        self.core
            .reconcile_update_mesh_for_test(same_host, self.rendezvous);
    }

    fn confirms_selected_server(&self) {
        let keys = Keys::generate();
        let key = format!(
            "{}/releases/lifecycle",
            keys.public_key().to_bech32().unwrap()
        );
        let event = EventBuilder::new(Kind::Custom(30064), "")
            .tags([
                Tag::identifier("releases/lifecycle"),
                Tag::custom(TagKind::Custom("hash".into()), ["42".repeat(32)]),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        let providers = self.core.update_sources.providers.clone();
        let client = self.core.logged_in.as_ref().unwrap().client.clone();
        self.core.runtime.block_on(async {
            client.wait_for_connection(Duration::from_secs(2)).await;
            client.send_event(&event).await.unwrap();
            let resolver = crate::update_announcements::AvailableUpdateResolver::new(
                providers,
                Duration::from_millis(150),
            )
            .unwrap();
            assert_eq!(
                resolver.resolve(&key).await.unwrap().unwrap().hash,
                [0x42; 32]
            );
        });
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.core.shutdown();
    }
}

#[test]
fn update_sources_follow_selected_servers_without_a_mesh() {
    let relay = crate::local_relay::TestRelay::start_with_bind("127.0.0.1:0").unwrap();
    let mut fixture = Fixture::new(relay.url());
    fixture.reconcile(false);
    assert!(fixture.core.device_sync.is_none());
    assert_eq!(fixture.core.update_sources.providers.len(), 1);
    fixture.confirms_selected_server();
    let original = fixture.core.update_sources.relay.as_ref().unwrap().clone();
    fixture.reconcile(false);
    assert!(Arc::ptr_eq(
        &original,
        fixture.core.update_sources.relay.as_ref().unwrap()
    ));
    fixture.core.set_nostr_relays(&[]);
    assert!(fixture.core.update_sources.providers.is_empty());
    assert!(fixture.core.update_sources.relay_urls.is_empty());
    assert!(fixture.core.update_sources.relay.is_none());
}

#[test]
fn update_sources_follow_server_changes_on_preserved_bluetooth_endpoint() {
    let first = crate::local_relay::TestRelay::start_with_bind("127.0.0.1:0").unwrap();
    let second = crate::local_relay::TestRelay::start_with_bind("127.0.0.1:0").unwrap();
    let mut fixture = Fixture::new(first.url());
    // No Nearby or known peers: the same-host endpoint must still use account servers.
    fixture.reconcile(true);
    let endpoint = fixture.core.device_sync.as_ref().unwrap().endpoint.clone();
    fixture.confirms_selected_server();
    let original_relay = Arc::downgrade(fixture.core.update_sources.relay.as_ref().unwrap());
    fixture.core.host_ble_attached = true;
    fixture.core.set_nostr_relays(&[second.url().to_string()]);
    fixture.reconcile(true);
    assert!(Arc::ptr_eq(
        &endpoint,
        &fixture.core.device_sync.as_ref().unwrap().endpoint
    ));
    assert!(original_relay.upgrade().is_none());
    assert_eq!(
        fixture.core.update_sources.relay_urls,
        vec![second.url().to_string()]
    );
    fixture.confirms_selected_server();
    fixture.core.set_nostr_relays(&[]);
    fixture.reconcile(true);
    assert!(Arc::ptr_eq(
        &endpoint,
        &fixture.core.device_sync.as_ref().unwrap().endpoint
    ));
    assert_eq!(fixture.core.update_sources.providers.len(), 1);
    assert!(fixture.core.update_sources.relay_urls.is_empty());
    fixture.core.logout();
    assert!(fixture.core.update_sources.providers.is_empty());
    assert!(fixture.core.update_sources.relay.is_none());
}
