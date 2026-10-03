/// Exact ciphertext acknowledgements survive app event-cache eviction and restart.
/// Timestamp-only acknowledgements cannot distinguish same-second messages.
#[derive(Clone, Debug, Default)]
struct ProtocolGroupReplayCache {
    order: std::collections::VecDeque<String>,
    members: HashSet<String>,
}

impl ProtocolGroupReplayCache {
    fn contains(&self, fingerprint: &str) -> bool {
        self.members.contains(fingerprint)
    }

    fn insert(&mut self, fingerprint: String) {
        if !self.members.insert(fingerprint.clone()) {
            return;
        }
        self.order.push_back(fingerprint);
        if self.order.len() > PROCESSED_GROUP_SENDER_KEY_LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.members.remove(&oldest);
            }
        }
    }
}

impl Serialize for ProtocolGroupReplayCache {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.order.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ProtocolGroupReplayCache {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries = Vec::<String>::deserialize(deserializer)?;
        let mut cache = Self::default();
        for entry in entries
            .into_iter()
            .rev()
            .take(PROCESSED_GROUP_SENDER_KEY_LIMIT)
            .rev()
        {
            cache.insert(entry);
        }
        Ok(cache)
    }
}

fn group_sender_key_fingerprint(message: &GroupSenderKeyMessage) -> String {
    use nostr::hashes::{sha256, Hash, HashEngine};
    let mut engine = sha256::Hash::engine();
    for bytes in [
        message.group_id.as_bytes(),
        &message.sender_event_pubkey.to_bytes(),
        &message.key_id.to_be_bytes(),
        &message.message_number.to_be_bytes(),
        &message.created_at.get().to_be_bytes(),
        &[u8::from(message.encrypted_header.is_some())],
        message
            .encrypted_header
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        &message.ciphertext,
    ] {
        engine.input(&(bytes.len() as u64).to_be_bytes());
        engine.input(bytes);
    }
    sha256::Hash::from_engine(engine).to_string()
}

#[cfg(test)]
mod group_replay_tests {
    use super::*;

    #[test]
    fn exact_group_replay_cache_is_bounded_and_restores_membership() {
        let mut cache = ProtocolGroupReplayCache::default();
        for index in 0..=PROCESSED_GROUP_SENDER_KEY_LIMIT {
            cache.insert(index.to_string());
        }
        cache.insert("1".into());
        assert_eq!(cache.order.len(), PROCESSED_GROUP_SENDER_KEY_LIMIT);
        assert!(!cache.contains("0"));
        assert!(cache.contains("1"));
        let restored: ProtocolGroupReplayCache =
            serde_json::from_str(&serde_json::to_string(&cache).unwrap()).unwrap();
        assert_eq!(restored.order, cache.order);
        assert_eq!(restored.members, cache.members);
        let oversized = (0..PROCESSED_GROUP_SENDER_KEY_LIMIT + 7)
            .map(|i| i.to_string())
            .collect::<Vec<_>>();
        let restored: ProtocolGroupReplayCache =
            serde_json::from_str(&serde_json::to_string(&oversized).unwrap()).unwrap();
        assert_eq!(restored.order.len(), PROCESSED_GROUP_SENDER_KEY_LIMIT);
        assert!(!restored.contains("6"));
        assert!(restored.contains("7"));
    }

    #[test]
    fn group_replay_fingerprint_distinguishes_same_second_ciphertexts_and_headers() {
        let mut message = GroupSenderKeyMessage {
            group_id: "test-group".into(),
            sender_event_pubkey: ndr_device(Keys::generate().public_key()),
            key_id: 0,
            message_number: 0,
            encrypted_header: Some("header".into()),
            created_at: NdrUnixSeconds(123),
            ciphertext: vec![1, 2, 3],
        };
        let first = group_sender_key_fingerprint(&message);
        message.ciphertext.push(4);
        assert_ne!(first, group_sender_key_fingerprint(&message));
        message.ciphertext.pop();
        message.encrypted_header = Some("other-header".into());
        assert_ne!(first, group_sender_key_fingerprint(&message));
        message.encrypted_header = Some("header".into());
        assert_eq!(first, group_sender_key_fingerprint(&message));
    }
}
