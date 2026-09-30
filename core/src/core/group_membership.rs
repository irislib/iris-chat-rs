use super::*;

impl AppCore {
    pub(super) fn group_snapshot_restores_removed_member(
        &self,
        current: &GroupSnapshot,
        incoming: &GroupSnapshot,
    ) -> bool {
        incoming.revision <= current.revision
            && self.logged_in.as_ref().is_some_and(|local| {
                let includes_local = |group: &GroupSnapshot| {
                    group
                        .members
                        .iter()
                        .any(|member| member.to_bytes() == local.owner_pubkey.to_bytes())
                };
                !includes_local(current) && includes_local(incoming)
            })
    }

    pub(super) fn is_removed_from_group(&self, chat_id: &str) -> bool {
        let Some(group) = parse_group_id_from_chat_id(chat_id).and_then(|id| self.groups.get(&id))
        else {
            return false;
        };
        self.logged_in.as_ref().is_some_and(|local| {
            !group
                .members
                .iter()
                .any(|member| member.to_bytes() == local.owner_pubkey.to_bytes())
        })
    }

    pub(super) fn reject_removed_group_send(&mut self, chat_id: &str) -> bool {
        if !self.is_removed_from_group(chat_id) {
            return false;
        }
        self.state.toast = Some("You’re no longer in this group.".to_string());
        self.emit_state();
        true
    }

    pub(super) fn discard_removed_group_publications(&mut self) {
        let removed = self
            .pending_relay_publishes
            .values()
            .filter(|pending| {
                pending.inner_event_id.is_some()
                    && pending
                        .chat_id
                        .as_deref()
                        .is_some_and(|id| self.is_removed_from_group(id))
            })
            .map(|pending| pending.event_id.clone())
            .collect::<Vec<_>>();
        for event_id in removed {
            // Keep membership controls so removal can still reach other devices.
            self.forget_pending_relay_publish(&event_id);
            if let Some(mesh) = &self.device_sync {
                if let Ok(mut outbox) = mesh.nearby_outbox.write() {
                    outbox.forget(&event_id);
                }
            }
        }
    }
}
