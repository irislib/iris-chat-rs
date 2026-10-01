impl ProtocolEngine {
    /// Retire matching plaintext sibling intents, including intents waiting for
    /// roster discovery. Already sealed events and all other queues are untouched.
    /// Callers must durably migrate the source data first and must not expose this
    /// engine for sending if retirement fails.
    pub fn retire_pending_local_sibling_events(
        &mut self,
        matches: impl Fn(PublicKey, &UnsignedEvent) -> bool,
    ) -> anyhow::Result<usize> {
        self.with_state_checkpoint(|engine| {
            let before = engine.pending_local_sibling_sends.len();
            engine.pending_local_sibling_sends.retain(|pending| {
                let Some((owner, _, payload)) = decode_local_sibling_payload(&pending.payload)
                else {
                    return true;
                };
                !serde_json::from_slice::<UnsignedEvent>(&payload)
                    .is_ok_and(|event| matches(owner, &event))
            });
            let retired = before - engine.pending_local_sibling_sends.len();
            if retired > 0 {
                // This upgrade barrier cannot be deferred by a surrounding batch.
                engine.persist_now()?;
            }
            Ok(retired)
        })
    }
}
