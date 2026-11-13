use super::*;

pub(super) const MAX_READY_GROUP_PUBLICATIONS: usize = 4_096;

impl ProtocolEngine {
    pub fn owns_group_data_stream(&self, chat_id: &str, author: PublicKey) -> bool {
        let Some(group) = chat_id.strip_prefix("group:") else {
            return false;
        };
        self.group_manager.group_has_member(group, self.local_owner) == Some(true)
            && self.group_manager.is_local_sender_key_stream(
                group,
                ndr_device(author),
                self.local_device,
            )
    }

    /// Authors learned from private distributions for groups this device belongs to.
    pub fn subscribable_group_sender_event_pubkeys(&self) -> Vec<PublicKey> {
        self.group_manager
            .subscribable_sender_event_pubkeys()
            .into_iter()
            .filter_map(|a| public_device(a).ok())
            .collect()
    }

    fn ready_group_publication_is_allowed(&self, pending: &ProtocolReadyGroupPublish) -> bool {
        // Envelopes sealed before carriers were recorded still name their
        // recipient device. They are retired here, never offered again.
        pending.publish.is_publishable()
            && (pending.publish.carrier != ProtocolCarrier::SharedGroup
                || self
                    .owns_group_data_stream(&pending.publish.chat_id, pending.publish.event.pubkey))
            && !pending
                .publish
                .chat_id
                .strip_prefix("group:")
                .is_some_and(|group| self.local_owner_is_inactive_for_group(group))
            && !pending
                .wake_recipient
                .is_some_and(|owner| self.held_direct_chats.contains(&owner.to_hex()))
    }

    pub(super) fn retire_ineligible_group_publications(&mut self) -> anyhow::Result<()> {
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

    pub(super) fn retain_group_publications(
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
}
