use super::*;

impl AppCore {
    /// Check every carrier, including saved ciphertext replay. Encryption alone
    /// does not prevent a public signer from identifying the sender.
    pub(in crate::core) fn private_publication_allowed(&self, event: &Event) -> bool {
        let kind = event.kind.as_u16() as u32;
        if kind == u32::from(super::super::calls::push::CALL_WAKE_KIND) {
            let Ok(wake) = serde_json::from_str::<serde_json::Value>(&event.content) else {
                return false;
            };
            let Some(events) = wake.get("events").and_then(serde_json::Value::as_array) else {
                return false;
            };
            return wake.get("type").and_then(serde_json::Value::as_str) == Some("call-wake")
                && wake.get("v").and_then(serde_json::Value::as_u64) == Some(3)
                && (1..=2).contains(&events.len())
                && !self.is_public_identity_signer(event.pubkey)
                && events.iter().all(|inner| {
                    serde_json::from_value::<Event>(inner.clone()).is_ok_and(|inner| {
                        matches!(
                            inner.kind.as_u16() as u32,
                            MESSAGE_EVENT_KIND | INVITE_RESPONSE_KIND
                        ) && self.private_publication_allowed(&inner)
                    })
                });
        }
        let private_envelope = matches!(kind, MESSAGE_EVENT_KIND | INVITE_RESPONSE_KIND);
        let public_invite = kind == INVITE_EVENT_KIND
            && event.tags.iter().any(|tag| {
                let parts = tag.as_slice();
                parts.first().is_some_and(|name| name == "d")
                    && parts
                        .get(1)
                        .is_some_and(|value| value.starts_with("double-ratchet/invites/"))
            });
        if (private_envelope || public_invite)
            && !nostr_double_ratchet::has_metadata_privacy_version(event)
        {
            return false;
        }
        !private_envelope || !self.is_public_identity_signer(event.pubkey)
    }

    fn is_public_identity_signer(&self, signer: PublicKey) -> bool {
        let hex = signer.to_hex();
        self.logged_in.as_ref().is_some_and(|local| {
            signer == local.owner_pubkey || signer == local.device_keys.public_key()
        }) || self
            .protocol_engine
            .as_ref()
            .and_then(ProtocolEngine::local_invite)
            .is_some_and(|invite| invite.inviter_ephemeral_public_key.to_string() == hex)
            || self.app_keys.values().any(|keys| {
                keys.owner_pubkey_hex == hex
                    || keys
                        .devices
                        .iter()
                        .any(|device| device.identity_pubkey_hex == hex)
            })
    }

    /// Keep the old ciphertext and chat history locally, but never replay it.
    pub(in crate::core) fn quarantine_private_publication(
        &mut self,
        pending: &PendingRelayPublish,
    ) {
        let mut retained = pending.clone();
        retained.last_error =
            Some("Private protocol changed; old ciphertext will not be resent".into());
        if let Err(error) = self.app_store.upsert_pending_relay_publish(&retained) {
            self.push_debug_log(
                "publish.privacy",
                format!("quarantine_store_failed={error}"),
            );
        }
        if let (Some(chat), Some(message)) = (&pending.chat_id, &pending.inner_event_id) {
            if self.threads.get(chat).is_some_and(|thread| {
                thread.messages.iter().any(|entry| {
                    entry.id == *message
                        && matches!(
                            entry.delivery,
                            DeliveryState::Queued | DeliveryState::Pending
                        )
                })
            }) {
                self.update_message_delivery(chat, message, DeliveryState::Failed);
            }
        }
    }
}
