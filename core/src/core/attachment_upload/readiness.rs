use super::*;
use std::future::Future;
use tokio::sync::watch;

#[derive(Clone)]
pub(super) enum State {
    Pending,
    Ready(Weak<RoutedStore<AttachmentChunkStore>>),
    Closed,
}

fn ready(state: &State) -> anyhow::Result<Option<Arc<RoutedStore<AttachmentChunkStore>>>> {
    match state {
        State::Pending => Ok(None),
        State::Ready(route) => Ok(route.upgrade()),
        State::Closed => anyhow::bail!("attachment account is closed"),
    }
}

async fn while_open<T>(
    changes: &mut watch::Receiver<State>,
    work: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    tokio::pin!(work);
    loop {
        ready(&changes.borrow())?;
        tokio::select! {
            biased;
            changed = changes.changed() => {
                changed.map_err(|_| anyhow::anyhow!("attachment account is closed"))?;
            }
            result = &mut work => {
                ready(&changes.borrow())?;
                return result;
            }
        }
    }
}

async fn wait_for_route(
    changes: &mut watch::Receiver<State>,
) -> anyhow::Result<Arc<RoutedStore<AttachmentChunkStore>>> {
    loop {
        if let Some(route) = ready(&changes.borrow_and_update())? {
            return Ok(route);
        }
        changes
            .changed()
            .await
            .map_err(|_| anyhow::anyhow!("attachment account is closed"))?;
    }
}

/// A cold-start HTTP miss may precede the account's peer transport. Healthy
/// cache/HTTP reads never wait. There is one event-driven retry, with no polling
/// or detached task; the caller's existing reader slot bounds waiting requests.
pub(super) async fn read<S: Store + 'static>(
    cid: &Cid,
    local: Option<Arc<AttachmentChunkStore>>,
    fallback: Arc<S>,
    max_bytes: u64,
    maximum_wait: Duration,
) -> anyhow::Result<Vec<u8>> {
    let Some(local) = local else {
        return read_hashtree_attachment(cid, fallback, max_bytes).await;
    };
    let _lease = local.reading()?;
    let mut changes = local.readiness.subscribe();
    let route = ready(&changes.borrow())?;
    if let Some(route) = route {
        return while_open(
            &mut changes,
            read_hashtree_attachment(cid, route, max_bytes),
        )
        .await;
    }
    let initial_error = match while_open(
        &mut changes,
        read_hashtree_attachment(cid, fallback, max_bytes),
    )
    .await
    {
        Ok(bytes) => return Ok(bytes),
        Err(error) => error,
    };
    let route = match tokio::time::timeout(maximum_wait, wait_for_route(&mut changes)).await {
        Ok(result) => result?,
        Err(_) => return Err(initial_error),
    };
    // The route is attached to this exact store, never to a later global account.
    anyhow::ensure!(
        Arc::ptr_eq(route.primary(), &local),
        "attachment account changed"
    );
    while_open(
        &mut changes,
        read_hashtree_attachment(cid, route, max_bytes),
    )
    .await
}

#[cfg(test)]
#[path = "readiness_tests.rs"]
mod tests;
