#[test]
fn signer_login_rejects_partial_device_list_without_end_of_stored_events() {
    use futures_util::{SinkExt, StreamExt};
    let owner = Keys::generate();
    let temp = tempfile::TempDir::new().unwrap();
    let (mut core, messages, updates) = signer_test_core(temp.path(), Vec::new());
    let event = app_keys_event(&owner, &[&Keys::generate()], unix_now().get());
    let listener = core
        .runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    core.preferences.nostr_relay_urls = vec![format!("ws://{}", listener.local_addr().unwrap())];
    let incomplete_server = core.runtime.spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            let Ok(raw) = message.into_text() else {
                continue;
            };
            let request: serde_json::Value = serde_json::from_str(&raw).unwrap();
            if request[0] == "REQ" {
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        serde_json::json!(["EVENT", request[1], event]).to_string(),
                    ))
                    .await
                    .unwrap();
                // A partial response is not proof that this is the current head.
                // Deliberately keep the connection open without sending EOSE.
                std::future::pending::<()>().await;
            }
        }
    });
    core.begin_signer_login(&owner.public_key().to_hex());
    pump_signer_core_until(&mut core, &messages, |core| {
        !core.state.busy.restoring_session
    });
    assert!(core.logged_in.is_none());
    assert_eq!(
        core.state.toast.as_deref(),
        Some("Could not check all message servers. Try again.")
    );
    assert!(!updates
        .try_iter()
        .any(|update| matches!(update, AppUpdate::SignerLoginSignEvent { .. })));
    incomplete_server.abort();
}

#[test]
fn signer_roster_preserves_conflicting_heads_from_an_incomplete_server() {
    use futures_util::{SinkExt, StreamExt};
    let relay = crate::local_relay::TestRelay::start();
    let owner = Keys::generate();
    let temp = tempfile::TempDir::new().unwrap();
    let (core, _, _) = signer_test_core(temp.path(), vec![relay.url().into()]);
    let now = unix_now().get();
    let first = app_keys_event(&owner, &[&Keys::generate()], now);
    let competing = app_keys_event(&owner, &[&Keys::generate()], now);
    publish_signer_test_event(&core, &relay, &first);
    let listener = core.runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let partial_url = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let event = competing.clone();
    let partial = core.runtime.spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            let Ok(raw) = message.into_text() else { continue };
            let request: serde_json::Value = serde_json::from_str(&raw).unwrap();
            if request[0] == "REQ" {
                socket.send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!(["EVENT", request[1], event]).to_string(),
                )).await.unwrap();
                std::future::pending::<()>().await;
            }
        }
    });
    let result = core.runtime.block_on(super::account_signer_relay::fetch_signer_roster(
        owner.public_key(), &[RelayUrl::parse(relay.url()).unwrap(), partial_url],
    ));
    assert_eq!(result.unwrap_err(), "Conflicting device lists. Try again later.");
    partial.abort();
}

#[test]
fn signer_authorization_rejects_device_list_that_cannot_fit_handshake_proof() {
    let owner = Keys::generate();
    let old_device = Keys::generate();
    let new_device = Keys::generate();
    let now = unix_now().get();
    let mut previous = AppKeys::new(vec![DeviceEntry::new(old_device.public_key(), now - 1)])
        .get_event_at(owner.public_key(), now - 1);
    previous.tags.push(
        nostr::Tag::parse([
            nostr_double_ratchet::APP_KEYS_ENCRYPTED_DEVICE_LABELS_FACT,
            &"x".repeat(35 * 1024),
        ])
        .unwrap(),
    );
    previous.id = None;
    let event = previous.sign_with_keys(&owner).unwrap();
    assert!(
        AppKeys::from_event(&event).is_ok(),
        "the roster itself is valid"
    );
    assert!(prepare_signer_authorization(owner.public_key(), new_device.public_key(), Some(&event), now).is_err(), "reject before prompting for a signature when the authorization cannot travel with the handshake");

    // Exercise the actual lookup-to-prompt boundary, not only the preparer.
    let relay = crate::local_relay::TestRelay::start();
    let temp = tempfile::TempDir::new().unwrap();
    let (mut core, messages, updates) = signer_test_core(temp.path(), vec![relay.url().into()]);
    publish_signer_test_event(&core, &relay, &event);
    core.begin_signer_login(&owner.public_key().to_hex());
    pump_signer_core_until(&mut core, &messages, |core| {
        !core.state.busy.restoring_session
    });
    assert!(core.logged_in.is_none());
    assert_eq!(
        core.state.toast.as_deref(),
        Some("Device list is too large.")
    );
    assert!(!updates
        .try_iter()
        .any(|update| matches!(update, AppUpdate::SignerLoginSignEvent { .. })));
}
