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
