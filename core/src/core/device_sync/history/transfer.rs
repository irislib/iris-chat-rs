use super::*;

impl AppCore {
    pub(super) fn ingest_device_history_records(
        &mut self,
        peer: &str,
        state: &mut HistorySession,
        records: Vec<DeviceSyncRecord>,
        requested: Vec<String>,
    ) -> bool {
        if records.len() > 32
            || requested.len() > 32
            || (!requested.is_empty()
                && (requested.len() != state.requested.len()
                    || requested.iter().collect::<BTreeSet<_>>().len() != requested.len()))
            || requested.iter().any(|id| !state.requested.contains(id))
            || records.iter().any(|record| {
                (matches!(record, DeviceSyncRecord::PrivateBlock { .. }) && !state.private_events)
                    || record.scope() != state.scope
                    || !state.filter.contains(record.timestamp())
                    || !state.requested.contains(&hex(&record.id()))
            })
        {
            return false;
        }
        for record in records {
            state.pending_records.insert(hex(&record.id()), record);
        }
        if requested.is_empty() {
            return true;
        }
        let mut messages = Vec::new();
        let mut mutations = Vec::new();
        self.enter_batch();
        for (hash, record) in std::mem::take(&mut state.pending_records) {
            let blocked = match &record {
                DeviceSyncRecord::Message { message } => !self.block_allows_history(
                    &message.chat_id,
                    &message.author,
                    message.created_at,
                ),
                DeviceSyncRecord::Reaction { reaction } => !self.block_allows_history(
                    &reaction.chat_id,
                    &reaction.author,
                    reaction.created_at,
                ),
                DeviceSyncRecord::MessageMutation { mutation } => !self.block_allows_history(
                    &mutation.chat_id,
                    &mutation.author,
                    mutation.created_at,
                ),
                _ => false,
            };
            if blocked {
                // An intentional private block resolves this requested record;
                // it must not keep initial history in a perpetual waiting state.
                state.received.insert(hash);
                continue;
            }

            if let DeviceSyncRecord::Message { mut message } = record {
                if !state.initial {
                    message.legacy_reactions = None;
                }
                messages.push(message);
            } else if matches!(record, DeviceSyncRecord::MessageMutation { .. }) {
                mutations.push((hash, record));
            } else if self.apply_sync_record(record) {
                state.received.insert(hash);
            } else {
                state.withheld = true;
            }
        }
        let incoming = messages
            .iter()
            .map(|message| {
                (
                    message.chat_id.clone(),
                    message.id.clone(),
                    hex(&record_id(&message.chat_id, &message.id)),
                    self.app_store
                        .message_exists_or_deleted(&message.chat_id, Some(&message.id), None)
                        .unwrap_or(true),
                )
            })
            .collect::<Vec<_>>();
        self.apply_device_sync_snapshot(
            DeviceSyncSnapshot {
                roster_at: state.filter.since,
                messages,
                ..Default::default()
            },
            Some(state.filter.since),
        );
        // Originals are admitted before any control is accepted, regardless of
        // packet/hash order within the batch. An unknown target is not stored.
        let target_since = self.device_history_mutation_target_since(peer);
        for (hash, record) in mutations {
            if self.history_mutation_target_allowed(&record, target_since)
                && self.apply_sync_record(record)
            {
                state.received.insert(hash);
            } else {
                state.deferred_mutation = true;
                state.withheld = true;
            }
        }
        self.rebuild_state();
        self.emit_state();
        self.exit_batch();
        self.persist_best_effort_inner();
        for (chat, id, hash, existed) in incoming {
            if self
                .app_store
                .message_exists_or_deleted(&chat, Some(&id), None)
                .unwrap_or(false)
            {
                if state.received.insert(hash) && !existed {
                    state.batch_imported += 1;
                    state.imported_original = true;
                }
            } else {
                state.withheld = true;
            }
        }
        if requested.iter().any(|id| !state.received.contains(id)) {
            state.withheld = true;
        }
        for id in requested {
            state.requested.remove(&id);
            state.received.remove(&id);
        }
        if state.initial {
            if !self.add_device_history_imported(peer, state.batch_imported) {
                state.withheld = true;
            }
            self.update_device_history_progress(
                peer,
                crate::DeviceHistorySyncPhase::Transferring,
                None,
            );
        }
        state.batch_imported = 0;
        true
    }
}
