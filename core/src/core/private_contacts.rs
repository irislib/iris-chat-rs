use super::*;
use crate::private_contact_sync_v2::*;

mod migration;
mod readiness;
mod transport;
pub(super) use transport::obsolete_private_contact_event;
pub(super) const PRIVATE_CONTACT_CONTROL_KIND: u32 =
    crate::private_contact_sync_v2::PRIVATE_CONTACT_CONTROL_KIND as u32;

#[derive(Default)]
pub(super) struct PrivateContactRuntime {
    pub(super) state: Option<PrivateContactSyncStateV2>,
    generation: u64,
    last_requests: BTreeMap<String, u64>,
    pub(super) last_recovery_request_at: Option<Instant>,
    pub(super) sent_device_labels: Vec<super::private_device_labels::PrivateDeviceLabel>,
}
impl PrivateContactRuntime {
    pub(super) fn reset(&mut self) {
        self.stop_network();
        self.generation = self.generation.wrapping_add(1);
        self.state = None;
        self.last_requests.clear();
        self.last_recovery_request_at = None;
        self.sent_device_labels.clear();
    }

    pub(super) fn stop_network(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

fn opaque_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

impl AppCore {
    pub(super) fn private_contact_state(&mut self) -> anyhow::Result<PrivateContactSyncStateV2> {
        let owner = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no account"))?
            .owner_pubkey
            .to_hex();
        if let Some(state) = self
            .private_contacts
            .state
            .as_ref()
            .filter(|state| state.owner == owner)
        {
            return Ok(state.clone());
        }
        self.restore_private_contact_state(&owner)
    }

    pub(super) fn restore_private_contact_state(
        &mut self,
        owner: &str,
    ) -> anyhow::Result<PrivateContactSyncStateV2> {
        let mut state = match self.app_store.load_private_contact_sync(owner)? {
            Some(json) => migrate_private_contact_sync_v2(&serde_json::from_str(&json)?, owner)?,
            None => create_private_contact_sync_v2(owner, &opaque_id())?,
        };
        // Absence is not a deletion. Counter-zero migration cannot overwrite a
        // real edit from another app, even when that edit is learned later.
        for (contact, profile) in &self.owner_profiles {
            let patch = BTreeMap::from([
                (
                    "favorite".into(),
                    serde_json::json!(profile.contact_memory.favorite),
                ),
                ("nickname".into(), serde_json::json!(profile.nickname)),
                ("note".into(), serde_json::json!(profile.contact_note)),
            ]);
            state = seed_private_contact_v2(&state, contact, &patch)?;
        }
        self.commit_private_contacts(state.clone())?;
        Ok(state)
    }

    fn commit_private_contacts(&mut self, state: PrivateContactSyncStateV2) -> anyhow::Result<()> {
        self.app_store
            .save_private_contact_sync(&state.owner, &serde_json::to_string(&state)?)?;
        self.private_contacts.state = Some(state);
        self.project_private_contacts();
        Ok(())
    }

    pub(super) fn project_private_contacts(&mut self) {
        let Some(state) = self.private_contacts.state.as_ref() else {
            return;
        };
        let mut changed = false;
        for (contact, fields) in &state.contacts {
            let values = private_contact_values_v2(state, contact);
            let profile = self.owner_profiles.entry(contact.clone()).or_default();
            if fields.contains_key("favorite") && profile.contact_memory.favorite != values.favorite
            {
                profile.contact_memory.favorite = values.favorite;
                changed = true;
            }
            if fields.contains_key("nickname") && profile.nickname != values.nickname {
                profile.nickname = values.nickname;
                changed = true;
            }
            if fields.contains_key("note") && profile.contact_note != values.note {
                profile.contact_note = values.note;
                changed = true;
            }
        }
        if changed {
            self.mark_mobile_push_dirty();
        }
    }

    pub(super) fn edit_private_contact_fields(
        &mut self,
        contact: &str,
        patch: PrivateContactPatchV2,
    ) -> bool {
        let result = (|| -> anyhow::Result<bool> {
            let state = self.private_contact_state()?;
            let next = edit_private_contact_v2(&state, contact, &patch)?;
            if next == state {
                return Ok(false);
            }
            self.commit_private_contacts(next)?;
            Ok(true)
        })();
        match result {
            Ok(true) => {
                self.kick_private_contact_sync();
                self.broadcast_device_sync_snapshot();
                true
            }
            Ok(false) => true,
            Err(error) => {
                self.push_debug_log("private_contacts.save_failed", error.to_string());
                self.state.toast = Some("Could not save contact details. Try again.".into());
                self.emit_state();
                false
            }
        }
    }

    pub(super) fn merge_private_contact_from_sibling(
        &mut self,
        document: &PrivateContactDocumentV2,
    ) -> bool {
        let result = (|| -> anyhow::Result<bool> {
            let state = self.private_contact_state()?;
            let next = merge_private_contact_document_v2(&state, document)?;
            if state == next {
                return Ok(false);
            }
            self.commit_private_contacts(next)?;
            self.kick_private_contact_sync();
            Ok(true)
        })();
        match result {
            Ok(changed) => changed,
            Err(error) => {
                self.push_debug_log("private_contacts.merge_failed", error.to_string());
                false
            }
        }
    }
}
