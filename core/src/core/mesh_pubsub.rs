use super::protocol::{PROTOCOL_RECONNECT_CHECK_SECS, PROTOCOL_SUBSCRIPTION_LIVENESS_CHECK_SECS};
use super::*;
use nostr_pubsub::{
    EventBus, EventSource, NostrEventSubscriber, NostrEventSubscription, VerifiedEvent,
};
use nostr_pubsub_fips::FipsPubsubClient;

pub(super) const MESH_REPLAY_EVENTS: usize = 64;
const OUTBOX_BATCH: usize = 16;

pub(super) struct MeshProtocolSubscriptions {
    filters: Vec<Filter>,
    subscriptions: Vec<Box<dyn NostrEventSubscription>>,
    publish_slots: Arc<tokio::sync::Semaphore>,
    outbox_cursor: Option<String>,
    retry_needed: bool,
}

impl Default for MeshProtocolSubscriptions {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            subscriptions: Vec::new(),
            publish_slots: Arc::new(tokio::sync::Semaphore::new(64)),
            outbox_cursor: None,
            retry_needed: false,
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
        if self.filters == filters {
            return Ok(());
        }
        let handler = Arc::new(move |delivery: nostr_pubsub::QueryEvent| {
            let _ = sender.send(CoreMsg::Internal(Box::new(InternalEvent::MeshEvent(
                delivery.event.into_event(),
            ))));
        });
        let mut subscriptions = Vec::new();
        for filters in filters.chunks(client.options().max_filters_per_subscription) {
            subscriptions.push(
                NostrEventSubscriber::subscribe(client, filters.to_vec(), handler.clone()).await?,
            );
        }
        self.subscriptions = subscriptions;
        self.filters = filters;
        Ok(())
    }
}

impl AppCore {
    pub(super) fn reconcile_mesh_protocol_subscriptions(&mut self) {
        let filters = self
            .compute_protocol_subscription_plan()
            .as_ref()
            .map(super::protocol::build_protocol_subscription_filters)
            .unwrap_or_default();
        let Some(mesh) = self.device_sync.as_mut() else {
            return;
        };
        let Some(client) = &mesh.pubsub else {
            return;
        };
        let result = self.runtime.block_on(mesh.protocol_subscriptions.update(
            client,
            filters,
            self.core_sender.clone(),
        ));
        mesh.protocol_subscriptions.retry_needed = result.is_err();
        if let Err(error) = result {
            self.push_debug_log("mesh.subscription.error", error.to_string());
            self.schedule_fast_protocol_retry_if_pending();
        }
    }

    pub(super) fn publish_mesh_event(&self, event: &Event) {
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
        self.runtime.spawn(async move {
            let _permit = permit;
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                client.publish(event, EventSource::local_index("iris-chat")),
            )
            .await;
            if !matches!(result, Ok(Ok(_))) {
                crate::perflog!("mesh.publish deferred");
            }
        });
    }

    pub(super) fn has_mesh_protocol_retry_work(&self) -> bool {
        self.device_sync
            .as_ref()
            .is_some_and(|mesh| mesh.protocol_subscriptions.retry_needed)
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
