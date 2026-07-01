impl ProtocolEngine {
    fn resolve_message_sender_owner(
        &self,
        envelope: &MessageEnvelope,
    ) -> ProtocolSenderOwnerResolution {
        self.resolve_message_sender_owner_for_sender(envelope.sender)
    }

    fn resolve_message_sender_owner_for_sender(
        &self,
        sender: NdrDevicePubkey,
    ) -> ProtocolSenderOwnerResolution {
        self.session_record_matching_message_sender(sender)
            .map(|record| self.owner_resolution_for_sender_record(record))
            .unwrap_or_else(|| ProtocolSenderOwnerResolution::ProvisionalDeviceOwner {
                owner: provisional_owner_from_sender_pubkey(sender),
            })
    }

    fn resolve_group_pairwise_sender_owner(
        &self,
        sender_owner: NdrOwnerPubkey,
        sender_device: Option<NdrDevicePubkey>,
    ) -> ProtocolSenderOwnerResolution {
        if let Some(sender_device) = sender_device {
            if let Some(record) = self.session_record_for_device_identity(sender_device) {
                return self.owner_resolution_for_sender_record(record);
            }
            if sender_owner == provisional_owner_from_sender_pubkey(sender_device) {
                return ProtocolSenderOwnerResolution::ProvisionalDeviceOwner {
                    owner: sender_owner,
                };
            }
        }
        ProtocolSenderOwnerResolution::Verified {
            owner: sender_owner,
        }
    }

    fn owner_resolution_for_sender_record(
        &self,
        record: ProtocolSenderDeviceRecord,
    ) -> ProtocolSenderOwnerResolution {
        if record.storage_owner == provisional_owner_from_sender_pubkey(record.device_pubkey) {
            ProtocolSenderOwnerResolution::ProvisionalDeviceOwner {
                owner: record.storage_owner,
            }
        } else {
            ProtocolSenderOwnerResolution::Verified {
                owner: record.storage_owner,
            }
        }
    }

    fn session_record_matching_message_sender(
        &self,
        sender: NdrDevicePubkey,
    ) -> Option<ProtocolSenderDeviceRecord> {
        for user in self.session_manager.snapshot().users {
            for record in user.devices {
                let matches_active = record
                    .active_session
                    .as_ref()
                    .is_some_and(|state| session_state_matches_sender(state, sender));
                let matches_inactive = record
                    .inactive_sessions
                    .iter()
                    .any(|state| session_state_matches_sender(state, sender));
                if matches_active || matches_inactive {
                    return Some(ProtocolSenderDeviceRecord {
                        storage_owner: user.owner_pubkey,
                        device_pubkey: record.device_pubkey,
                    });
                }
            }
        }
        None
    }

    fn session_record_for_device_identity(
        &self,
        sender_device: NdrDevicePubkey,
    ) -> Option<ProtocolSenderDeviceRecord> {
        for user in self.session_manager.snapshot().users {
            for record in user.devices {
                if record.device_pubkey == sender_device {
                    return Some(ProtocolSenderDeviceRecord {
                        storage_owner: user.owner_pubkey,
                        device_pubkey: record.device_pubkey,
                    });
                }
            }
        }
        None
    }

    fn ensure_local_roster(&mut self, created_at: NdrUnixSeconds) {
        let has_local_roster = self
            .session_manager
            .snapshot()
            .users
            .into_iter()
            .any(|user| user.owner_pubkey == self.local_owner && user.roster.is_some());
        if !has_local_roster {
            self.session_manager.apply_local_roster(DeviceRoster::new(
                created_at,
                vec![AuthorizedDevice::new(self.local_device, created_at)],
            ));
            self.invalidate_known_message_author_cache();
        }
    }

    fn protocol_group_send_from_prepared(
        &mut self,
        prepared: &GroupPreparedSend,
        inner_event_id: Option<String>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        let mut event_ids = Vec::new();
        let mut effects = Vec::new();
        let chat_id = group_chat_id(&prepared.group_id);
        effects.extend(protocol_effects_from_group_prepared_publish(
            &prepared.local_sibling,
            inner_event_id.clone(),
            chat_id.clone(),
            &mut event_ids,
        )?);
        effects.extend(protocol_effects_from_group_prepared_publish(
            &prepared.remote,
            inner_event_id,
            chat_id,
            &mut event_ids,
        )?);
        Ok(ProtocolGroupSendResult {
            event_ids,
            effects,
            ..Default::default()
        })
    }

    fn sync_group_to_local_siblings(
        &mut self,
        group: &GroupSnapshot,
    ) -> anyhow::Result<(Vec<ProtocolEffect>, Vec<String>)> {
        let now = NdrUnixSeconds(unix_now().get());
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        let prepared = self.group_manager.sync_group_to_local_siblings(
            &mut self.session_manager,
            &mut ctx,
            &group.group_id,
        )?;
        let mut event_ids = Vec::new();
        let effects = protocol_effects_from_group_prepared_publish(
            &prepared,
            None,
            group_chat_id(&group.group_id),
            &mut event_ids,
        )?;
        Ok((effects, Vec::new()))
    }

    fn record_pending_decrypted_delivery(
        &mut self,
        decrypted: ProtocolDecryptedMessage,
        created_at_secs: u64,
    ) {
        let pending = ProtocolPendingDecryptedDelivery {
            sender: decrypted.sender,
            sender_device: decrypted.sender_device,
            conversation_owner: decrypted.conversation_owner,
            content: decrypted.content,
            event_id: decrypted.event_id,
            created_at_secs,
        };
        if !self
            .pending_decrypted_deliveries
            .iter()
            .any(|existing| existing.event_id == pending.event_id)
        {
            self.pending_decrypted_deliveries.push(pending);
        }
    }
}
