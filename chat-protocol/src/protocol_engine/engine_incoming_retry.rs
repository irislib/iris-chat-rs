impl ProtocolEngine {
    pub fn has_due_pending_retry_work(&self, now: NdrUnixSeconds) -> bool {
        let _ = now;
        false
    }

    pub fn process_direct_message_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<Option<ProtocolDecryptedMessage>> {
        let envelope = parse_message_event(event)?;

        match self.decrypt_direct_message_envelope(event, &envelope, true) {
            Ok(Some(decrypted)) => Ok(Some(decrypted)),
            Ok(None) => Ok(None),
            Err(_error) if protocol_event_has_tag(event, "header") && !self.is_known_message_author(event.pubkey) => {
                let _ = parse_group_sender_key_message_event_unchecked(event);
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn process_group_outer_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let has_header = protocol_event_has_tag(event, "header");
        let parsed = if has_header {
            parse_group_sender_key_message_event_unchecked(event)
        } else {
            parse_group_sender_key_message_event(event)
        };
        let Ok(parsed) = parsed else {
            return Ok(ProtocolGroupIncomingResult::default());
        };
        let Some(message) = self.group_sender_key_message_from_parsed(&parsed) else {
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                pending: true,
                ..Default::default()
            });
        };
        let mut result = self.handle_group_sender_key_message(message)?;
        result.consumed = true;
        Ok(result)
    }

    pub fn process_group_pairwise_payload(
        &mut self,
        payload: &[u8],
        from_owner_pubkey: PublicKey,
        from_sender_device_pubkey: Option<PublicKey>,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let (is_group_payload, is_supported_group_payload) =
            classify_group_pairwise_payload(payload).unwrap_or((false, false));
        if is_group_payload && !is_supported_group_payload {
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        }

        let sender_device = from_sender_device_pubkey.map(ndr_device);
        let sender_owner = match self.resolve_group_pairwise_sender_owner(
            ndr_owner(from_owner_pubkey),
            sender_device,
        ) {
            ProtocolSenderOwnerResolution::Verified { owner }
            | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => owner,
        };

        let result = match sender_device {
            Some(device_pubkey) => self
                .group_manager
                .handle_pairwise_payload(sender_owner, device_pubkey, payload),
            None => self.group_manager.handle_incoming(sender_owner, payload),
        };

        match result {
            Ok(Some(GroupIncomingEvent::SenderKeyRepairRequested(repair))) => {
                let effects = self.sender_key_repair_response_effects(
                    repair.requester_owner,
                    &repair.request,
                    NdrUnixSeconds(unix_now().get()),
                )?;
                self.persist()?;
                Ok(ProtocolGroupIncomingResult {
                    effects,
                    consumed: true,
                    ..Default::default()
                })
            }
            Ok(Some(event)) => {
                let mut effects = Vec::new();
                if sender_owner != self.local_owner {
                    if let GroupIncomingEvent::MetadataUpdated(group) = &event {
                        let (sync_effects, _) = self.sync_group_to_local_siblings(group)?;
                        effects.extend(sync_effects);
                    }
                }
                self.persist()?;
                Ok(ProtocolGroupIncomingResult {
                    events: vec![event],
                    effects,
                    consumed: true,
                    ..Default::default()
                })
            }
            Ok(None) => Ok(ProtocolGroupIncomingResult {
                consumed: is_group_payload,
                pending: is_supported_group_payload,
                ..Default::default()
            }),
            Err(_error) if is_supported_group_payload => Ok(ProtocolGroupIncomingResult {
                consumed: true,
                pending: true,
                ..Default::default()
            }),
            Err(error) => Err(error.into()),
        }
    }

    pub fn ack_pending_decrypted_deliveries(&mut self) -> anyhow::Result<()> {
        if self.pending_decrypted_deliveries.is_empty() {
            return Ok(());
        }
        self.pending_decrypted_deliveries.clear();
        self.persist()
    }

    fn decrypt_direct_message_envelope(
        &mut self,
        event: &Event,
        envelope: &MessageEnvelope,
        record_delivery: bool,
    ) -> anyhow::Result<Option<ProtocolDecryptedMessage>> {
        let sender_owner = match self.resolve_message_sender_owner(envelope) {
            ProtocolSenderOwnerResolution::Verified { owner }
            | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => owner,
        };
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(event.created_at.as_secs()), &mut rng);
        let Some(received) = self
            .session_manager
            .receive(&mut ctx, sender_owner, envelope)?
        else {
            return Ok(None);
        };
        self.invalidate_known_message_author_cache();
        let (conversation_owner, payload) = decode_local_sibling_payload(&received.payload)
            .map(|(owner, payload)| (Some(owner), payload))
            .unwrap_or((None, received.payload));
        let content = String::from_utf8(payload)?;
        let decrypted = ProtocolDecryptedMessage {
            sender: public_owner(received.owner_pubkey)?,
            sender_device: Some(public_device(received.device_pubkey)?),
            conversation_owner,
            content,
            event_id: Some(event.id.to_string()),
        };
        if record_delivery {
            self.record_pending_decrypted_delivery(decrypted.clone(), event.created_at.as_secs());
        }
        self.persist()?;
        Ok(Some(decrypted))
    }
}
