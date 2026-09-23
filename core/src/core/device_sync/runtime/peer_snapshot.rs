use fips_core::{FipsEndpointError, FipsEndpointPeer};
use std::future::Future;
use std::time::Duration;

// A busy endpoint can time out while its transport remains alive. Keep both
// nearby delivery and recent-peer observation running after that transient
// error. Runtime shutdown still cancels their tasks or closes the endpoint.
pub(super) async fn query<F, Fut>(snapshot: F) -> Result<Vec<FipsEndpointPeer>, FipsEndpointError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<FipsEndpointPeer>, FipsEndpointError>>,
{
    query_with_timeout_handler(snapshot, || {}).await
}

pub(super) async fn query_with_timeout_handler<F, Fut, OnTimeout>(
    mut snapshot: F,
    mut on_timeout: OnTimeout,
) -> Result<Vec<FipsEndpointPeer>, FipsEndpointError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<FipsEndpointPeer>, FipsEndpointError>>,
    OnTimeout: FnMut(),
{
    loop {
        match snapshot().await {
            Err(error @ FipsEndpointError::Timeout { .. }) => {
                crate::perflog!("fips.peers.retry error={error}");
                on_timeout();
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_snapshot_recovers_after_timeout_and_stops_when_closed() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut attempts = 0;
        let timed_out = std::cell::Cell::new(false);
        let recovered = runtime.block_on(query_with_timeout_handler(
            || {
                attempts += 1;
                std::future::ready(if attempts == 1 {
                    Err(FipsEndpointError::Timeout {
                        operation: "peer snapshot",
                    })
                } else {
                    assert!(timed_out.get(), "clear stale status before retrying");
                    Ok(Vec::new())
                })
            },
            || timed_out.set(true),
        ));
        assert!(recovered.is_ok());
        assert_eq!(attempts, 2);

        attempts = 0;
        let closed = runtime.block_on(query(|| {
            attempts += 1;
            std::future::ready(Err(FipsEndpointError::Closed))
        }));
        assert!(matches!(closed, Err(FipsEndpointError::Closed)));
        assert_eq!(attempts, 1);
    }
}
