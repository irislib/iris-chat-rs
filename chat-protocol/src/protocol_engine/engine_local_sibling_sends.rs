impl ProtocolEngine {
    fn prepare_pending_local_sibling_send(
        &mut self,
        pending: &mut ProtocolPendingLocalSiblingSend,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<(Vec<ProtocolEffect>, bool)> {
        pending.next_retry_at_secs = next_pending_retry_at_secs(pending.created_at_secs, now);
        // The provisional one-device roster created at startup cannot prove
        // there are no other devices. Keep the intent until discovery finishes.
        if !self.local_app_keys_observed && !self.has_authoritative_local_roster() {
            return Ok((Vec::new(), false));
        }
        let snapshot = self.session_manager.snapshot();
        let Some(devices) =
            user_record_snapshot(&snapshot, self.local_owner).and_then(roster_device_pubkeys)
        else {
            return Ok((Vec::new(), false));
        };
        let targets = devices
            .into_iter()
            .filter(|device| {
                *device != self.local_device && !pending.completed_devices.contains(device)
            })
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return Ok((Vec::new(), true));
        }
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        let prepared = self.session_manager.prepare_local_sibling_send_to_devices(
            &mut ctx,
            targets,
            pending.payload.clone(),
        )?;
        let mut event_ids = Vec::new();
        let effects = protocol_effects_from_prepared(
            &prepared,
            self.local_handshake_owner_proof(),
            Some(pending.message_id.clone()),
            pending.chat_id.clone(),
            &mut event_ids,
        )?;
        pending.completed_devices.extend(
            prepared
                .deliveries
                .iter()
                .map(|delivery| delivery.device_pubkey),
        );
        Ok((effects, prepared.relay_gaps.is_empty()))
    }

    fn retry_pending_local_sibling_sends(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        if !self
            .pending_local_sibling_sends
            .iter()
            .any(|pending| pending.next_retry_at_secs <= now.get())
        {
            return Ok(Vec::new());
        }
        self.with_state_checkpoint(|engine| {
            let mut effects = Vec::new();
            let mut processed = 0usize;
            for mut pending in std::mem::take(&mut engine.pending_local_sibling_sends) {
                if pending.next_retry_at_secs > now.get()
                    || processed >= PENDING_GROUP_FANOUT_RETRY_BATCH_SIZE
                {
                    engine.pending_local_sibling_sends.push(pending);
                    continue;
                }
                processed += 1;
                // A malformed session must not lose this or later queued work.
                let sessions = engine.session_manager.clone();
                match engine.prepare_pending_local_sibling_send(&mut pending, now) {
                    Ok((ready, complete)) => {
                        effects.extend(ready);
                        if !complete {
                            engine.pending_local_sibling_sends.push(pending);
                        }
                    }
                    Err(_) => {
                        engine.session_manager = sessions;
                        engine.pending_local_sibling_sends.push(pending);
                    }
                }
            }
            engine.persist()?;
            Ok(effects)
        })
    }
}
