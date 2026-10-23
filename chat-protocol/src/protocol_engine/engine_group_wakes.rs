const MAX_READY_GROUP_PUBLICATIONS: usize = 4_096;

impl ProtocolEngine {
    fn ready_group_publication_is_allowed(&self, pending: &ProtocolReadyGroupPublish) -> bool {
        !pending
            .publish
            .chat_id
            .strip_prefix("group:")
            .is_some_and(|group| self.local_owner_is_inactive_for_group(group))
            && !pending
                .wake_recipient
                .is_some_and(|owner| self.held_direct_chats.contains(&owner.to_hex()))
    }

    fn retire_ineligible_group_publications(&mut self) -> anyhow::Result<()> {
        let rejected = self
            .ready_group_publications
            .iter()
            .filter(|(_, pending)| !self.ready_group_publication_is_allowed(pending))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        if rejected.is_empty() {
            return Ok(());
        }
        self.with_state_checkpoint(|engine| {
            for id in rejected {
                engine.ready_group_publications.remove(&id);
            }
            engine.persist_now()
        })
    }

    fn retain_group_publications(
        &mut self,
        effects: &[ProtocolEffect],
        wake_recipient: Option<NdrOwnerPubkey>,
    ) -> anyhow::Result<()> {
        let added = effects
            .iter()
            .filter(|effect| {
                let ProtocolEffect::Publish(publish) = effect;
                !self
                    .ready_group_publications
                    .contains_key(&publish.event.id.to_hex())
            })
            .count();
        anyhow::ensure!(
            self.ready_group_publications.len().saturating_add(added)
                <= MAX_READY_GROUP_PUBLICATIONS,
            "Outgoing group queue is full"
        );
        for effect in effects {
            let ProtocolEffect::Publish(publish) = effect;
            self.ready_group_publications
                .entry(publish.event.id.to_hex())
                .or_insert_with(|| ProtocolReadyGroupPublish {
                    publish: publish.clone(),
                    wake_recipient,
                });
        }
        Ok(())
    }

    /// Ready group ciphertext remains here until the application's durable
    /// outbox owns it. Reoffering the same signed events never advances ratchets.
    pub fn pending_group_publish_effects(&self) -> Vec<ProtocolEffect> {
        self.ready_group_publications
            .values()
            .filter(|pending| self.ready_group_publication_is_allowed(pending))
            .map(|pending| ProtocolEffect::Publish(pending.publish.clone()))
            .collect()
    }

    pub fn has_pending_group_publish(&self, event_id: &str) -> bool {
        self.ready_group_publications.contains_key(event_id)
    }

    /// Call only after the complete publication batch is committed to the
    /// application outbox (or durably rejected by its block/removal policy).
    pub fn ack_group_publish_effects(&mut self, effects: &[ProtocolEffect]) -> anyhow::Result<()> {
        if !effects.iter().any(|effect| {
            let ProtocolEffect::Publish(publish) = effect;
            self.has_pending_group_publish(&publish.event.id.to_hex())
        }) {
            return Ok(());
        }
        self.with_state_checkpoint(|engine| {
            for effect in effects {
                let ProtocolEffect::Publish(publish) = effect;
                engine
                    .ready_group_publications
                    .remove(&publish.event.id.to_hex());
            }
            engine.persist_now()
        })
    }

    /// Prepare recipient-specific notification delivery in the same checkpoint
    /// as the shared group ciphertext. Shared authors never enter push filters.
    fn prepare_private_group_wakes(
        &mut self,
        group_id: &str,
        output: &mut ProtocolGroupSendResult,
        inner_event_id: Option<String>,
    ) -> anyhow::Result<()> {
        let group = self
            .group_manager
            .group(group_id)
            .ok_or_else(|| anyhow::anyhow!("group is unavailable"))?;
        let own_authors = self.group_sender_event_pubkeys_for_owner(self.owner_pubkey);
        let broadcasts = output
            .effects
            .iter()
            .filter_map(|effect| {
                let ProtocolEffect::Publish(publish) = effect;
                (own_authors.contains(&publish.event.pubkey)
                    && publish.event.tags.public_keys().next().is_none())
                .then_some(publish.event.clone())
            })
            .collect::<Vec<_>>();
        let chat_id = group_chat_id(group_id);
        for event in broadcasts {
            let content = serde_json::json!({
                "v": 1, "type": "group-wake", "event": event,
            })
            .to_string();
            let mut rumor = UnsignedEvent::new(
                self.owner_pubkey,
                event.created_at,
                nostr::Kind::Custom(21_113),
                [],
                content,
            );
            rumor.ensure_id();
            // A wake without an application message ID is still an individual
            // delivery, not replaceable group metadata.
            let delivery_id = inner_event_id
                .clone()
                .or_else(|| rumor.id.map(|id| id.to_hex()));
            let payload = serde_json::to_vec(&rumor)?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
            for recipient_owner in group.members.iter().copied().collect::<BTreeSet<_>>() {
                if recipient_owner == self.local_owner
                    || self
                        .held_direct_chats
                        .contains(&public_owner(recipient_owner)?.to_hex())
                {
                    continue;
                }
                let prepared = self.session_manager.prepare_remote_send(
                    &mut ctx,
                    recipient_owner,
                    payload.clone(),
                )?;
                let prepared = group_publish_from_prepared_send(
                    prepared,
                    GroupPendingFanout::Remote {
                        recipient_owner,
                        payload: payload.clone(),
                    },
                );
                self.queue_group_pending_fanouts(group_id, &prepared, delivery_id.clone());
                let ready = protocol_effects_from_group_prepared_publish(
                    &prepared,
                    self,
                    delivery_id.clone(),
                    chat_id.clone(),
                    &mut output.event_ids,
                )?;
                self.retain_group_publications(&ready, Some(recipient_owner))?;
                output.effects.extend(ready);
            }
            let payload = local_sibling_payload(self.owner_pubkey, &payload)?;
            let prepared = self
                .session_manager
                .prepare_local_sibling_send_reusing_sessions(&mut ctx, payload.clone())?;
            let prepared = group_publish_from_prepared_send(
                prepared,
                GroupPendingFanout::LocalSiblings { payload },
            );
            self.queue_group_pending_fanouts(group_id, &prepared, delivery_id.clone());
            output
                .effects
                .extend(protocol_effects_from_group_prepared_publish(
                    &prepared,
                    self,
                    delivery_id.clone(),
                    chat_id.clone(),
                    &mut output.event_ids,
                )?);
        }
        Ok(())
    }
}
