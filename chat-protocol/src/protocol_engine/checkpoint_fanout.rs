#[cfg(test)]
thread_local! {
    static CHECKPOINT_FANOUT_SERIALIZED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Queued payloads are immutable. Retry timing and remaining recipients live in
// ProtocolPendingGroupFanout, outside this shared generation. Checkpoint and
// rollback clones therefore need neither copy nor re-encode every payload.
// The cache retires with the payload, including on successful delivery.
#[derive(Clone)]
struct CheckpointGroupFanout {
    value: Arc<GroupPendingFanout>,
    serialized: Arc<Mutex<Option<Arc<serde_json::value::RawValue>>>>,
    repair_identity: Arc<std::sync::OnceLock<Option<serde_json::Value>>>,
    checkpoint_body: Arc<std::sync::OnceLock<Arc<crate::StorageCheckpointBody>>>,
}

impl From<GroupPendingFanout> for CheckpointGroupFanout {
    fn from(value: GroupPendingFanout) -> Self {
        Self {
            value: Arc::new(value),
            serialized: Arc::default(),
            repair_identity: Arc::default(),
            checkpoint_body: Arc::default(),
        }
    }
}

impl AsRef<GroupPendingFanout> for CheckpointGroupFanout {
    fn as_ref(&self) -> &GroupPendingFanout {
        &self.value
    }
}

impl PartialEq for CheckpointGroupFanout {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}
impl Eq for CheckpointGroupFanout {}

impl std::fmt::Debug for CheckpointGroupFanout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl CheckpointGroupFanout {
    fn checkpoint_body(&self) -> Result<Arc<crate::StorageCheckpointBody>, serde_json::Error> {
        if let Some(body) = self.checkpoint_body.get() {
            return Ok(body.clone());
        }
        let body = Arc::new(crate::StorageCheckpointBody::new(self.serialized()?));
        // Sharing is immutable; a racing equivalent initialization is harmless.
        let _ = self.checkpoint_body.set(body);
        Ok(self
            .checkpoint_body
            .get()
            .expect("initialized checkpoint body")
            .clone())
    }
    fn repair_identity(&self) -> Option<&serde_json::Value> {
        self.repair_identity
            .get_or_init(|| {
                let GroupPendingFanout::Remote { payload, .. } = self.as_ref() else {
                    return None;
                };
                if !matches!(
                    JsonGroupPayloadCodecV1.decode_pairwise_command(payload),
                    Ok(Some(GroupPairwiseCommand::SenderKeyRepairRequest { .. }))
                ) {
                    return None;
                }
                let mut identity: serde_json::Value = serde_json::from_slice(payload).ok()?;
                let fields = identity.as_object_mut()?;
                if fields.get("kind")?.as_u64() != Some(10_447) {
                    return None;
                }
                // These are refresh clocks, not recovery selectors. The receiver
                // has no request expiry or request-ID correlation. Preserve every
                // other field, including extensions and the original device key.
                fields.remove("id");
                fields.remove("created_at");
                let mut content: serde_json::Value =
                    serde_json::from_str(fields.get("content")?.as_str()?).ok()?;
                content.as_object_mut()?.remove("createdAt");
                fields.insert("content".into(), content);
                fields.get_mut("tags")?.as_array_mut()?.retain(|tag| {
                    tag.as_array()
                        .and_then(|parts| parts.first())
                        .and_then(serde_json::Value::as_str)
                        != Some("ms")
                });
                Some(identity)
            })
            .as_ref()
    }

    fn serialized(&self) -> Result<Arc<serde_json::value::RawValue>, serde_json::Error> {
        let mut cached = self
            .serialized
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(value) = cached.as_ref() {
            return Ok(value.clone());
        }
        let json: Arc<serde_json::value::RawValue> =
            serde_json::value::to_raw_value(self.value.as_ref())?.into();
        #[cfg(test)]
        CHECKPOINT_FANOUT_SERIALIZED_BYTES.with(|count| count.set(count.get() + json.get().len()));
        *cached = Some(json.clone());
        Ok(json)
    }
}

impl ProtocolPendingGroupFanout {
    fn same_pending_repair(&self, other: &Self) -> bool {
        if self.inner_event_id.is_some()
            || other.inner_event_id.is_some()
            || self.group_id != other.group_id
            || self.remaining_devices != other.remaining_devices
        {
            return false;
        }
        let (
            GroupPendingFanout::Remote {
                recipient_owner: left,
                ..
            },
            GroupPendingFanout::Remote {
                recipient_owner: right,
                ..
            },
        ) = (self.fanout.as_ref(), other.fanout.as_ref())
        else {
            return false;
        };
        left == right
            && matches!((self.fanout.repair_identity(), other.fanout.repair_identity()),
            (Some(left), Some(right)) if left == right)
    }
}

impl Serialize for CheckpointGroupFanout {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.serialized()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CheckpointGroupFanout {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        GroupPendingFanout::deserialize(deserializer).map(Self::from)
    }
}

fn append_checkpoint_padding(output: &mut String, target: usize) {
    // Copy fixed ASCII blocks instead of extending one char at a time. Keep
    // scratch space bounded even when an existing checkpoint has large slack.
    const SPACES: &str = concat!(
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
        "                                                                                                                                                                                                                                                                ",
    );
    let mut remaining = target.saturating_sub(output.len());
    while remaining >= SPACES.len() {
        output.push_str(SPACES);
        remaining -= SPACES.len();
    }
    if let Some(tail) = SPACES.get(..remaining) {
        output.push_str(tail);
    }
}
