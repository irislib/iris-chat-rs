use super::*;
use crate::private_contact_sync::*;

mod history;
mod transport;
pub(super) const PRIVATE_CONTACT_CONTROL_KIND: u32 = 10451;

#[derive(Default)]
pub(super) struct PrivateContactRuntime {
    pub(super) state: Option<PrivateContactSyncState>,
    generation: u64,
    retry_at: Option<u64>,
    history: Option<tokio::task::JoinHandle<()>>,
    last_requests: BTreeMap<String, u64>,
}
impl PrivateContactRuntime {
    pub(super) fn reset(&mut self) {
        self.stop_network();
        self.generation = self.generation.wrapping_add(1);
        self.state = None;
        self.retry_at = None;
        self.last_requests.clear();
    }

    pub(super) fn stop_network(&mut self) {
        if let Some(task) = self.history.take() {
            task.abort();
        }
    }
}

fn opaque_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

impl AppCore {
    pub(super) fn private_contact_state(&mut self) -> anyhow::Result<PrivateContactSyncState> {
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
        let mut state = match self.app_store.load_private_contact_sync(&owner)? {
            Some(json) => restore_private_contact_sync(&json, &owner)?,
            None => create_private_contact_sync(&owner, &opaque_id())?,
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
            state = seed_private_contact(&state, contact, &patch, Some(&opaque_id()))?;
        }
        self.commit_private_contacts(state.clone())?;
        Ok(state)
    }

    fn commit_private_contacts(&mut self, state: PrivateContactSyncState) -> anyhow::Result<()> {
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
            let values = private_contact_values(state, contact);
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
        patch: PrivateContactPatch,
    ) -> bool {
        let result = (|| -> anyhow::Result<Option<PrivateContactDocument>> {
            let state = self.private_contact_state()?;
            let next = edit_private_contact(&state, contact, &patch, Some(&opaque_id()))?;
            if next == state {
                return Ok(None);
            }
            let document = next
                .records
                .get(contact)
                .map(|record| record.document.clone());
            self.commit_private_contacts(next)?;
            Ok(document)
        })();
        match result {
            Ok(Some(document)) => {
                self.send_private_contact_document(&document);
                self.kick_private_contact_sync();
                self.broadcast_device_sync_snapshot();
                true
            }
            Ok(None) => true,
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
        document: &PrivateContactDocument,
    ) -> bool {
        let result = (|| -> anyhow::Result<bool> {
            let state = self.private_contact_state()?;
            let next = stage_private_contact_document(&state, document, Some(&opaque_id()))?;
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

    pub(super) fn seed_legacy_private_contact(&mut self, contact: &str) {
        let result = (|| -> anyhow::Result<()> {
            let state = self.private_contact_state()?;
            let Some(profile) = self.owner_profiles.get(contact) else {
                return Ok(());
            };
            let patch = BTreeMap::from([
                ("nickname".into(), serde_json::json!(profile.nickname)),
                ("note".into(), serde_json::json!(profile.contact_note)),
            ]);
            let next = seed_private_contact(&state, contact, &patch, Some(&opaque_id()))?;
            if next != state {
                self.commit_private_contacts(next)?;
            }
            // Legacy snapshots have no causal field revisions. They can seed
            // unknown values, but cannot resurrect a shared deletion or edit.
            self.project_private_contacts();
            Ok(())
        })();
        if let Err(error) = result {
            self.push_debug_log("private_contacts.seed_failed", error.to_string());
        }
    }
}
