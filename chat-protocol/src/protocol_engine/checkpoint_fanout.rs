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
}

impl From<GroupPendingFanout> for CheckpointGroupFanout {
    fn from(value: GroupPendingFanout) -> Self {
        Self {
            value: Arc::new(value),
            serialized: Arc::default(),
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
