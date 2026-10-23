// Retry clocks are mutable; the large event/envelope body is shared across
// checkpoint and rollback clones. Any body mutation starts a fresh cache
// generation, including owner resolution and imported-envelope changes.
#[derive(Clone)]
struct CheckpointInbound {
    value: Arc<ProtocolPendingInboundBody>,
    cached: Arc<std::sync::OnceLock<CheckpointInboundBodies>>,
}

#[derive(Clone)]
struct CheckpointInboundBodies {
    event: Arc<crate::StorageCheckpointBody>,
    envelope: Arc<crate::StorageCheckpointBody>,
}

impl From<ProtocolPendingInboundBody> for CheckpointInbound {
    fn from(value: ProtocolPendingInboundBody) -> Self {
        Self {
            value: Arc::new(value),
            cached: Arc::default(),
        }
    }
}

impl std::fmt::Debug for CheckpointInbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl PartialEq for CheckpointInbound {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}
impl Eq for CheckpointInbound {}

impl std::ops::Deref for ProtocolPendingInbound {
    type Target = ProtocolPendingInboundBody;
    fn deref(&self) -> &Self::Target {
        &self.body.value
    }
}

impl std::ops::DerefMut for ProtocolPendingInbound {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.body.cached = Arc::default();
        Arc::make_mut(&mut self.body.value)
    }
}

impl Serialize for CheckpointInbound {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CheckpointInbound {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ProtocolPendingInboundBody::deserialize(deserializer).map(Self::from)
    }
}

impl CheckpointInbound {
    fn checkpoint_bodies(&self) -> Result<CheckpointInboundBodies, serde_json::Error> {
        if let Some(cached) = self.cached.get() {
            return Ok(cached.clone());
        }
        let bodies = CheckpointInboundBodies {
            event: Arc::new(crate::StorageCheckpointBody::new(
                serde_json::value::to_raw_value(&self.value.event)?.into(),
            )),
            envelope: Arc::new(crate::StorageCheckpointBody::new(
                serde_json::value::to_raw_value(&self.value.envelope)?.into(),
            )),
        };
        let _ = self.cached.set(bodies);
        Ok(self
            .cached
            .get()
            .expect("initialized inbound checkpoint bodies")
            .clone())
    }
}

fn append_checkpoint_inbound(
    checkpoint: &mut crate::StorageCheckpoint,
    inbound: &[ProtocolPendingInbound],
) -> Result<(), serde_json::Error> {
    #[derive(Serialize)]
    struct Clocks<'a> {
        created_at_secs: u64,
        next_retry_at_secs: u64,
        event_id: &'a str,
    }
    #[derive(Serialize)]
    struct Owners<'a> {
        sender_message_pubkey_hex: &'a Option<String>,
        resolved_owner_pubkey_hex: &'a Option<String>,
        claimed_owner_pubkey_hex: &'a Option<String>,
        metadata_verified: bool,
    }
    for (index, pending) in inbound.iter().enumerate() {
        let bodies = pending.body.checkpoint_bodies()?;
        let clocks = serde_json::to_string(&Clocks {
            created_at_secs: pending.created_at_secs,
            next_retry_at_secs: pending.next_retry_at_secs,
            event_id: &pending.event_id,
        })?;
        let owners = serde_json::to_string(&Owners {
            sender_message_pubkey_hex: &pending.sender_message_pubkey_hex,
            resolved_owner_pubkey_hex: &pending.resolved_owner_pubkey_hex,
            claimed_owner_pubkey_hex: &pending.claimed_owner_pubkey_hex,
            metadata_verified: pending.metadata_verified,
        })?;
        checkpoint.push_record(
            2,
            bodies.event,
            format!("{}{{\"event\":", if index == 0 { "" } else { "," }),
            format!(",{},", &clocks[1..clocks.len() - 1]),
            0,
        );
        checkpoint.push_record(
            3,
            bodies.envelope,
            "\"envelope\":".into(),
            format!(",{}", &owners[1..]),
            0,
        );
    }
    Ok(())
}
