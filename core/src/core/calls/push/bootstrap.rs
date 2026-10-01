//! Bounded normal invite-response discovery. No URLs or recipient keys come from a push.
use super::*;

const LOOKUP_BUDGET: Duration = Duration::from_millis(1800);

pub(super) async fn fetch(id: EventId, urls: Vec<String>) -> Option<Event> {
    search(Some(id), None, urls, |event| Some(event.clone())).await
}

async fn search<T>(
    id: Option<EventId>,
    recipient: Option<PublicKey>,
    urls: Vec<String>,
    mut accept: impl FnMut(&Event) -> Option<T>,
) -> Option<T> {
    if urls.is_empty() || (id.is_none() && recipient.is_none()) {
        return None;
    }
    let client = Client::default();
    let result = tokio::time::timeout(LOOKUP_BUDGET, async {
        for url in urls {
            let _ = client.add_relay(url).await;
        }
        let mut notifications = client.notifications();
        client.connect().await;
        let base = Filter::new().kind(Kind::from(INVITE_RESPONSE_KIND as u16));
        if let Some(id) = id {
            client
                .subscribe(base.clone().id(id).limit(1), None)
                .await
                .ok()?;
        }
        if let Some(recipient) = recipient {
            // Same locally derived recipient as the normal protocol subscription.
            client
                .subscribe(base.pubkey(recipient).limit(64), None)
                .await
                .ok()?;
        }
        let mut seen = HashSet::new();
        loop {
            match notifications.recv().await.ok()? {
                RelayPoolNotification::Event { event, .. }
                    if (id == Some(event.id)
                        || recipient.is_some_and(|key| {
                            event.tags.public_keys().any(|target| *target == key)
                        }))
                        && event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND
                        && event.verify().is_ok()
                        && seen.insert(event.id) =>
                {
                    // Recipient ephemeral key and encrypted owner authorization are
                    // checked by the existing invite-response engine before use.
                    if let Some(result) = accept(&event) {
                        return Some(result);
                    }
                    if seen.len() >= 64 {
                        return None;
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .ok()
    .flatten();
    let _ = tokio::time::timeout(Duration::from_millis(100), client.shutdown()).await;
    result
}

pub(super) fn recover_preview(
    data_dir: String,
    local_owner: PublicKey,
    keys: Keys,
    caller_owner: PublicKey,
    caller_device: PublicKey,
    wake: CallWake,
    urls: Vec<String>,
) -> Option<ProtocolDecryptedMessage> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    // FFI may be called on a small Swift cooperative stack. Poll network futures
    // on a bounded dedicated stack, never on the caller's stack/runtime.
    std::thread::Builder::new()
        .name("call-bootstrap".into())
        .stack_size(4 * 1024 * 1024)
        .spawn(move || {
            let recover = || {
                let mut engine = super::super::super::mobile_push::direct_message_preview_engine(
                    &data_dir,
                    local_owner,
                    &keys,
                )?;
                let recipient = engine.local_invite_response_pubkey();
                let id = wake
                    .bootstrap_event_id
                    .as_deref()
                    .and_then(|id| EventId::from_hex(id).ok());
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()?;
                runtime.block_on(search(id, recipient, urls, |response| {
                    let events = [vec![response.clone()], wake.events.clone()].concat();
                    let message =
                        super::super::super::mobile_push::preview_direct_messages_in_engine(
                            &mut engine,
                            &events,
                        )?;
                    (message.sender == caller_owner
                        && message.sender_device == Some(caller_device)
                        && decrypted_offer(&message).is_some())
                    .then_some(message)
                }))
            };
            let _ = tx.send(recover());
        })
        .ok()?;
    rx.recv_timeout(Duration::from_secs(2)).ok().flatten()
}
