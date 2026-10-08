//! Independent live observations with one authenticated rollback watermark.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::future::join_all;
use hashtree_core::Cid;
use hashtree_resolver::{nostr::NostrRootResolver, Event, ResolverError, RootResolver};
use hashtree_updater::{
    NostrEventSubscriber, PubsubRootResolver, UpdateError, UpdateEventCache, UpdateRef,
};
use nostr::ToBech32;
use nostr_pubsub::{Filter, NostrEventHandler, NostrEventSubscription};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct AvailableUpdateResolver {
    providers: Vec<Arc<dyn NostrEventSubscriber>>,
    window: Duration,
    roots: Arc<Mutex<HashMap<String, Arc<Mutex<UpdateEventCache>>>>>,
}

impl AvailableUpdateResolver {
    pub fn new(
        providers: Vec<Arc<dyn NostrEventSubscriber>>,
        window: Duration,
    ) -> Result<Self, UpdateError> {
        if providers.is_empty() {
            return Err(UpdateError::Announcement(
                "No update sources are available".into(),
            ));
        }
        Ok(Self {
            providers,
            window,
            roots: Arc::default(),
        })
    }

    pub async fn ingest_event(&self, event: Event) -> Result<bool, ResolverError> {
        let tree = event
            .tags
            .identifier()
            .ok_or_else(|| ResolverError::Other("root event has no tree identifier".into()))?;
        let key = format!("{}/{tree}", event.pubkey.to_bech32().map_err(network)?);
        self.cache(&key)?
            .lock()
            .map_err(network)?
            .ingest_event(event)
            .map_err(network)
    }

    pub async fn latest_event(&self, key: &str) -> Result<Option<Event>, ResolverError> {
        Ok(self
            .cache(key)?
            .lock()
            .map_err(network)?
            .latest()
            .map(|event| event.as_event().clone()))
    }

    fn cache(&self, key: &str) -> Result<Arc<Mutex<UpdateEventCache>>, ResolverError> {
        let mut roots = self.roots.lock().map_err(network)?;
        if let Some(cache) = roots.get(key) {
            return Ok(cache.clone());
        }
        let (npub, tree_name) = key
            .split_once('/')
            .ok_or_else(|| ResolverError::Other("invalid update resolver key".into()))?;
        let cache = Arc::new(Mutex::new(
            UpdateEventCache::new(&UpdateRef {
                npub: npub.to_owned(),
                tree_name: tree_name.to_owned(),
                path: None,
            })
            .map_err(network)?,
        ));
        roots.insert(key.to_owned(), cache.clone());
        Ok(cache)
    }
}

#[async_trait]
impl RootResolver for AvailableUpdateResolver {
    async fn resolve(&self, key: &str) -> Result<Option<Cid>, ResolverError> {
        let watermark = self.cache(key)?;
        let results = join_all(self.providers.iter().map(|provider| {
            // Freshness and delivery health belong to this source and check.
            // Sharing the upstream event-ID cache would let an interrupted
            // source veto a healthy announcement of identical content.
            let resolver = PubsubRootResolver::new(
                Arc::new(RecordingProvider {
                    provider: provider.clone(),
                    watermark: watermark.clone(),
                }),
                self.window,
            );
            async move { resolver.resolve(key).await }
        }))
        .await;

        // Every source independently checks freshness and uninterrupted delivery.
        // Even a failed source may advance the shared signed-root watermark, so
        // compare only after all observation windows have completed.
        if let Some(event) = self.latest_event(key).await? {
            if let Some(current) = NostrRootResolver::root_from_event(key, &event)? {
                if results
                    .iter()
                    .any(|result| matches!(result, Ok(Some(root)) if root == &current))
                {
                    // An independently confirmed identical root is sufficient
                    // even if another announcement refreshed its timestamp. Keep
                    // the newest signed event for subsequent rollback checks.
                    return Ok(Some(current));
                }
            }
        }
        Err(ResolverError::Network(format!(
            "Update check inconclusive: no available source confirmed the current signed root for {key}"
        )))
    }

    async fn subscribe(&self, _key: &str) -> Result<mpsc::Receiver<Option<Cid>>, ResolverError> {
        Err(ResolverError::Other(
            "Use the shared pubsub provider for continuous subscriptions".into(),
        ))
    }
}

struct RecordingProvider {
    provider: Arc<dyn NostrEventSubscriber>,
    watermark: Arc<Mutex<UpdateEventCache>>,
}

#[async_trait]
impl NostrEventSubscriber for RecordingProvider {
    async fn subscribe(
        &self,
        filters: Vec<Filter>,
        handler: NostrEventHandler,
    ) -> nostr_pubsub::Result<Box<dyn NostrEventSubscription>> {
        let watermark = self.watermark.clone();
        self.provider
            .subscribe(
                filters,
                Arc::new(move |incoming| {
                    // Retain authenticated observations immediately, even if this
                    // source fails or the caller cancels before its window completes.
                    // The upstream cache enforces publisher, tree and event ordering.
                    if let Ok(mut cache) = watermark.lock() {
                        cache.ingest(incoming.event.clone());
                    }
                    handler(incoming);
                }),
            )
            .await
    }
}

fn network(error: impl std::fmt::Display) -> ResolverError {
    ResolverError::Network(error.to_string())
}

#[cfg(test)]
mod tests;
