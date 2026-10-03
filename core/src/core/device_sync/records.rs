use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

mod capture;
mod legacy;
mod projection;
mod store;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum RecordScope {
    History,
    State,
}

#[derive(Clone)]
pub(super) enum RecordLocator {
    Group(String),
    Head(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(super) enum DeviceSyncRecord {
    Message { message: DeviceSyncMessage },
    Reaction { reaction: DeviceSyncReaction },
    Group { group: DeviceSyncGroup },
    GroupSettings { settings: DeviceSyncGroupSettings },
    Profile { event: Event },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeviceSyncReaction {
    pub chat_id: String,
    pub id: String,
    pub author: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<u64>,
    pub message_id: String,
    pub emoji: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeviceSyncGroupSettings {
    pub group_id: String,
    pub id: String,
    pub author: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<u64>,
    pub message_ttl_seconds: Option<u64>,
}

impl DeviceSyncRecord {
    pub(super) fn id(&self) -> [u8; 32] {
        let value = match self {
            Self::Message { message } => serde_json::json!([message.chat_id, message.id]),
            Self::Reaction { reaction: r } => serde_json::json!(["reaction", r.chat_id, r.id]),
            Self::Profile { event } => {
                serde_json::json!(["profile", event.pubkey.to_hex(), event.id.to_hex()])
            }
            Self::GroupSettings { settings: s } => {
                serde_json::json!(["groupSettings", s.group_id, s.id])
            }
            Self::Group { group: g } => serde_json::json!([
                "group",
                g.id,
                g.revision,
                g.updated_at,
                g.name,
                g.description,
                g.picture,
                g.created_by,
                g.members.iter().collect::<BTreeSet<_>>(),
                g.admins.iter().collect::<BTreeSet<_>>(),
                g.protocol.as_deref().unwrap_or("pairwise_fanout_v1"),
                g.created_at
            ]),
        };
        Sha256::digest(value.to_string().as_bytes()).into()
    }
    pub(super) fn fits_packet(&self) -> bool {
        serde_json::to_vec(&DeviceSyncPacket::HistoryRecords {
            v: 1,
            session: "0".repeat(32),
            records: vec![self.clone()],
            requested: Vec::new(),
        })
        .is_ok_and(|bytes| bytes.len() <= DEVICE_SYNC_MAX_PACKET_BYTES)
    }
    pub(super) fn timestamp(&self) -> u64 {
        match self {
            Self::Message { message } => message.created_at,
            Self::Reaction { reaction } => reaction.created_at,
            _ => 0,
        }
    }
    pub(super) fn scope(&self) -> RecordScope {
        match self {
            Self::Message { .. } | Self::Reaction { .. } => RecordScope::History,
            _ => RecordScope::State,
        }
    }
    pub(super) fn locator(&self) -> Option<RecordLocator> {
        if let Self::Group { group } = self {
            return Some(RecordLocator::Group(group.id.clone()));
        }
        self.storage_key().map(RecordLocator::Head)
    }
    fn storage_key(&self) -> Option<String> {
        Some(match self {
            Self::Reaction { reaction: r } => {
                serde_json::json!(["reaction", r.chat_id, r.message_id, r.author]).to_string()
            }
            Self::GroupSettings { settings: s } => {
                serde_json::json!(["groupSettings", s.group_id]).to_string()
            }
            Self::Profile { event } => {
                serde_json::json!(["profile", event.pubkey.to_hex()]).to_string()
            }
            _ => return None,
        })
    }
    fn wins(&self, previous: &Self) -> bool {
        match (self, previous) {
            (Self::Reaction { reaction: a }, Self::Reaction { reaction: b }) => {
                (effective_ms(a.created_at, a.created_at_ms), &a.id)
                    > (effective_ms(b.created_at, b.created_at_ms), &b.id)
            }
            (Self::GroupSettings { settings: a }, Self::GroupSettings { settings: b }) => {
                (effective_ms(a.created_at, a.created_at_ms), &a.id)
                    > (effective_ms(b.created_at, b.created_at_ms), &b.id)
            }
            (Self::Profile { event: a }, Self::Profile { event: b }) => {
                a.created_at > b.created_at || (a.created_at == b.created_at && a.id < b.id)
            }
            _ => false,
        }
    }
}

fn effective_ms(seconds: u64, millis: Option<u64>) -> u64 {
    millis.unwrap_or(seconds.saturating_mul(1000))
}
fn valid_time(seconds: u64, millis: Option<u64>) -> bool {
    seconds <= unix_now().get().saturating_add(300)
        && millis.is_none_or(|ms| ms <= 9_007_199_254_740_991 && ms / 1000 == seconds)
}

impl AppCore {
    fn sync_record_author_allowed(&self, chat: &str, author: &str, admin: bool) -> bool {
        let Some(local) = self
            .logged_in
            .as_ref()
            .map(|login| login.owner_pubkey.to_hex())
        else {
            return false;
        };
        if let Some(group_id) = parse_group_id_from_chat_id(chat) {
            self.groups.get(&group_id).is_some_and(|group| {
                group.members.iter().any(|key| key.to_hex() == local)
                    && (if admin { &group.admins } else { &group.members })
                        .iter()
                        .any(|key| key.to_hex() == author)
            })
        } else {
            !admin && self.threads.contains_key(chat) && (author == local || author == chat)
        }
    }
    pub(super) fn sync_record_allowed(&self, record: &DeviceSyncRecord) -> bool {
        if !record.fits_packet() {
            return false;
        }
        match record {
            DeviceSyncRecord::Message { message } => {
                messages::history_message_allowed(self, message)
            }
            DeviceSyncRecord::Reaction { reaction: r } => {
                !r.id.is_empty()
                    && r.id.len() <= 128
                    && !r.message_id.is_empty()
                    && r.message_id.len() <= 128
                    && r.emoji.len() <= 256
                    && valid_time(r.created_at, r.created_at_ms)
                    && !self.sync_reaction_target_expired(&r.chat_id, &r.message_id)
                    && self.sync_record_author_allowed(&r.chat_id, &r.author, false)
                    && !self.chat_activity_is_deleted(&r.chat_id, r.created_at)
                    && !self
                        .app_store
                        .message_was_locally_deleted(&r.chat_id, Some(&r.message_id), None)
                        .unwrap_or(true)
            }
            DeviceSyncRecord::GroupSettings { settings: s } => {
                !s.id.is_empty()
                    && s.id.len() <= 128
                    && valid_time(s.created_at, s.created_at_ms)
                    && s.message_ttl_seconds
                        .is_none_or(|ttl| ttl > 0 && ttl <= 9_007_199_254_740_991)
                    && self.sync_record_author_allowed(
                        &format!("group:{}", s.group_id),
                        &s.author,
                        true,
                    )
            }
            DeviceSyncRecord::Profile { event } => {
                event.kind == Kind::Metadata
                    && event.verify().is_ok()
                    && event.content.len() <= 32 * 1024
                    && event.tags.len() <= 256
                    && event.created_at.as_secs() <= unix_now().get().saturating_add(300)
                    && self.profile_sync_owner_allowed(event.pubkey)
            }
            DeviceSyncRecord::Group { group } => {
                group.clone().into_group_snapshot().is_some()
                    && self
                        .logged_in
                        .as_ref()
                        .is_some_and(|login| group.members.contains(&login.owner_pubkey.to_hex()))
            }
        }
    }
    fn sync_reaction_target_expired(&self, chat: &str, id: &str) -> bool {
        let expiry = self
            .threads
            .get(chat)
            .and_then(|thread| thread.messages.iter().find(|m| m.id == id))
            .and_then(|m| m.expires_at_secs)
            .or_else(|| {
                self.app_store
                    .load_messages_around(chat, id, 0, 0)
                    .ok()?
                    .into_iter()
                    .next()?
                    .expires_at_secs
            });
        expiry.is_some_and(|until| until <= unix_now().get())
    }
    fn profile_sync_owner_allowed(&self, owner: PublicKey) -> bool {
        let Some(local) = self
            .logged_in
            .as_ref()
            .map(|login| login.owner_pubkey.to_hex())
        else {
            return false;
        };
        let owner = owner.to_hex();
        owner == local
            || (self.threads.contains_key(&owner) && !self.chat_deletions.contains_key(&owner))
            || self.groups.values().any(|group| {
                group.members.iter().any(|key| key.to_hex() == local)
                    && group.members.iter().any(|key| key.to_hex() == owner)
            })
    }
    pub(super) fn apply_sync_record(&mut self, record: DeviceSyncRecord) -> bool {
        if let DeviceSyncRecord::Reaction { reaction } = &record {
            if self
                .app_store
                .message_was_locally_deleted(&reaction.chat_id, Some(&reaction.message_id), None)
                .unwrap_or(false)
                || self.chat_activity_is_deleted(&reaction.chat_id, reaction.created_at)
                || self.sync_reaction_target_expired(&reaction.chat_id, &reaction.message_id)
            {
                return true;
            }
        }
        if !self.sync_record_allowed(&record) {
            return false;
        }
        match record {
            DeviceSyncRecord::Message { .. } => false,
            DeviceSyncRecord::Group { group } => {
                let expected = group.clone();
                self.apply_device_sync_snapshot(
                    DeviceSyncSnapshot {
                        groups: vec![group],
                        ..Default::default()
                    },
                    None,
                );
                self.persist_best_effort_inner();
                self.sync_group_is_durable(&expected)
            }
            DeviceSyncRecord::Profile { event } => {
                self.apply_profile_metadata_event(&event);
                self.persist_best_effort();
                self.sync_record_is_stored(&DeviceSyncRecord::Profile { event })
            }
            record => {
                let Ok(accepted) = self.store_sync_record(&record) else {
                    return false;
                };
                if accepted {
                    match &record {
                        DeviceSyncRecord::Reaction { reaction: r } => self
                            .apply_incoming_reaction_to_chat(
                                &r.chat_id,
                                &r.message_id,
                                &r.author,
                                &r.emoji,
                            ),
                        DeviceSyncRecord::GroupSettings { settings: s } => {
                            let chat = format!("group:{}", s.group_id);
                            if let Some(ttl) = s.message_ttl_seconds {
                                self.chat_message_ttl_seconds.insert(chat, ttl);
                            } else {
                                self.chat_message_ttl_seconds.remove(&chat);
                            }
                        }
                        _ => {}
                    }
                    self.persist_best_effort();
                }
                // A newer durable head resolves an older requested record too.
                self.sync_record_is_stored(&record)
            }
        }
    }
}

#[cfg(test)]
mod tests;
