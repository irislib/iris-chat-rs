use super::protocol::{PROTOCOL_RECONNECT_CHECK_SECS, PROTOCOL_SUBSCRIPTION_LIVENESS_CHECK_SECS};
use super::*;
use nostr_pubsub::{
    EventBus, EventSource, NostrEventSubscriber, NostrEventSubscription,
    SubscriptionDeliveryStatus, VerifiedEvent,
};
use nostr_pubsub_fips::FipsPubsubClient;
mod privacy;
pub(in crate::core) use privacy::PrivateMeshProviders;

pub(super) const MESH_REPLAY_EVENTS: usize = 64;
const OUTBOX_BATCH: usize = 16;

#[cfg(test)]
mod tests;

pub(super) struct MeshProtocolSubscriptions {
    filters: Vec<Filter>,
    subscriptions: Vec<Box<dyn NostrEventSubscription>>,
    publish_slots: Arc<tokio::sync::Semaphore>,
    outbox_cursor: Option<String>,
    retry_needed: bool,
    reconcile_pending: bool,
    replay_source: Option<Arc<storage::mesh_group_envelopes::GroupEnvelopeReplay>>,
    discovery: privacy::PrivateMeshDiscovery,
}

impl Default for MeshProtocolSubscriptions {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            subscriptions: Vec::new(),
            publish_slots: Arc::new(tokio::sync::Semaphore::new(64)),
            outbox_cursor: None,
            retry_needed: false,
            reconcile_pending: false,
            replay_source: None,
            discovery: privacy::PrivateMeshDiscovery::default(),
        }
    }
}

impl MeshProtocolSubscriptions {
    async fn update(
        &mut self,
        client: &FipsPubsubClient,
        filters: Vec<Filter>,
        sender: Sender<CoreMsg>,
    ) -> nostr_pubsub::Result<()> {
        let handler = Arc::new(move |delivery: nostr_pubsub::QueryEvent| {
            let _ = sender.send(CoreMsg::Internal(Box::new(InternalEvent::MeshEvent(
                delivery.event.into_event(),
            ))));
        });
        if self.filters == filters && !self.retry_needed {
            for (index, filters) in filters
                .chunks(client.options().max_filters_per_subscription)
                .enumerate()
            {
                if !matches!(
                    self.subscriptions
                        .get(index)
                        .and_then(|subscription| subscription.delivery_status()),
                    Some(SubscriptionDeliveryStatus::Closed | SubscriptionDeliveryStatus::Lagged)
                ) {
                    continue;
                }
                // Await the terminal forwarder's admitted bodies before
                // restoring live delivery. Missed history still needs catch-up.
                self.retry_needed = true;
                self.subscriptions.remove(index).close().await?;
                let subscription =
                    NostrEventSubscriber::subscribe(client, filters.to_vec(), handler.clone())
                        .await?;
                self.subscriptions.insert(index, subscription);
                self.retry_needed = false;
            }
            return Ok(());
        }
        let mut subscriptions = Vec::new();
        for filters in filters.chunks(client.options().max_filters_per_subscription) {
            subscriptions.push(
                NostrEventSubscriber::subscribe(client, filters.to_vec(), handler.clone()).await?,
            );
        }
        let mut close_error = None;
        for subscription in std::mem::replace(&mut self.subscriptions, subscriptions) {
            if matches!(
                subscription.delivery_status(),
                Some(SubscriptionDeliveryStatus::Closed | SubscriptionDeliveryStatus::Lagged)
            ) {
                if let Err(error) = subscription.close().await {
                    close_error.get_or_insert(error);
                }
            }
        }
        self.filters = filters;
        self.retry_needed = close_error.is_some();
        if let Some(error) = close_error {
            return Err(error);
        }
        Ok(())
    }
}

impl AppCore {
    pub(super) fn handle_fips_peer_discovery_needed(&self, event: InternalEvent) {
        let InternalEvent::FipsPeerDiscoveryNeeded {
            generation,
            author,
            requires_pending_work,
        } = event
        else {
            return;
        };
        if generation == self.fips_connection_generation
            && (!requires_pending_work || self.has_private_mesh_discovery_demand(author))
        {
            self.refresh_private_mesh_discovery_for_author(author);
        }
    }

    pub(super) fn has_private_mesh_discovery_demand(&self, author: PublicKey) -> bool {
        self.active_call_targets_device(author)
            || self.device_sync.as_ref().is_some_and(|mesh| {
                let Some(peer) = super::device_sync::fips_peer_from_hex(&author.to_hex()) else {
                    return false;
                };
                mesh.nearby_outbox
                    .read()
                    .is_ok_and(|outbox| outbox.has_pending_private_for_peer(&peer.npub()))
            })
    }

    pub(super) fn refresh_private_mesh_discovery_for_author(&self, author: PublicKey) {
        if let Some(mesh) = &self.device_sync {
            mesh.protocol_subscriptions.discovery.refresh_author(author);
        }
    }

    pub(super) fn refresh_private_mesh_discovery(&self) {
        if let Some(mesh) = &self.device_sync {
            mesh.protocol_subscriptions.discovery.refresh_all();
        }
    }

    fn mesh_group_replay_groups(&self) -> storage::mesh_group_envelopes::AllowedGroups {
        self.groups
            .values()
            .filter_map(|group| {
                let chat = group_chat_id(&group.group_id);
                if self.logged_in.is_none()
                    || !self.threads.contains_key(&chat)
                    || self.thread_is_message_request(&chat)
                    || self.is_removed_from_group(&chat)
                {
                    return None;
                }
                let members = group
                    .members
                    .iter()
                    .map(ToString::to_string)
                    .filter(|owner| !self.is_owner_blocked(owner))
                    .collect();
                Some((chat, members))
            })
            .collect()
    }

    #[cfg(test)]
    fn mesh_group_replay_source(
        &self,
    ) -> Option<Arc<storage::mesh_group_envelopes::GroupEnvelopeReplay>> {
        let owner = self.logged_in.as_ref()?.owner_pubkey.to_hex();
        Some(Arc::new(
            storage::mesh_group_envelopes::GroupEnvelopeReplay::new(
                self.app_store.shared(),
                owner,
                self.mesh_group_replay_groups(),
            ),
        ))
    }

    pub(super) fn retain_mesh_group_envelope(&mut self, event: &Event, chat: &str) {
        let Some(owner) = self
            .logged_in
            .as_ref()
            .map(|login| login.owner_pubkey.to_hex())
        else {
            return;
        };
        let groups = self.mesh_group_replay_groups();
        let Some(members) = groups.get(chat) else {
            return;
        };
        let event_id = event.id.to_hex();
        let Some(message) = self.threads.get(chat).and_then(|thread| {
            thread.messages.iter().find(|message| {
                message.source_event_id.as_deref() == Some(event_id.as_str())
                    && matches!(message.kind, ChatMessageKind::User)
                    && !message.deleted_for_everyone
                    && message.edit_history.is_empty()
                    && message
                        .author_owner_pubkey_hex
                        .as_ref()
                        .is_some_and(|sender| members.contains(sender))
                    && message
                        .author_owner_pubkey_hex
                        .as_ref()
                        .is_some_and(|sender| {
                            self.block_allows_history(chat, sender, message.created_at_secs)
                        })
                    && message
                        .expires_at_secs
                        .is_none_or(|until| until > unix_now().get())
            })
        }) else {
            return;
        };
        if let Err(error) = self
            .app_store
            .retain_group_envelope(&owner, chat, &message.id, event)
        {
            self.push_debug_log("mesh.group_replay.store", error.to_string());
        }
    }

    pub(super) fn reconcile_mesh_protocol_subscriptions(&mut self) {
        let Some(mesh) = self
            .device_sync
            .as_mut()
            .filter(|mesh| mesh.pubsub.is_some())
        else {
            return;
        };
        // Catch-up changes ratchet authors repeatedly. Subscribe to the final
        // state once, instead of cloning and reconciling every intermediate state.
        if self.batch_depth > 0 {
            mesh.protocol_subscriptions.reconcile_pending = true;
            return;
        }
        mesh.protocol_subscriptions.reconcile_pending = false;
        let filters = self
            .compute_protocol_subscription_plan()
            .as_ref()
            .map(super::protocol::build_protocol_subscription_filters)
            .unwrap_or_default();
        let (mut filters, mut discovery) = self.split_private_discovery_filters(filters);
        // Sibling keys can change without replacing the live endpoint. Keep
        // public author lookups anonymous unless explicitly local at startup.
        let own_authors = self.mesh_public_service_authors();
        filters.retain(|filter| {
            let sensitive = filter.kinds.as_ref().is_some_and(|kinds| {
                kinds
                    .iter()
                    .any(|kind| matches!(kind.as_u16(), 0 | 10002 | 10050 | 30078 | 37368 | 37195))
            });
            if sensitive
                && filter.authors.as_ref().is_some_and(|authors| {
                    authors.iter().any(|author| !own_authors.contains(author))
                })
            {
                if let Some(authors) = &filter.authors {
                    for author in authors {
                        let mut scoped = filter.clone();
                        scoped.authors = Some(std::collections::BTreeSet::from([*author]));
                        discovery.push(scoped);
                    }
                }
                false
            } else {
                true
            }
        });
        if let Some(mesh) = &self.device_sync {
            discovery.extend(mesh.endpoint_peer_advert_filters());
        }
        let Some(mesh) = self.device_sync.as_mut() else {
            return;
        };
        let Some(client) = &mesh.pubsub else {
            return;
        };
        // An identified client must never serve another recipient's durable
        // envelopes, even in response to an exact-ID query after a restart.
        let result = client.set_replay_source(None).and_then(|()| {
            mesh.protocol_subscriptions.replay_source = None;
            if let Some(providers) = &mesh.private_providers {
                mesh.protocol_subscriptions.discovery.update(
                    providers.clone(),
                    discovery,
                    mesh.endpoint.clone(),
                    self.core_sender.clone(),
                    &self.runtime,
                );
            }
            self.runtime.block_on(mesh.protocol_subscriptions.update(
                client,
                filters,
                self.core_sender.clone(),
            ))
        });
        mesh.protocol_subscriptions.retry_needed = result.is_err();
        if let Err(error) = result {
            self.push_debug_log("mesh.subscription.error", error.to_string());
            self.schedule_fast_protocol_retry_if_pending();
        } else if self.has_mesh_protocol_liveness_work() {
            self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                PROTOCOL_SUBSCRIPTION_LIVENESS_CHECK_SECS,
            ));
        }
    }

    pub(super) fn flush_mesh_protocol_subscriptions(&mut self) {
        if self
            .device_sync
            .as_ref()
            .is_some_and(|mesh| mesh.protocol_subscriptions.reconcile_pending)
        {
            self.reconcile_mesh_protocol_subscriptions();
        }
    }

    pub(super) fn publish_mesh_event(&self, event: &Event) {
        if !self.private_publication_allowed(event) {
            return;
        }
        let Some(mesh) = &self.device_sync else {
            return;
        };
        let Some(client) = mesh.pubsub.clone() else {
            return;
        };
        // The durable application outbox owns retries if the carrier is busy.
        let Ok(permit) = mesh
            .protocol_subscriptions
            .publish_slots
            .clone()
            .try_acquire_owned()
        else {
            return;
        };
        let Ok(event) = VerifiedEvent::try_from(event.clone()) else {
            return;
        };
        let cancelled = self.publication_task_token(&event.as_event().id.to_hex());
        let providers = mesh.private_providers.clone();
        self.runtime.spawn(async move {
            let _permit = permit;
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let private = super::fips_nearby::is_private_transport_event(event.as_event());
            let result = if private {
                match providers {
                    Some(providers) => providers.publish(event, cancelled).await,
                    None => Err("private providers unavailable".into()),
                }
            } else {
                tokio::time::timeout(
                    Duration::from_secs(2),
                    client.publish(event, EventSource::local_index("iris-chat")),
                )
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map(|_| ()).map_err(|error| error.to_string()))
            };
            if result.is_err() {
                crate::perflog!("mesh.publish deferred");
            }
        });
    }

    pub(super) fn has_mesh_protocol_retry_work(&self) -> bool {
        self.device_sync
            .as_ref()
            .is_some_and(|mesh| mesh.protocol_subscriptions.retry_needed)
    }

    pub(super) fn has_mesh_protocol_liveness_work(&self) -> bool {
        self.device_sync.as_ref().is_some_and(|mesh| {
            mesh.protocol_subscriptions.retry_needed
                || !mesh.protocol_subscriptions.subscriptions.is_empty()
        })
    }

    pub(super) fn has_mesh_outbox_work(&self) -> bool {
        self.pending_relay_publishes
            .values()
            .any(|pending| self.mesh_message_retry_needed(pending) == Some(true))
            && self
                .device_sync
                .as_ref()
                .is_some_and(|mesh| mesh.pubsub.is_some())
    }

    pub(super) fn schedule_mesh_outbox_retry(&mut self) {
        if !self.pending_relay_publishes.is_empty()
            && self
                .device_sync
                .as_ref()
                .is_some_and(|mesh| mesh.pubsub.is_some())
        {
            self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                if self.has_mesh_outbox_work() {
                    PROTOCOL_RECONNECT_CHECK_SECS
                } else {
                    PROTOCOL_SUBSCRIPTION_LIVENESS_CHECK_SECS
                },
            ));
        }
    }

    fn mesh_message_retry_needed(&self, pending: &PendingRelayPublish) -> Option<bool> {
        let message = self
            .threads
            .get(pending.chat_id.as_ref()?)?
            .messages
            .iter()
            .find(|message| Some(&message.id) == pending.inner_event_id.as_ref())?;
        Some(!matches!(
            message.delivery,
            DeliveryState::Received | DeliveryState::Seen
        ))
    }

    pub(super) fn replay_mesh_outbox(&mut self) {
        use std::ops::Bound::{Excluded, Unbounded};
        self.discard_removed_group_publications();
        let Some(mesh) = self.device_sync.as_mut() else {
            return;
        };
        let Some(client) = &mesh.pubsub else {
            return;
        };
        let limit = OUTBOX_BATCH
            .min(client.options().max_replay_events)
            .min(self.pending_relay_publishes.len());
        let start = mesh
            .protocol_subscriptions
            .outbox_cursor
            .as_ref()
            .map_or(Unbounded, Excluded);
        let batch = self
            .pending_relay_publishes
            .range::<String, _>((start, Unbounded))
            .chain(self.pending_relay_publishes.iter())
            .take(limit)
            .map(|(id, pending)| (id.clone(), pending.clone()))
            .collect::<Vec<_>>();
        mesh.protocol_subscriptions.outbox_cursor = batch.last().map(|(id, _)| id.clone());
        for (_, pending) in batch {
            if pending
                .chat_id
                .as_deref()
                .is_some_and(|chat| self.blocked_direct_publication(chat))
            {
                continue;
            }
            // Existing authenticated receipts stop mesh retries; relay persistence is separate.
            if self.mesh_message_retry_needed(&pending) != Some(false) {
                if let Ok(event) = serde_json::from_str::<Event>(&pending.event_json) {
                    self.publish_mesh_event(&event);
                }
            }
        }
        self.schedule_mesh_outbox_retry();
    }
}
