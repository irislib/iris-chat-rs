//! A socket belongs to one public author, exact relay and unchanged filters.
//! It can only run discovery REQs; private publication never uses this type.
use super::*;
use tokio::sync::OwnedSemaphorePermit;

const MAX_RETAINED_CONNECTIONS: usize = 64;
const MAX_CONNECTION_AGE: Duration = Duration::from_secs(600);
const MAX_IDLE_CONTROL_FRAMES: usize = 64;
static RETAINED_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

struct Session {
    socket: ReadSocket,
    authentication: ReadAuthentication,
    closed_subscription: Option<SubscriptionId>,
    opened: tokio::time::Instant,
    permit: Option<OwnedSemaphorePermit>,
}

impl Session {
    fn accepts_idle_control(&self, text: &str) -> bool {
        use serde_json::Value;
        let Ok(Value::Array(parts)) = serde_json::from_str(text) else {
            return false;
        };
        let closed = |id: &str| {
            self.closed_subscription
                .as_ref()
                .is_some_and(|subscription| subscription.as_str() == id)
        };
        match parts.as_slice() {
            [Value::String(kind), Value::String(_)] if kind == "NOTICE" => true,
            [Value::String(kind), Value::String(id)] if kind == "EOSE" => closed(id),
            [Value::String(kind), Value::String(id), Value::String(_)] if kind == "CLOSED" => {
                closed(id)
            }
            [Value::String(kind), Value::String(id), Value::Bool(true), Value::String(_)]
                if kind == "OK" =>
            {
                self.authentication.accepted
                    && self.authentication.id.as_deref() == Some(id.as_str())
            }
            _ => false,
        }
    }
}

pub(in crate::core) struct ScopedReadConnection {
    relay: RelayUrl,
    filters: Vec<Filter>,
    session: Option<Session>,
    slots: Arc<Semaphore>,
}

impl ScopedReadConnection {
    pub(in crate::core) fn new(relay: &RelayUrl, filters: &[Filter]) -> Self {
        Self {
            relay: relay.clone(),
            filters: filters.to_vec(),
            session: None,
            slots: RETAINED_SLOTS
                .get_or_init(|| Arc::new(Semaphore::new(MAX_RETAINED_CONNECTIONS)))
                .clone(),
        }
    }

    fn matches(&self, relay: &RelayUrl, filters: &[Filter]) -> bool {
        &self.relay == relay
            && self.filters == filters
            && scoped_tls::sole_public_author(filters).is_some()
            && !filters.iter().any(requests_message_history)
    }

    async fn open(&self, connector: Option<Connector>) -> anyhow::Result<Session> {
        // Retention must never block a query or consume a cold-read permit.
        // When these slots are full, this read keeps its disposable behavior.
        let permit = self.slots.clone().try_acquire_owned().ok();
        let socket = open_read_socket(
            &self.relay,
            connector,
            relay_addresses::connect(&self.relay),
        )
        .await?;
        Ok(Session {
            socket,
            authentication: ReadAuthentication::default(),
            closed_subscription: None,
            opened: tokio::time::Instant::now(),
            permit,
        })
    }

    pub(super) async fn read(
        &mut self,
        relay: &RelayUrl,
        filters: &[Filter],
        collected: &mut Collection,
        connector: Option<Connector>,
        mut verification: Option<RelayVerification<'_>>,
    ) -> anyhow::Result<()> {
        if !self.matches(relay, filters) {
            self.session = None;
            anyhow::bail!("anonymous read connection scope changed");
        }
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.opened.elapsed() >= MAX_CONNECTION_AGE)
        {
            self.session = None;
        }
        let reused = self.session.is_some();
        // Take ownership during the transaction. A timeout or cancellation
        // drops a partially consumed connection instead of reusing it later.
        let mut session = match self.session.take() {
            Some(session) => session,
            None => self.open(connector.clone()).await?,
        };
        let mut result = read_query(
            relay,
            filters,
            collected,
            &mut session.socket,
            &mut session.authentication,
            verification.as_mut().map(RelayVerification::reborrow),
        )
        .await;
        if reused && collected.events.is_empty() && result.as_ref().is_err_and(transport_closed) {
            // A server may close between the idle read and a new REQ. Recover
            // once inside the original query deadline, with a fresh socket and
            // fresh anonymous AUTH. Semantic/auth/limit errors never retry here.
            drop(session);
            session = self.open(connector).await?;
            result = read_query(
                relay,
                filters,
                collected,
                &mut session.socket,
                &mut session.authentication,
                verification,
            )
            .await;
        }
        session.closed_subscription = result?;
        if session.closed_subscription.is_some()
            && session.permit.is_some()
            && session.opened.elapsed() < MAX_CONNECTION_AGE
        {
            self.session = Some(session);
        } else {
            let _ = session.socket.close(None).await;
        }
        Ok(())
    }

    async fn idle(&mut self) {
        if let Some(session) = self.session.as_mut() {
            let deadline = session.opened + MAX_CONNECTION_AGE;
            for _ in 0..MAX_IDLE_CONTROL_FRAMES {
                match tokio::time::timeout_at(deadline, session.socket.next()).await {
                    Ok(Some(Ok(Message::Ping(_)))) => {
                        // Tungstenite queues the matching Pong on receipt.
                        if !matches!(
                            tokio::time::timeout_at(deadline, session.socket.flush()).await,
                            Ok(Ok(()))
                        ) {
                            break;
                        }
                    }
                    Ok(Some(Ok(Message::Pong(_)))) => {}
                    Ok(Some(Ok(Message::Text(text)))) if session.accepts_idle_control(&text) => {
                        // AUTH may retry a public REQ that was already served,
                        // leaving a second EOSE after CLOSE. Accept only inert
                        // acknowledgements of this socket's completed state.
                        // They spend the same bounded control-frame budget and
                        // never open a subscription, change AUTH or wake a poll.
                    }
                    // No subscription is open while idle. Unexpected data,
                    // re-authentication, closure, flooding or expiry disposes
                    // the socket; none can wake a reconnect loop.
                    _ => break,
                }
            }
        }
        self.session = None;
        std::future::pending::<()>().await;
    }
}

fn transport_closed(error: &anyhow::Error) -> bool {
    error.is::<ReadClosed>()
        || matches!(
            error.downcast_ref::<tokio_tungstenite::tungstenite::Error>(),
            Some(
                tokio_tungstenite::tungstenite::Error::ConnectionClosed
                    | tokio_tungstenite::tungstenite::Error::AlreadyClosed
                    | tokio_tungstenite::tungstenite::Error::Io(_)
            )
        )
}

/// Keep control traffic flowing without extending the caller's poll cadence.
/// Cancellation leaves a healthy socket in its exact owning worker; dropping
/// that worker/configuration disposes every socket and releases every permit.
pub(in crate::core) async fn idle_connections(connections: &mut [ScopedReadConnection]) {
    futures_util::future::join_all(connections.iter_mut().map(ScopedReadConnection::idle)).await;
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests;
