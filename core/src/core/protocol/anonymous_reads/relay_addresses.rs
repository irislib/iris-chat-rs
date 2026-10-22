//! Only public message-server DNS answers are shared. No author, filter, URL
//! path, socket, authentication key or TLS state belongs in this cache.
use nostr::RelayUrl;
use std::{
    collections::BTreeMap,
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};
use tokio::{net::TcpStream, sync::Mutex as AsyncMutex};

// getaddrinfo does not expose DNS TTLs. This short application limit coalesces
// a discovery wave without retaining an answer through a 120-second quiet cycle.
const TTL: Duration = Duration::from_secs(30);
const MAX_SERVERS: usize = 64;
const MAX_CACHED_ADDRESSES: usize = 16;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Server {
    host: String,
    port: u16,
}

impl Server {
    fn from_relay(relay: &RelayUrl) -> io::Result<Self> {
        let url = url::Url::parse(relay.as_str()).map_err(io::Error::other)?;
        let host = url
            .host_str()
            .ok_or_else(|| io::Error::other("message server has no hostname"))?;
        Ok(Self {
            host: host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned(),
            port: url
                .port_or_known_default()
                .ok_or_else(|| io::Error::other("message server has no port"))?,
        })
    }
}

#[derive(Clone)]
struct Answer {
    addresses: Arc<[SocketAddr]>,
    resolved_at: Instant,
}

type Slot = Arc<AsyncMutex<Option<Answer>>>;

#[derive(Default)]
struct State {
    generation: u64,
    entries: BTreeMap<Server, Slot>,
}

#[derive(Default)]
struct AddressCache {
    state: Mutex<State>,
}

struct Lookup {
    answer: Answer,
    generation: u64,
    slot: Option<Slot>,
}

impl AddressCache {
    fn state(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                // DNS answers are disposable. Discard every entry and advance
                // the generation before recovering from an interrupted edit.
                let mut state = poisoned.into_inner();
                state.generation = state.generation.wrapping_add(1);
                state.entries.clear();
                self.state.clear_poison();
                state
            }
        }
    }

    fn clear(&self) {
        let mut state = self.state();
        state.generation = state.generation.wrapping_add(1);
        state.entries.clear();
    }

    fn slot(&self, server: &Server) -> (u64, Option<Slot>) {
        let mut state = self.state();
        if let Some(slot) = state.entries.get(server) {
            return (state.generation, Some(slot.clone()));
        }
        if state.entries.len() == MAX_SERVERS {
            // Never evict a running lookup or its waiters: that would defeat
            // coalescing. Excess distinct servers can still resolve uncached.
            let idle = state
                .entries
                .iter()
                .find(|(_, slot)| Arc::strong_count(slot) == 1)
                .map(|(key, _)| key.clone());
            if let Some(idle) = idle {
                state.entries.remove(&idle);
            } else {
                return (state.generation, None);
            }
        }
        let slot = Arc::new(AsyncMutex::new(None));
        state.entries.insert(server.clone(), slot.clone());
        (state.generation, Some(slot))
    }

    async fn resolve<F, Fut>(&self, server: &Server, resolver: &F) -> io::Result<Lookup>
    where
        F: Fn(Server) -> Fut,
        Fut: Future<Output = io::Result<Vec<SocketAddr>>>,
    {
        loop {
            let (generation, slot) = self.slot(server);
            // This per-server lock coalesces lookups, never connections. A
            // cancelled caller drops the guard; we spawn no resolver task.
            // Tokio's existing blocking system lookup itself is not cancellable.
            let mut cached = match &slot {
                Some(slot) => Some(slot.lock().await),
                None => None,
            };
            let answer = cached
                .as_deref()
                .and_then(Option::as_ref)
                .filter(|answer| answer.resolved_at.elapsed() < TTL)
                .cloned();
            let result = match answer {
                Some(answer) => Ok(answer),
                None => resolver(server.clone()).await.and_then(|addresses| {
                    if addresses.is_empty() {
                        return Err(io::Error::other("message server has no addresses"));
                    }
                    Ok(Answer {
                        addresses: addresses.into(),
                        resolved_at: Instant::now(),
                    })
                }),
            };
            // A network reset also invalidates an answer which was still being
            // resolved. It must neither populate the new cache nor escape it.
            let state = self.state();
            if state.generation != generation {
                continue;
            }
            let answer = result?;
            if let Some(cached) = cached.as_deref_mut() {
                // Do not truncate resolver ordering or IPv4/IPv6 fallback.
                // Large answers remain usable, but are not retained.
                *cached = (answer.addresses.len() <= MAX_CACHED_ADDRESSES).then(|| answer.clone());
            }
            drop(state);
            drop(cached);
            return Ok(Lookup {
                answer,
                generation,
                slot,
            });
        }
    }

    async fn invalidate(&self, server: &Server, lookup: &Lookup) {
        let Some(slot) = lookup.slot.as_ref() else {
            return;
        };
        // A cache-hit waiter may briefly own this lock without replacing the
        // failed answer. Wait for it so a retry cannot reuse that same answer.
        // Never hold the global state lock across this await.
        let mut cached = slot.lock().await;
        let state = self.state();
        if state.generation != lookup.generation
            || !state
                .entries
                .get(server)
                .is_some_and(|current| Arc::ptr_eq(current, slot))
        {
            return;
        }
        // Failure from an older connection must not invalidate a newer answer.
        if cached
            .as_ref()
            .is_some_and(|answer| Arc::ptr_eq(&answer.addresses, &lookup.answer.addresses))
        {
            *cached = None;
        }
    }

    async fn connect_with<R, RF, C, CF, T>(
        &self,
        server: &Server,
        resolver: R,
        connect: C,
    ) -> io::Result<T>
    where
        R: Fn(Server) -> RF,
        RF: Future<Output = io::Result<Vec<SocketAddr>>>,
        C: Fn(Arc<[SocketAddr]>) -> CF,
        CF: Future<Output = io::Result<T>>,
    {
        let mut retry = true;
        loop {
            let lookup = self.resolve(server, &resolver).await?;
            match connect(lookup.answer.addresses.clone()).await {
                Ok(socket) => return Ok(socket),
                Err(error) => {
                    self.invalidate(server, &lookup).await;
                    if !retry {
                        return Err(error);
                    }
                    retry = false;
                }
            }
        }
    }
}

fn shared() -> &'static AddressCache {
    static CACHE: OnceLock<AddressCache> = OnceLock::new();
    CACHE.get_or_init(AddressCache::default)
}

pub(super) fn clear() {
    shared().clear();
}

pub(super) async fn connect(relay: &RelayUrl) -> io::Result<TcpStream> {
    let server = Server::from_relay(relay)?;
    if let Ok(ip) = server.host.parse::<IpAddr>() {
        return TcpStream::connect(SocketAddr::new(ip, server.port)).await;
    }
    shared()
        .connect_with(
            &server,
            |server| async move {
                tokio::net::lookup_host((server.host.as_str(), server.port))
                    .await
                    .map(|addresses| addresses.collect())
            },
            |addresses| async move { TcpStream::connect(addresses.as_ref()).await },
        )
        .await
}

#[cfg(test)]
mod tests;
