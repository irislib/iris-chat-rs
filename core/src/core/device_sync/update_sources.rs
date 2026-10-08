use super::*;
use crate::update_announcements::{register_update_providers, shared_update_providers};

type Provider = Arc<dyn nostr_pubsub::NostrEventSubscriber>;

/// Message-server ownership follows the account, independently of its mesh.
#[derive(Default)]
pub(in crate::core) struct UpdateSources {
    key: Option<(String, String, Vec<String>)>,
    relay_urls: Vec<String>,
    relay: Option<Provider>,
    providers: Vec<Provider>,
}

impl UpdateSources {
    pub(in crate::core) fn new(relay_urls: Vec<String>) -> Self {
        register_update_providers(&[], relay_urls.clone());
        Self {
            relay_urls,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests;

impl AppCore {
    pub(in crate::core) fn reconcile_update_sources(&mut self) {
        let Some(logged_in) = &self.logged_in else {
            self.update_sources.relay_urls = self.preferences.nostr_relay_urls.clone();
            self.clear_update_sources();
            return;
        };
        let relays = logged_in
            .relay_urls
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let key = (
            logged_in.owner_pubkey.to_hex(),
            logged_in.device_keys.public_key().to_hex(),
            relays.clone(),
        );
        if self.update_sources.key.as_ref() != Some(&key) {
            let (mut providers, error) = self.runtime.block_on(shared_update_providers(
                None,
                Some(logged_in.client.clone()),
                relays.clone(),
            ));
            self.update_sources.key = Some(key);
            self.update_sources.relay_urls = relays;
            self.update_sources.relay = providers.pop();
            if let Some(error) = error {
                self.push_debug_log("update.message_servers.start.error", error);
            }
        }
        let peer = self
            .device_sync
            .as_ref()
            .and_then(|runtime| runtime.pubsub.as_ref())
            .cloned();
        self.register_update_sources(peer.as_ref());
    }

    pub(super) fn register_update_sources(&mut self, peer: Option<&Arc<FipsPubsubClient>>) {
        self.update_sources.providers = peer
            .map(|client| Arc::new(client.fresh_subscriber()) as Provider)
            .into_iter()
            .chain(self.update_sources.relay.clone())
            .collect();
        register_update_providers(
            &self.update_sources.providers,
            self.update_sources.relay_urls.clone(),
        );
    }

    pub(in crate::core) fn clear_update_sources(&mut self) {
        // Retain the last explicit server policy after providers expire, but
        // never retain an old account's client across logout or replacement.
        self.update_sources.key = None;
        self.update_sources.relay = None;
        self.update_sources.providers.clear();
        register_update_providers(&[], self.update_sources.relay_urls.clone());
    }
}
