impl ProtocolEngine {
    pub fn ingest_group_roster_fact_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<Option<ProtocolGroupRosterIngestionResult>> {
        if !is_group_roster_fact_event(event) {
            return Ok(None);
        }
        let fact = parse_group_roster_fact_event(event)?;
        if !self.group_roster_fact_is_authorized(&fact) {
            return Ok(None);
        }

        let checkpoint = self.state_checkpoint();
        let Some(snapshot) = self.remember_group_roster_fact_event(&fact.group_id, event) else {
            return Ok(None);
        };
        let applied = match self.install_group_roster_snapshot(snapshot.clone()) {
            Ok(applied) => applied,
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                return Err(error);
            }
        };
        let mut retry_batch = if applied {
            match self.retry_pending_protocol(NdrUnixSeconds(event.created_at.as_secs())) {
                Ok(retry_batch) => retry_batch,
                Err(error) => {
                    self.restore_checkpoint(checkpoint);
                    return Err(error);
                }
            }
        } else {
            ProtocolRetryBatch::default()
        };
        if applied {
            match self.sync_group_to_local_siblings(&snapshot) {
                Ok(effects) => {
                    retry_batch.group_result.effects.extend(effects);
                }
                Err(error) => {
                    self.restore_checkpoint(checkpoint);
                    return Err(error);
                }
            }
        }
        if let Err(error) = self.persist() {
            self.restore_checkpoint(checkpoint);
            return Err(error);
        }

        Ok(Some(ProtocolGroupRosterIngestionResult {
            snapshot: applied.then_some(snapshot),
            retry_batch,
        }))
    }

    fn remember_group_roster_fact_event(
        &mut self,
        group_id: &str,
        event: &Event,
    ) -> Option<GroupSnapshot> {
        let history = self
            .group_roster_fact_histories
            .entry(group_id.to_string())
            .or_default();
        if history
            .events
            .iter()
            .any(|existing| existing.id == event.id)
        {
            return None;
        }
        history.events.push(event.clone());
        history.events.sort_by(|left, right| {
            left.created_at
                .as_secs()
                .cmp(&right.created_at.as_secs())
                .then_with(|| left.id.to_hex().cmp(&right.id.to_hex()))
        });
        let excess = history
            .events
            .len()
            .saturating_sub(GROUP_ROSTER_FACT_EVENT_HISTORY_LIMIT);
        if excess > 0 {
            history.events.drain(0..excess);
        }
        project_group_roster_fact_events(history.events.iter())
            .into_iter()
            .find(|snapshot| snapshot.group_id == group_id)
    }

    fn group_roster_fact_is_authorized(&self, fact: &GroupRosterFact) -> bool {
        let signer_owner = ndr_owner(fact.signer_pubkey);
        if let Some(current) = self.group_manager.group(&fact.group_id) {
            return current.admins.contains(&signer_owner);
        }

        let local_owner = self.group_manager.snapshot().local_owner_pubkey;
        fact.snapshot.admins.contains(&signer_owner)
            && fact.snapshot.members.contains(&local_owner)
    }

    fn install_group_roster_snapshot(&mut self, snapshot: GroupSnapshot) -> anyhow::Result<bool> {
        if self.group_roster_snapshot_is_stale(&snapshot) {
            return Ok(false);
        }

        let mut manager_snapshot = self.group_manager.snapshot();
        match manager_snapshot
            .groups
            .iter_mut()
            .find(|group| group.group_id == snapshot.group_id)
        {
            Some(group) if *group == snapshot => return Ok(false),
            Some(group) => *group = snapshot,
            None => manager_snapshot.groups.push(snapshot),
        }
        manager_snapshot
            .groups
            .sort_by(|left, right| left.group_id.cmp(&right.group_id));
        self.group_manager = GroupEventManager::from_snapshot(manager_snapshot)?;
        Ok(true)
    }

    fn group_roster_snapshot_is_stale(&self, incoming: &GroupSnapshot) -> bool {
        self.group_manager
            .group(&incoming.group_id)
            .is_some_and(|current| {
                current.revision > incoming.revision
                    || (current.revision == incoming.revision
                        && current.updated_at > incoming.updated_at)
            })
    }

}
