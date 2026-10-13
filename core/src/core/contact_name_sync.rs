use super::*;
use crate::contact_memory::AcceptedNameChange;

pub(super) const NAME_APPROVAL_KIND: u32 = 10455;
const MAX_REVISION: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NameApproval {
    v: u8,
    owner: String,
    pub(super) contact: String,
    revision: u64,
    first_seen_name: String,
    change: AcceptedNameChange,
}

pub(super) fn name_approval(event: &Event) -> Option<NameApproval> {
    if event.kind.as_u16() != NAME_APPROVAL_KIND as u16
        || event.content.len() > 4096
        || event.tags.len() != 2
        || event.verify().is_err()
    {
        return None;
    }
    let state: NameApproval = serde_json::from_str(&event.content).ok()?;
    if state.v != 1
        || state.revision > MAX_REVISION
        || PublicKey::from_hex(&state.owner).ok()?.to_hex() != state.owner
        || PublicKey::from_hex(&state.contact).ok()?.to_hex() != state.contact
        || state.owner == state.contact
        || state.change.accepted_at_secs > event.created_at.as_secs()
        || state.change.previous_name == state.change.accepted_name
        || [
            &state.first_seen_name,
            &state.change.previous_name,
            &state.change.accepted_name,
        ]
        .iter()
        .any(|name| name.trim().is_empty() || name.len() > 320)
        || !event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["p", &state.owner])
        || !event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["d", &format!("iris:name-approval:{}", state.contact)])
    {
        return None;
    }
    Some(state)
}

fn version(event: &Event) -> Option<(u64, u64, String, String)> {
    let state = name_approval(event)?;
    Some((
        state.revision,
        state.change.accepted_at_secs,
        event.pubkey.to_hex(),
        event.id.to_hex(),
    ))
}

#[derive(Default)]
pub(super) struct NameApprovalPolicy {
    events: BTreeMap<String, Event>,
}

impl NameApprovalPolicy {
    fn head(&self) -> Option<&Event> {
        self.events.values().max_by_key(|event| version(event))
    }
}

impl AppCore {
    pub(super) fn name_approval_allowed(&self, event: &Event) -> bool {
        name_approval(event).is_some_and(|state| {
            self.logged_in
                .as_ref()
                .is_some_and(|login| state.owner == login.owner_pubkey.to_hex())
        }) && event.created_at.as_secs() <= unix_now().get().saturating_add(300)
    }

    pub(super) fn project_name_approval(&mut self, event: &Event) {
        let Some(state) = name_approval(event) else {
            return;
        };
        let policy = self
            .name_approvals
            .entry(state.contact.clone())
            .or_default();
        policy.events.insert(event.id.to_hex(), event.clone());
        let Some(head) = policy.head().and_then(name_approval) else {
            return;
        };
        let memory = &mut self
            .owner_profiles
            .entry(state.contact)
            .or_default()
            .contact_memory;
        memory.first_seen_name = Some(head.first_seen_name);
        memory.accepted_name = Some(head.change.accepted_name);
        for event in policy.events.values() {
            if let Some(approval) = name_approval(event) {
                if !memory.name_changes.contains(&approval.change) {
                    memory.name_changes.push(approval.change);
                }
            }
        }
        memory.name_changes.sort_by(|a, b| {
            (a.accepted_at_secs, &a.previous_name, &a.accepted_name).cmp(&(
                b.accepted_at_secs,
                &b.previous_name,
                &b.accepted_name,
            ))
        });
        self.mark_mobile_push_dirty();
        self.bump_user_discovery_revision();
    }

    pub(super) fn write_name_approval(
        &mut self,
        contact: &str,
        first_seen_name: String,
        change: AcceptedNameChange,
        legacy: bool,
    ) -> anyhow::Result<()> {
        let login = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No account"))?;
        let owner = login.owner_pubkey;
        let revision = if legacy {
            0
        } else {
            self.name_approvals
                .get(contact)
                .and_then(|policy| policy.head())
                .and_then(name_approval)
                .map_or(1, |state| state.revision.saturating_add(1))
        };
        anyhow::ensure!(revision <= MAX_REVISION, "Name approval revision exhausted");
        let state = NameApproval {
            v: 1,
            owner: owner.to_hex(),
            contact: contact.into(),
            revision,
            first_seen_name,
            change,
        };
        let event = EventBuilder::new(
            Kind::Custom(NAME_APPROVAL_KIND as u16),
            serde_json::to_string(&state)?,
        )
        .tag(nostr::Tag::public_key(owner))
        .tag(nostr::Tag::identifier(format!(
            "iris:name-approval:{contact}"
        )))
        .allow_self_tagging()
        .sign_with_keys(&login.device_keys)?;
        anyhow::ensure!(
            self.apply_name_approval_event(event.clone()),
            "Name approval not persisted"
        );
        // Keep the signed event private: only authenticated sibling ratchets
        // and the account's State reconciliation carry it, never public relays.
        let content =
            serde_json::json!({"type":"private-name-approval","v":1,"event":event}).to_string();
        let unsigned = EventBuilder::new(Kind::Custom(NAME_APPROVAL_KIND as u16), content)
            .tag(nostr::Tag::public_key(owner))
            .allow_self_tagging()
            .build(owner);
        self.send_protocol_engine_unsigned_event_to_local_siblings(
            owner,
            &owner.to_hex(),
            unsigned,
            "name_approval.self_sync",
        );
        self.broadcast_device_sync_snapshot();
        Ok(())
    }

    pub(super) fn migrate_name_approvals(&mut self) {
        let legacy = self
            .owner_profiles
            .iter()
            .filter_map(|(contact, profile)| {
                if self.name_approvals.contains_key(contact) {
                    return None;
                }
                let memory = &profile.contact_memory;
                let change = memory.name_changes.last()?.clone();
                if memory.accepted_name.as_deref() != Some(&change.accepted_name) {
                    return None;
                }
                Some((contact.clone(), memory.first_seen_name.clone()?, change))
            })
            .collect::<Vec<_>>();
        for (contact, first, change) in legacy {
            if let Err(error) = self.write_name_approval(&contact, first, change, true) {
                self.push_debug_log("name_approval.migrate_failed", error.to_string());
            }
        }
    }

    pub(super) fn receive_name_approval_control(
        &mut self,
        owner: PublicKey,
        device: Option<PublicKey>,
        content: &str,
    ) -> bool {
        if content.len() > 8192
            || self
                .logged_in
                .as_ref()
                .is_none_or(|login| login.owner_pubkey != owner)
            || !device.is_some_and(|device| self.device_sync_peer_is_authorized(&device.to_hex()))
        {
            return true;
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Control {
            #[serde(rename = "type")]
            kind: String,
            v: u8,
            event: Event,
        }
        let Ok(control) = serde_json::from_str::<Control>(content) else {
            return true;
        };
        if control.kind != "private-name-approval"
            || control.v != 1
            || !self.name_approval_allowed(&control.event)
        {
            return true;
        }
        if !self.apply_name_approval_event(control.event) {
            return false;
        }
        self.rebuild_persist_and_emit_state();
        self.broadcast_device_sync_snapshot();
        true
    }
}
