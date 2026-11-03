use super::*;

// These facts are derived from the outgoing message row. Old installations
// already have them, including for messages outside the loaded history page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::core) struct RecipientReceipt {
    pub chat_id: String,
    pub message_id: String,
    pub created_at: u64,
    pub recipient: String,
    pub seen: bool,
    pub updated_at: u64,
}

impl RecipientReceipt {
    pub(super) fn record_id(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        // Two devices may observe the same authenticated receipt at different times.
        let key = serde_json::json!([
            "recipientReceipt",
            self.chat_id,
            self.message_id,
            self.created_at,
            self.recipient,
            self.seen
        ]);
        Sha256::digest(key.to_string().as_bytes()).into()
    }

    pub(super) fn sync_time(&self, receipt_link_at: Option<u64>) -> u64 {
        self.created_at.max(receipt_link_at.unwrap_or(0))
    }

    pub(in crate::core) fn from_delivery(
        chat: &str,
        id: &str,
        created_at: u64,
        delivery: &MessageRecipientDeliverySnapshot,
    ) -> Option<Self> {
        let seen = match delivery.delivery {
            DeliveryState::Seen => true,
            DeliveryState::Received => false,
            _ => return None,
        };
        Some(Self {
            chat_id: chat.into(),
            message_id: id.into(),
            created_at,
            // Older direct rows can retain the authenticated device alias.
            recipient: if is_group_chat_id(chat) {
                delivery.owner_pubkey_hex.clone()
            } else {
                chat.into()
            },
            seen,
            updated_at: delivery.updated_at_secs,
        })
    }

    pub(super) fn excluded(&self, core: &AppCore) -> bool {
        !core.block_allows_history(&self.chat_id, &self.recipient, self.created_at)
            || core.chat_activity_is_deleted(&self.chat_id, self.created_at)
            || core
                .app_store
                .message_was_locally_deleted(&self.chat_id, Some(&self.message_id), None)
                .unwrap_or(true)
    }

    pub(super) fn valid(&self) -> bool {
        valid_device_sync_chat_id(&self.chat_id)
            && !self.message_id.is_empty()
            && self.message_id.len() <= 128
            && PublicKey::from_hex(&self.recipient).is_ok()
            && (is_group_chat_id(&self.chat_id) || self.chat_id == self.recipient)
            && self.created_at.max(self.updated_at) <= unix_now().get().saturating_add(300)
    }

    fn matches_target(&self, message: &ChatMessageSnapshot) -> bool {
        message.is_outgoing
            && matches!(message.kind, ChatMessageKind::User)
            && message.created_at_secs == self.created_at
            && message.expires_at_secs.is_none_or(|t| t > unix_now().get())
            && !message.deleted_for_everyone
    }

    pub(super) fn current_on(&self, core: &AppCore) -> bool {
        !self.excluded(core)
            && core
                .message_for_mutation(&self.chat_id, &self.message_id)
                .is_some_and(|m| {
                    self.matches_target(&m)
                        && m.recipient_deliveries.iter().any(|r| {
                            (self.chat_id == self.recipient || r.owner_pubkey_hex == self.recipient)
                                && match r.delivery {
                                    DeliveryState::Received => !self.seen,
                                    DeliveryState::Seen => self.seen,
                                    _ => false,
                                }
                        })
                })
    }

    pub(super) fn apply(&self, core: &mut AppCore) -> Option<bool> {
        let mut message = core.message_for_mutation(&self.chat_id, &self.message_id)?;
        if !self.matches_target(&message) || self.excluded(core) {
            return None;
        }
        let delivery = if self.seen {
            DeliveryState::Seen
        } else {
            DeliveryState::Received
        };
        if !super::super::chat_receipts::apply_message_receipt(
            &mut message,
            &delivery,
            false,
            Some(&self.recipient),
            self.updated_at,
        ) {
            return Some(false);
        }
        if let Some(loaded) = core
            .threads
            .get_mut(&self.chat_id)
            .and_then(|t| t.messages.iter_mut().find(|m| m.id == self.message_id))
        {
            loaded.delivery = message.delivery;
            loaded.recipient_deliveries = message.recipient_deliveries;
        } else if let Err(error) =
            core.app_store
                .save_message_receipts(&self.chat_id, &[message], None)
        {
            core.push_debug_log("storage.messages.receipt.error", error.to_string());
            return None;
        }
        core.persist_best_effort();
        Some(true)
    }
}

impl AppCore {
    pub(in crate::core) fn broadcast_device_sync_receipts(&self, receipts: &[RecipientReceipt]) {
        let Some(runtime) = &self.device_sync else {
            return;
        };
        for sibling in &runtime.siblings {
            let peer = sibling.pubkey().to_string();
            if !self.device_sync_peer_is_authorized(&peer) {
                continue;
            }
            let since = self.device_history_mutation_target_since(&peer);
            for chunk in receipts.chunks(DEVICE_SYNC_RECORD_BATCH) {
                let receipts = chunk
                    .iter()
                    .filter(|r| r.created_at >= since && !r.excluded(self))
                    .cloned()
                    .collect::<Vec<_>>();
                if !receipts.is_empty() {
                    self.send_history_packets(
                        &peer,
                        vec![DeviceSyncPacket::RecipientReceipts {
                            v: DEVICE_SYNC_VERSION,
                            receipts,
                        }],
                    );
                }
            }
        }
    }

    pub(super) fn receive_device_sync_receipts(
        &mut self,
        peer: &str,
        receipts: Vec<RecipientReceipt>,
    ) {
        if receipts.len() > DEVICE_SYNC_RECORD_BATCH {
            return;
        }
        let since = self.device_history_mutation_target_since(peer);
        self.enter_batch();
        let mut missing = false;
        let mut changed = false;
        for receipt in receipts {
            if receipt.created_at < since || !receipt.valid() || receipt.excluded(self) {
                continue;
            }
            match receipt.apply(self) {
                Some(merged) => changed |= merged,
                None => missing = true,
            }
        }
        if changed {
            self.rebuild_state();
            self.emit_state();
        }
        self.exit_batch();
        if missing {
            // The compact live update may have overtaken the original message.
            self.request_device_sync_snapshot(peer, None);
        }
    }

    // After an approved full transfer, the ordinary history window starts at
    // link time. Receipts for already-shared originals remain eligible there.
    // The original itself still goes through the normal message history gate.
    pub(super) fn receipt_history_link_at(&self, peer: &str, since: u64) -> Option<u64> {
        self.device_history_transfer(peer)
            .filter(|r| r.policy_known && r.since == 0 && r.link_at == since)
            .map(|r| r.link_at)
    }

    pub(super) fn visit_recipient_receipts(
        &self,
        since: u64,
        until: u64,
        mut visit: impl FnMut(RecipientReceipt) -> bool,
    ) -> anyhow::Result<bool> {
        let now = unix_now().get();
        let mut visit_message =
            |chat: &str,
             id: &str,
             created_at: u64,
             deliveries: &[MessageRecipientDeliverySnapshot]| {
                deliveries
                    .iter()
                    .filter_map(|d| RecipientReceipt::from_delivery(chat, id, created_at, d))
                    .all(&mut visit)
            };
        let mut loaded = HashSet::new();
        for thread in self.threads.values() {
            for m in &thread.messages {
                loaded.insert((thread.chat_id.as_str(), m.id.as_str()));
                if m.is_outgoing
                    && matches!(m.kind, ChatMessageKind::User)
                    && !m.deleted_for_everyone
                    && (since..=until).contains(&m.created_at_secs)
                    && m.expires_at_secs.is_none_or(|t| t > now)
                    && !visit_message(
                        &m.chat_id,
                        &m.id,
                        m.created_at_secs,
                        &m.recipient_deliveries,
                    )
                {
                    return Ok(false);
                }
            }
        }
        // Only receipt columns are needed, even when reconciling a long history
        // of attachments. Release the database lock before admission checks.
        let mut after = (String::new(), String::new());
        loop {
            let rows = {
                let db = self.app_store.shared();
                let conn = db.lock().map_err(|_| anyhow::anyhow!("Storage lock"))?;
                let mut query = conn.prepare_cached(
                    "SELECT chat_id,id,created_at_secs,recipient_deliveries_json
                    FROM messages WHERE is_outgoing=1 AND kind='user' AND deleted_for_everyone=0
                    AND recipient_deliveries_json != '[]' AND created_at_secs BETWEEN ?1 AND ?2
                    AND (expires_at_secs IS NULL OR expires_at_secs > ?3)
                    AND (chat_id,id) > (?4,?5) ORDER BY chat_id,id LIMIT 256",
                )?;
                let rows = query.query_map(
                    rusqlite::params![since, until, now, after.0, after.1],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, u64>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            if rows.is_empty() {
                return Ok(true);
            }
            for (chat, id, created_at, json) in rows {
                if !loaded.contains(&(chat.as_str(), id.as_str())) {
                    let deliveries =
                        serde_json::from_str::<Vec<MessageRecipientDeliverySnapshot>>(&json)
                            .unwrap_or_default();
                    if !visit_message(&chat, &id, created_at, &deliveries) {
                        return Ok(false);
                    }
                }
                after = (chat, id);
            }
        }
    }
}
