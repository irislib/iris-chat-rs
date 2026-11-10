use super::*;

// Discard outgoing work during decoding, before allocating copies of its bodies.
// Authentication evidence and incoming recovery queues retain the ordinary
// decoder and migration rules. This view must never replace the live checkpoint.
#[derive(Deserialize)]
pub(super) struct NotificationPreviewPersistedState {
    #[serde(default, rename = "pending_group_fanouts")]
    _fanouts: serde::de::IgnoredAny,
    #[serde(default, rename = "ready_group_publications")]
    _publications: serde::de::IgnoredAny,
    #[serde(default, rename = "pending_local_sibling_sends")]
    _sibling_sends: serde::de::IgnoredAny,
    #[serde(default, rename = "pending_remote_sends")]
    _remote_sends: serde::de::IgnoredAny,
    #[serde(flatten)]
    pub(super) state: ProtocolEnginePersistedState,
}

impl ProtocolEngine {
    /// Load an ephemeral receive preview without initializing an account,
    /// loading outgoing queues, or writing a startup checkpoint. Callers must
    /// provide an isolated storage overlay for any legacy migration writes and
    /// discard the engine after use; do not finish its batch or use it to send.
    pub fn load_for_notification_preview(
        storage: Arc<dyn StorageAdapter>,
        owner_pubkey: PublicKey,
        device_keys: &Keys,
    ) -> anyhow::Result<Option<Self>> {
        let Some(engine) = Self::load_persisted_state(
            storage,
            owner_pubkey,
            ndr_owner(owner_pubkey),
            ndr_device(device_keys.public_key()),
            device_keys.secret_key().to_secret_bytes(),
            true,
        )?
        else {
            return Ok(None);
        };
        // The ordinary constructor persists before returning. Starting the
        // batch here avoids materializing the entire backlog in a short-lived
        // iOS notification process, including on its first received event.
        engine.enter_batch();
        Ok(Some(engine))
    }
}
