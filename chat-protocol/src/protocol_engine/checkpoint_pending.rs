#[cfg(test)]
thread_local! {
    static CHECKPOINT_PENDING_SERIALIZED_ITEMS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

type PendingGroupMessage = nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent;

struct SerializedPendingGroupMessages {
    json: Box<serde_json::value::RawValue>,
    records: Vec<CheckpointRecord>,
}

impl SerializedPendingGroupMessages {
    fn new(json: Box<serde_json::value::RawValue>) -> Self {
        let records = checkpoint_records_with_cached_pending(json.get(), 1, None);
        Self { json, records }
    }
}

// Immutable checkpoint clones share both ciphertexts and their JSON. Mutation
// detaches both generations, so rollback can restore its original cache.
#[derive(Clone, Default)]
struct PendingGroupMessages {
    values: Arc<Vec<PendingGroupMessage>>,
    serialized: Arc<std::sync::Mutex<Option<Arc<SerializedPendingGroupMessages>>>>,
}

impl std::fmt::Debug for PendingGroupMessages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingGroupMessages")
            .field("len", &self.values.len())
            .finish()
    }
}

impl PartialEq for PendingGroupMessages {
    fn eq(&self, other: &Self) -> bool {
        self.values == other.values
    }
}

impl Eq for PendingGroupMessages {}

impl std::ops::Deref for PendingGroupMessages {
    type Target = [PendingGroupMessage];
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl<'a> IntoIterator for &'a PendingGroupMessages {
    type Item = &'a PendingGroupMessage;
    type IntoIter = std::slice::Iter<'a, PendingGroupMessage>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl From<Vec<PendingGroupMessage>> for PendingGroupMessages {
    fn from(values: Vec<PendingGroupMessage>) -> Self {
        Self {
            values: Arc::new(values),
            serialized: Arc::default(),
        }
    }
}

impl PendingGroupMessages {
    fn changed(&mut self) -> &mut Vec<PendingGroupMessage> {
        self.serialized = Arc::default();
        Arc::make_mut(&mut self.values)
    }

    fn push(&mut self, value: PendingGroupMessage) {
        self.changed().push(value);
    }
    fn insert(&mut self, index: usize, value: PendingGroupMessage) {
        self.changed().insert(index, value);
    }
    fn remove(&mut self, index: usize) -> PendingGroupMessage {
        self.changed().remove(index)
    }

    #[cfg(test)]
    fn pop(&mut self) -> Option<PendingGroupMessage> {
        self.changed().pop()
    }

    fn retain(&mut self, mut keep: impl FnMut(&PendingGroupMessage) -> bool) {
        // Do not detach/serialize the queue for the common no-op ACK/prune.
        let Some(first_removed) = self.values.iter().position(|value| !keep(value)) else {
            return;
        };
        let mut index = 0;
        self.changed().retain(|value| {
            let current = index;
            index += 1;
            current < first_removed || (current > first_removed && keep(value))
        });
    }

    fn serialized(&self) -> Result<Arc<SerializedPendingGroupMessages>, serde_json::Error> {
        let mut cached = self
            .serialized
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(value) = cached.as_ref() {
            return Ok(value.clone());
        }
        let json = serde_json::value::to_raw_value(self.values.as_ref())?;
        #[cfg(test)]
        CHECKPOINT_PENDING_SERIALIZED_ITEMS
            .with(|count| count.set(count.get() + self.values.len()));
        let value = Arc::new(SerializedPendingGroupMessages::new(json));
        *cached = Some(value.clone());
        Ok(value)
    }
}

#[cfg(test)]
impl std::ops::IndexMut<usize> for PendingGroupMessages {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.changed()[index]
    }
}

#[cfg(test)]
impl std::ops::Index<usize> for PendingGroupMessages {
    type Output = PendingGroupMessage;
    fn index(&self, index: usize) -> &Self::Output {
        &self.values[index]
    }
}

impl Serialize for PendingGroupMessages {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let cached = self.serialized().map_err(serde::ser::Error::custom)?;
        cached.json.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PendingGroupMessages {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values = Vec::deserialize(deserializer)?;
        let result = Self::from(values);
        // Compact once during restore: retaining old padding here would move
        // its absolute checkpoint boundaries when an earlier field grows.
        result.serialized().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}
