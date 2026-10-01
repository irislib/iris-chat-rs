//! Exact-ID lookup for an oversized first-call bootstrap. No URLs come from a push.
use super::*;

const LOOKUP_BUDGET: Duration = Duration::from_millis(1800);

pub(super) async fn fetch(id: EventId, urls: Vec<String>) -> Option<Event> {
    if urls.is_empty() {
        return None;
    }
    let client = Client::default();
    let result = tokio::time::timeout(LOOKUP_BUDGET, async {
        for url in urls {
            let _ = client.add_relay(url).await;
        }
        let mut notifications = client.notifications();
        client.connect().await;
        client
            .subscribe(
                Filter::new()
                    .id(id)
                    .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
                    .limit(1),
                None,
            )
            .await
            .ok()?;
        loop {
            match notifications.recv().await.ok()? {
                RelayPoolNotification::Event { event, .. }
                    if event.id == id
                        && event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND
                        && event.verify().is_ok() =>
                {
                    // Recipient ephemeral key and encrypted owner authorization are
                    // checked by the existing invite-response engine before use.
                    return Some((*event).clone());
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

pub(super) fn fetch_blocking(id: EventId, urls: Vec<String>) -> Option<Event> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    // FFI may be called on a small Swift cooperative stack. Poll network futures
    // on a bounded dedicated stack, never on the caller's stack/runtime.
    std::thread::Builder::new()
        .name("call-bootstrap".into())
        .stack_size(4 * 1024 * 1024)
        .spawn(move || {
            let event = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()
                .and_then(|runtime| runtime.block_on(fetch(id, urls)));
            let _ = tx.send(event);
        })
        .ok()?;
    rx.recv_timeout(Duration::from_secs(2)).ok().flatten()
}
