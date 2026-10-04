// Direct participants can deliver mutations out of order. Own-device traffic
// must instead use typed reconciliation, which checks the original's history
// entitlement before transferring replacement text.
enum DirectReceive {
    Pending,
    Discarded,
    Message(ProtocolDecryptedMessage),
}

fn is_message_mutation_payload(payload: &[u8]) -> bool {
    #[derive(Deserialize)]
    struct EventKind {
        kind: u32,
    }
    fn mutation_kind(payload: &[u8]) -> bool {
        serde_json::from_slice::<EventKind>(payload)
            .is_ok_and(|event| matches!(event.kind, 1009 | 5))
    }
    mutation_kind(payload)
        || matches!(JsonGroupPayloadCodecV1.decode_pairwise_command(payload),
            Ok(Some(GroupPairwiseCommand::GroupMessage { body, .. })) if mutation_kind(&body))
}

impl ProtocolEngine {
    fn flush_discarded_direct_receives(&mut self) -> anyhow::Result<()> {
        if self
            .pending_decrypted_deliveries
            .iter()
            .any(|delivery| delivery.discarded)
        {
            // Like a normal delivery journal, retain a completion across failed
            // saves. It holds no content and is excluded from disk checkpoints.
            self.persist()?;

        }
        Ok(())
    }
}
