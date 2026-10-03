use super::*;
use std::collections::BTreeSet;

impl AppCore {
    pub(super) fn build_device_sync_snapshot(
        &self,
        roster_at: u64,
        include_messages: bool,
    ) -> DeviceSyncSnapshot {
        let chat_ids = self
            .threads
            .keys()
            .chain(self.chat_read_states.keys())
            .filter(|id| valid_device_sync_chat_id(id))
            .cloned()
            .collect::<BTreeSet<_>>();
        let chats = chat_ids
            .into_iter()
            .filter_map(|id| {
                let read_state = self.chat_read_states.get(&id).cloned().filter(|state| {
                    self.chat_deletions
                        .get(&id)
                        .is_none_or(|deleted| state.updated_at_ms / 1000 > *deleted)
                });
                let thread = self.threads.get(&id);
                if thread.is_none() && read_state.is_none() {
                    return None;
                }
                Some(DeviceSyncChat {
                    updated_at: thread.map_or_else(
                        || {
                            read_state
                                .as_ref()
                                .map_or(0, |state| state.seen_through_secs)
                        },
                        |thread| thread.updated_at_secs,
                    ),
                    // Legacy receivers would bridge these fields to static-key records.
                    id,
                    read_state,
                })
            })
            .collect::<Vec<_>>();
        let direct_chat_ids = chats
            .iter()
            .filter_map(|chat| PublicKey::from_hex(&chat.id).ok())
            .map(|owner| owner.to_hex())
            .collect::<BTreeSet<_>>();
        let mut app_key_owners = direct_chat_ids.clone();
        app_key_owners.extend(
            self.app_keys
                .values()
                .filter(|known| {
                    known
                        .devices
                        .iter()
                        .any(|device| direct_chat_ids.contains(&device.identity_pubkey_hex))
                })
                .map(|known| known.owner_pubkey_hex.clone()),
        );
        if let Some(logged_in) = self.logged_in.as_ref() {
            app_key_owners.insert(logged_in.owner_pubkey.to_hex());
        }
        app_key_owners.extend(
            self.groups
                .values()
                .flat_map(|group| group.members.iter().map(|member| member.to_hex())),
        );
        let app_keys = app_key_owners
            .into_iter()
            .filter_map(|owner| {
                self.app_keys
                    .get(&owner)
                    .and_then(|known| DeviceSyncAppKeys::from_known(&owner, known))
            })
            .collect();
        let groups = self
            .groups
            .values()
            .map(|group| DeviceSyncGroup {
                legacy_message_ttl_seconds: self
                    .chat_message_ttl_seconds
                    .get(&group_chat_id(&group.group_id))
                    .copied(),
                id: group.group_id.clone(),
                name: group.name.clone(),
                description: group.about.clone(),
                picture: group.picture.clone(),
                created_by: group.created_by.to_hex(),
                members: group.members.iter().map(|member| member.to_hex()).collect(),
                admins: group.admins.iter().map(|admin| admin.to_hex()).collect(),
                protocol: Some(
                    match group.protocol.strategy {
                        GroupStrategy::PairwiseFanout => "pairwise_fanout_v1",
                        GroupStrategy::SenderKey => "sender_key_v1",
                    }
                    .to_string(),
                ),
                revision: group.revision,
                created_at: group.created_at.get(),
                updated_at: group.updated_at.get(),
                accepted: Some(true),
            })
            .collect();
        let messages = if include_messages {
            collect_device_sync_messages(self, roster_at, None, DEVICE_SYNC_PAGE_MESSAGES).0
        } else {
            Vec::new()
        };
        DeviceSyncSnapshot {
            roster_at,
            chats,
            chat_mutes: self.chat_mute_snapshot(),
            chat_pins: self.chat_pin_snapshot(),
            private_contacts_v2: self.private_contact_snapshot(),
            private_device_labels_v2: self.private_device_label_snapshot(),
            deleted_chats: self
                .chat_deletions
                .iter()
                .map(|(id, deleted_at)| DeviceSyncChatDeletion {
                    id: id.clone(),
                    deleted_at: *deleted_at,
                })
                .collect(),
            app_keys,
            groups,
            messages,
        }
    }

    pub(super) fn apply_device_sync_snapshot(
        &mut self,
        snapshot: DeviceSyncSnapshot,
        history_since: Option<u64>,
    ) {
        let Some(local_roster_at) = self.device_sync_roster_at() else {
            return;
        };

        let Some(local_owner_hex) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex())
        else {
            return;
        };
        let mut changed = false;
        for state in snapshot.chat_pins {
            match self.merge_chat_pin(state) {
                Ok(merged) => changed |= merged,
                Err(error) => self.push_debug_log("chat_pin.save_failed", error.to_string()),
            }
        }
        for state in snapshot.chat_mutes {
            match self.merge_chat_mute(state) {
                Ok(applied) => changed |= applied,
                Err(error) => self.push_debug_log("chat_mute.save_failed", error.to_string()),
            }
        }
        for deletion in snapshot.deleted_chats {
            if valid_device_sync_chat_id(&deletion.id)
                && deletion.deleted_at > 0
                && deletion.deleted_at <= unix_now().get().saturating_add(300)
            {
                changed |= self.apply_chat_deletion(&deletion.id, deletion.deleted_at);
            }
        }
        let mut app_keys_changed = false;
        let mut app_keys_retry_batch = ProtocolRetryBatch::default();

        for app_keys in snapshot.app_keys {
            let Some((owner, mut incoming, created_at)) = app_keys.into_app_keys() else {
                continue;
            };
            let owner_hex = owner.to_hex();
            let current = self.app_keys.get(&owner_hex).cloned();
            preserve_known_app_key_labels(current.as_ref(), &mut incoming);
            let (_, mut known) = canonical_known_app_keys_snapshot(
                current.as_ref(),
                owner,
                &incoming,
                created_at,
                None,
            );
            // Names have their own clock. A stale membership snapshot can still
            // carry a newer name for a device that remains authorized.
            let label_source = known_app_keys_from_ndr(owner, &incoming, created_at);
            account_app_keys::merge_known_device_labels(&mut known, &label_source);
            let effective = known_app_keys_to_ndr(&known);
            if current.as_ref() == Some(&known) {
                continue;
            }
            if let Err(error) =
                self.invalidate_removed_device_history(owner, current.as_ref(), &known)
            {
                self.push_debug_log("device_sync.history_revoke.error", error.to_string());
                continue;
            }
            let retry_batch = match self.protocol_engine.as_mut() {
                Some(engine) => match engine.ingest_app_keys_snapshot(
                    owner,
                    effective.clone(),
                    known.created_at_secs,
                ) {
                    Ok(batch) => batch,
                    Err(error) => {
                        self.push_debug_log("device_sync.app_keys.error", error.to_string());
                        continue;
                    }
                },
                None => ProtocolRetryBatch::default(),
            };
            self.app_keys.insert(owner_hex, known);
            self.migrate_verified_device_owner_threads(owner, &effective);
            Self::append_protocol_retry_batch(&mut app_keys_retry_batch, retry_batch);
            app_keys_changed = true;
        }
        if app_keys_changed {
            self.reconcile_device_sync();
            self.mark_mobile_push_dirty();
            self.refresh_local_authorization_state();
            changed = true;
        }

        let cutoff = history_since.unwrap_or(local_roster_at.max(snapshot.roster_at));

        for chat in &snapshot.chats {
            if PublicKey::from_hex(&chat.id).is_ok()
                && !self.threads.contains_key(&chat.id)
                && !self.chat_activity_is_deleted(&chat.id, chat.updated_at)
            {
                self.ensure_thread_record(&chat.id, chat.updated_at);
                changed = true;
            }
        }
        for group in snapshot.groups {
            let legacy_ttl = (!self.groups.contains_key(&group.id)
                && !self
                    .chat_message_ttl_seconds
                    .contains_key(&group_chat_id(&group.id)))
            .then_some(group.legacy_message_ttl_seconds)
            .flatten();
            let group_id = group.id.clone();
            if self.chat_activity_is_deleted(&group_chat_id(&group.id), group.updated_at) {
                continue;
            }
            let Some(group) = group.into_group_snapshot() else {
                continue;
            };
            let installed = self
                .protocol_engine
                .as_mut()
                .and_then(|engine| engine.install_device_sync_group(group.clone()).ok())
                .unwrap_or(false);
            if installed {
                if let Some(ttl) = legacy_ttl.filter(|ttl| {
                    *ttl > 0
                        && *ttl <= 9_007_199_254_740_991
                        && !self.has_group_settings_head(&group_id)
                }) {
                    self.chat_message_ttl_seconds
                        .insert(group_chat_id(&group_id), ttl);
                    changed = true;
                }
                let previous = self.groups.get(&group.group_id).cloned();
                if self.apply_group_roster_snapshot(group.clone(), group.updated_at.get()) {
                    self.apply_group_metadata_notice(previous.as_ref(), &group);
                    changed = true;
                }
            }
        }
        for chat in snapshot.chats {
            if valid_device_sync_chat_id(&chat.id) {
                if let Some(read_state) = chat.read_state {
                    changed |= self.apply_chat_read_state(&chat.id, read_state);
                }
            }
        }
        for label in snapshot.private_device_labels_v2 {
            changed |= self.merge_private_device_label(label);
        }
        for document in snapshot.private_contacts_v2 {
            changed |= self.merge_private_contact_from_sibling(&document);
        }
        let now = unix_now().get();
        for message in snapshot.messages {
            if self.chat_activity_is_deleted(&message.chat_id, message.created_at)
                || message.created_at < cutoff
                || message
                    .expires_at
                    .is_some_and(|expires_at| expires_at <= now)
                || !valid_device_sync_chat_id(&message.chat_id)
                || message.id.is_empty()
                || message.id.len() > 128
                || message.body.len() > 32 * 1024
                || PublicKey::from_hex(&message.author).is_err()
                || self.threads.get(&message.chat_id).is_some_and(|thread| {
                    thread.messages.iter().any(|known| known.id == message.id)
                })
                || self
                    .app_store
                    .message_exists_or_deleted(&message.chat_id, Some(&message.id), None)
                    .unwrap_or(true)
            {
                continue;
            }
            let legacy_reactions = message.legacy_reactions.clone().unwrap_or_default();
            let message_id = message.id.clone();
            let is_outgoing = message.author == local_owner_hex;
            let chat_id = message.chat_id.clone();
            let (body, attachments) = extract_message_attachments(&message.body);
            let already_seen = !is_outgoing
                && self.message_was_seen_on_own_device(&chat_id, message.created_at, &message.id);
            let count_unread = !is_outgoing && !already_seen && !self.is_chat_visible(&chat_id);
            if is_outgoing {
                self.accept_direct_peer(&chat_id);
            }
            let thread = self.ensure_thread_record(&chat_id, message.created_at);
            thread.updated_at_secs = thread.updated_at_secs.max(message.created_at);
            if count_unread {
                thread.unread_count = thread.unread_count.saturating_add(1);
            }
            thread.insert_message_sorted(ChatMessageSnapshot {
                system_notice_owner_pubkey_hex: None,
                direct_transfer: None,
                call: None,
                id: message.id,
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: message.author.clone(),
                author_owner_pubkey_hex: Some(message.author),
                author_picture_url: None,
                body,
                attachments,
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing,
                created_at_secs: message.created_at,
                expires_at_secs: message.expires_at,
                delivery: if is_outgoing {
                    DeliveryState::Sent
                } else if already_seen {
                    DeliveryState::Seen
                } else {
                    DeliveryState::Received
                },
                recipient_deliveries: Vec::new(),
                delivery_trace: MessageDeliveryTraceSnapshot::default(),
                source_event_id: None,
            });
            self.apply_legacy_sync_reactions(&chat_id, &message_id, legacy_reactions);
            self.restore_device_sync_reactions(&chat_id, &message_id);
            self.bump_typing_floor(&chat_id, message.created_at);
            if message.expires_at.is_some() {
                self.schedule_next_message_expiry();
            }
            changed = true;
        }
        if changed {
            self.request_protocol_subscription_refresh();
            self.persist_best_effort();
            self.rebuild_state();
            self.emit_state();
        }
        if !app_keys_retry_batch.is_empty() {
            self.process_protocol_engine_retry_batch("device_sync_app_keys", app_keys_retry_batch);
        }
    }
}
