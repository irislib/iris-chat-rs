// Keep real durable work in the measured windows even when missing-roster
// retries only update volatile backoff. The original 44 intents stay untouched.
struct BatchedExpiringControls {
    controls: Vec<(String, u64, bool)>,
    retired: usize,
}

impl BatchedExpiringControls {
    fn seed(engine: &mut ProtocolEngine, recovery_epoch: u64, now: u64) -> Self {
        assert_eq!(engine.pending_remote_sends.len(), 44);
        let prototype = engine.pending_remote_sends[0].clone();
        assert!(engine
            .session_manager
            .roster(prototype.recipient_owner)
            .is_none());
        let mut controls = Vec::new();
        for (index, offset) in [210, 270].into_iter().enumerate() {
            let expires = recovery_epoch + offset;
            assert!(
                expires > now,
                "restore exceeded the control scheduling margin"
            );
            let id = format!("cpu-expiring-control-{index}");
            assert!(!engine
                .pending_remote_sends
                .iter()
                .any(|pending| pending.message_id == id));
            let mut rumor = pairwise_codec::typing_event(
                engine.owner_pubkey,
                pairwise_codec::EncodeOptions::new(now, now * 1_000).with_expiration(expires),
            )
            .unwrap();
            rumor.ensure_id();
            let mut pending = prototype.clone();
            pending.message_id = id.clone();
            pending.inner_event_id = rumor.id.map(|id| id.to_hex());
            pending.payload = serde_json::to_vec(&rumor).unwrap();
            pending.created_at_secs = now;
            pending.next_retry_at_secs = expires;
            pending.expires_at_secs = Some(expires);
            engine.pending_remote_sends.push(pending);
            controls.push((id, expires, false));
        }
        engine.persist().unwrap();
        Self {
            controls,
            retired: 0,
        }
    }

    fn observe_turn(
        &mut self,
        engine: &ProtocolEngine,
        now: u64,
        puts_before: usize,
        puts_after: usize,
    ) {
        for (id, expires, retired) in &mut self.controls {
            let pending = engine
                .pending_remote_sends
                .iter()
                .find(|pending| pending.message_id == *id);
            if *retired {
                assert!(pending.is_none(), "retired control returned");
            } else if let Some(pending) = pending {
                assert_eq!(pending.expires_at_secs, Some(*expires));
                assert_eq!(pending.next_retry_at_secs, *expires);
            } else {
                assert!(now >= *expires, "control retired before expiry");
                assert!(
                    puts_after > puts_before,
                    "control retirement must flush in its own retry turn"
                );
                *retired = true;
                self.retired += 1;
            }
        }
    }
}
