impl ProtocolEngine {
    fn queue_remote_payload(
        &mut self,
        peer: PublicKey,
        chat_id: &str,
        payload: Vec<u8>,
        inner_event_id: Option<String>,
        message_id: &str,
        now: UnixSeconds,
    ) -> anyhow::Result<(Vec<String>, Vec<ProtocolEffect>)> {
        let recipient_owner = ndr_owner(peer);
        let existing = self
            .pending_remote_sends
            .iter()
            .position(|pending| {
                pending.recipient_owner == recipient_owner
                    && pending.message_id == message_id
                    && pending.chat_id == chat_id
            })
            .map(|index| self.pending_remote_sends.remove(index));
        let mut pending = if let Some(existing) = existing {
            existing
        } else {
            let eligible_devices = self
                .session_manager
                .roster(recipient_owner)
                .ok_or_else(|| anyhow::anyhow!("missing recipient device list"))?
                .devices()
                .iter()
                .map(|device| device.device_pubkey)
                .collect();
            let expires_at_secs = serde_json::from_slice::<UnsignedEvent>(&payload)
                .ok()
                .and_then(|event| event.tags.expiration().map(|time| time.as_secs()));
            ProtocolPendingRemoteSend {
                recipient_owner,
                eligible_devices,
                completed_devices: BTreeSet::new(),
                chat_id: chat_id.into(),
                payload,
                inner_event_id,
                message_id: message_id.into(),
                expires_at_secs,
                created_at_secs: now.get(),
                next_retry_at_secs: now.get(),
            }
        };
        let (effects, complete) =
            self.prepare_pending_remote_send(&mut pending, NdrUnixSeconds(now.get()))?;
        // The app's existing outbox retains messages with no ready recipient.
        // A partial send succeeds only after at least one real device delivery.
        let event_ids = effects
            .iter()
            .filter_map(|effect| match effect {
                ProtocolEffect::Publish(publish)
                    if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND =>
                {
                    Some(publish.event.id.to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(!event_ids.is_empty(), "no remote target prepared");
        if !complete {
            self.pending_remote_sends.push(pending);
        }
        Ok((event_ids, effects))
    }

    fn prepare_pending_remote_send(
        &mut self,
        pending: &mut ProtocolPendingRemoteSend,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<(Vec<ProtocolEffect>, bool)> {
        pending.next_retry_at_secs = next_pending_retry_at_secs(pending.created_at_secs, now);
        if pending
            .expires_at_secs
            .is_some_and(|expires| expires <= now.get())
            || self.signed_local_device_authorization() == Some(false)
        {
            return Ok((Vec::new(), true));
        }
        let Some(current) = self.session_manager.roster(pending.recipient_owner) else {
            return Ok((Vec::new(), false));
        };
        // Permanently remove revoked targets. Neither later linking nor
        // reauthorizing a device grants it historical queued message bodies.
        pending
            .eligible_devices
            .retain(|device| current.get_device(device).is_some());
        let targets = pending
            .eligible_devices
            .difference(&pending.completed_devices)
            .copied()
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return Ok((Vec::new(), true));
        }
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        let prepared = self.session_manager.prepare_remote_send_to_devices(
            &mut ctx,
            pending.recipient_owner,
            targets,
            pending.payload.clone(),
        )?;
        let effects = protocol_effects_from_prepared(
            &prepared,
            self,
            pending.inner_event_id.clone(),
            pending.chat_id.clone(),
            &mut Vec::new(),
        )?;
        pending.completed_devices.extend(
            prepared
                .deliveries
                .iter()
                .map(|delivery| delivery.device_pubkey),
        );
        Ok((effects, prepared.relay_gaps.is_empty()))
    }

    fn retry_pending_remote_sends(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        if !self
            .pending_remote_sends
            .iter()
            .any(|pending| pending.next_retry_at_secs <= now.get())
        {
            return Ok(Vec::new());
        }
        self.with_state_checkpoint(|engine| {
            let mut effects = Vec::new();
            let mut processed = 0usize;
            for mut pending in std::mem::take(&mut engine.pending_remote_sends) {
                if pending.next_retry_at_secs > now.get()
                    || processed >= PENDING_GROUP_FANOUT_RETRY_BATCH_SIZE
                {
                    engine.pending_remote_sends.push(pending);
                    continue;
                }
                processed += 1;
                let sessions = engine.session_manager.clone();
                match engine.prepare_pending_remote_send(&mut pending, now) {
                    Ok((ready, complete)) => {
                        effects.extend(ready);
                        if !complete {
                            engine.pending_remote_sends.push(pending);
                        }
                    }
                    Err(_) => {
                        engine.session_manager = sessions;
                        engine.pending_remote_sends.push(pending);
                    }
                }
            }
            engine.persist()?;
            Ok(effects)
        })
    }
}
