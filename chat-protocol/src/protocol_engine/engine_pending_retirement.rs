impl ProtocolEngine {
    /// Runtime hold while private block state is being applied. The owning app
    /// restores it from signed account state before allowing retry work.
    pub fn hold_pending_direct_sends(&mut self, peer: PublicKey, hold: bool) {
        if hold {
            self.held_direct_chats.insert(peer.to_hex());
        } else {
            self.held_direct_chats.remove(&peer.to_hex());
        }
    }

    /// Durably cancel unsent direct-chat intents after a local block. Group
    /// membership/key controls addressed to this peer keep their group chat ID
    /// and are retained, as are every other conversation's pending deliveries.
    pub fn retire_pending_direct_sends(
        &mut self,
        peer: PublicKey,
        through: Option<u64>,
    ) -> anyhow::Result<usize> {
        let chat_id = peer.to_hex();
        self.with_state_checkpoint(|engine| {
            let before =
                engine.pending_remote_sends.len() + engine.pending_local_sibling_sends.len();
            engine.pending_remote_sends.retain(|pending| {
                pending.recipient_owner != ndr_owner(peer)
                    || pending.chat_id != chat_id
                    || through.is_some_and(|cutoff| {
                        pending_direct_authored_at(&pending.payload, pending.created_at_secs)
                            > cutoff
                    })
            });
            engine.pending_local_sibling_sends.retain(|pending| {
                pending.chat_id != chat_id
                    || through.is_some_and(|cutoff| {
                        decode_local_sibling_payload(&pending.payload).map_or(
                            pending.created_at_secs,
                            |(_, _, payload)| {
                                pending_direct_authored_at(&payload, pending.created_at_secs)
                            },
                        ) > cutoff
                    })
            });
            let retired = before
                - engine.pending_remote_sends.len()
                - engine.pending_local_sibling_sends.len();
            if retired > 0 {
                // A surrounding batch must not defer this privacy barrier.
                engine.persist_now()?;
            }
            Ok(retired)
        })
    }

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

fn pending_direct_authored_at(payload: &[u8], fallback: u64) -> u64 {
    serde_json::from_slice::<UnsignedEvent>(payload)
        .map_or(fallback, |event| event.created_at.as_secs())
}
