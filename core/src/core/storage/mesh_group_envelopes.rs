//! A bounded peer-readable cache, separate from decrypted message/identity state.
use super::{AppStore, SharedConnection};
use crate::core::{identity::unix_now, MESSAGE_EVENT_KIND};
use nostr::{Event, Filter, JsonUtil};
use nostr_pubsub::{
    EventBus, EventSource, PublishReport, QueryEvent, QueryOptions, QueryReport, VerifiedEvent,
};
use rusqlite::params;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::TryLockError;
use std::time::Duration;

// Match the existing nearby outbox window. The wire byte cap also bounds disk
// retention to less than 16 MiB of event bodies, independent of attachment size.
pub(in crate::core) const MAX_ENVELOPES: usize =
    crate::core::fips_nearby::FIPS_NEARBY_OUTBOX_MAX_EVENTS;
const MAX_EVENT_BYTES: usize = nostr_pubsub_fips::FIPS_NOSTR_PUBSUB_MAX_FRAME_BYTES;
const MAX_QUERY_ROWS: usize = 256;
const MAX_QUERY_FILTERS: usize = 64;
const STORAGE_WAIT: Duration = Duration::from_millis(100);
const STORAGE_RETRY: Duration = Duration::from_millis(5);

/// Current accepted groups and their permitted authors, computed using the
/// production request/membership/block policy. Replacing the replay source on
/// subscription reconciliation revokes in-flight queries in the FIPS library.
pub(in crate::core) type AllowedGroups = BTreeMap<String, BTreeSet<String>>;

pub(in crate::core) struct GroupEnvelopeReplay {
    conn: SharedConnection,
    owner: String,
    groups: AllowedGroups,
}

impl GroupEnvelopeReplay {
    pub(in crate::core) fn same_permissions(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.conn, &other.conn)
            && self.owner == other.owner
            && self.groups == other.groups
    }

    pub(in crate::core) fn new(
        conn: SharedConnection,
        owner: String,
        groups: AllowedGroups,
    ) -> Self {
        Self {
            conn,
            owner,
            groups,
        }
    }

    /// `None` means the foreground is using storage. Keep acquisition inside a
    /// synchronous helper so no connection guard can cross an async yield.
    fn try_read(&self, filters: &[Filter], limit: usize) -> anyhow::Result<Option<QueryReport>> {
        if limit == 0 || filters.is_empty() || filters.len() > MAX_QUERY_FILTERS {
            return Ok(Some(QueryReport::default()));
        }
        // A protocol subscription names ratchet authors; WANT names an event.
        // A wildcard must not turn this local store into a general event dump.
        let ids: BTreeSet<_> = filters
            .iter()
            .flat_map(|f| f.ids.iter().flatten())
            .map(|id| id.to_hex())
            .collect();
        let authors: BTreeSet<_> = filters
            .iter()
            .flat_map(|f| f.authors.iter().flatten())
            .map(|key| key.to_hex())
            .collect();
        if (ids.is_empty() && authors.is_empty()) || ids.len() + authors.len() > MAX_QUERY_ROWS {
            return Ok(Some(QueryReport::default()));
        }
        let rows = {
            // Reuse the foreground connection without blocking the replay
            // worker behind it. A separate SQLite reader would race mobile
            // rollback-journal writes and would need another busy policy.
            let conn = match self.conn.try_lock() {
                Ok(conn) => conn,
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Poisoned(_)) => {
                    anyhow::bail!("group replay storage poisoned")
                }
            };
            super::validate_account_storage(&conn, &self.owner)?;
            let blocked = super::blocked_message_intervals(&conn, &self.owner)?;
            let mut stmt = conn.prepare_cached(
                "SELECT e.event_id, e.chat_id, m.author_owner_pubkey_hex, e.event_json
                 FROM mesh_group_envelopes e JOIN messages m
                   ON m.chat_id=e.chat_id AND m.id=e.message_id AND m.source_event_id=e.event_id
                 JOIN groups g ON m.chat_id='group:' || g.group_id
                 WHERE e.owner_pubkey_hex=?1 AND m.kind='user' AND m.deleted_for_everyone=0
                   AND json_array_length(m.edit_history_json)=0
                   AND (m.expires_at_secs IS NULL OR m.expires_at_secs>?2)
                   AND NOT EXISTS (SELECT 1 FROM json_each(?6) AS blocked
                       WHERE json_extract(blocked.value, '$.author')=m.author_owner_pubkey_hex
                         AND m.created_at_secs>=json_extract(blocked.value, '$.since')
                         AND (json_extract(blocked.value, '$.until') IS NULL
                              OR m.created_at_secs<json_extract(blocked.value, '$.until')))
                   AND (e.event_id IN (SELECT value FROM json_each(?3))
                        OR e.event_author IN (SELECT value FROM json_each(?4)))
                 ORDER BY e.sequence DESC LIMIT ?5",
            )?;
            let mapped = stmt.query_map(
                params![
                    self.owner,
                    unix_now().get() as i64,
                    serde_json::to_string(&ids)?,
                    serde_json::to_string(&authors)?,
                    MAX_QUERY_ROWS as i64,
                    blocked
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )?;
            mapped.collect::<Result<Vec<_>, _>>()?
        };
        let mut events = Vec::new();
        for (id, chat, sender, json) in rows {
            if json.len() > MAX_EVENT_BYTES
                || !self.groups.get(&chat).is_some_and(|members| {
                    sender
                        .as_ref()
                        .is_some_and(|sender| members.contains(sender))
                })
            {
                continue;
            }
            let Ok(event) = serde_json::from_str::<Event>(&json) else {
                continue;
            };
            if event.id.to_hex() != id
                || event.kind.as_u16() as u32 != MESSAGE_EVENT_KIND
                || !filters.iter().any(|filter| {
                    filter.match_event(&event, nostr::filter::MatchEventOptions::default())
                })
            {
                continue;
            }
            // Revalidate persisted bytes, including signature and ID. A damaged
            // cache is never a trusted source of wire events.
            let Ok(event) = VerifiedEvent::try_from(event) else {
                continue;
            };
            events.push(QueryEvent {
                event,
                source: EventSource::local_index("iris-chat-group-ciphertext"),
                priority: 0,
            });
            if events.len() == limit {
                break;
            }
        }
        Ok(Some(QueryReport { events }))
    }
}

#[async_trait::async_trait]
impl EventBus for GroupEnvelopeReplay {
    async fn publish(
        &self,
        _: VerifiedEvent,
        _: EventSource,
    ) -> nostr_pubsub::Result<PublishReport> {
        // Only the authenticated application ingestion path may populate it.
        Ok(PublishReport {
            accepted: false,
            priority: 0,
            reason: Some("read-only group replay source".into()),
        })
    }

    async fn query(
        &self,
        filters: Vec<Filter>,
        options: QueryOptions,
    ) -> nostr_pubsub::Result<QueryReport> {
        let limit = options
            .limit
            .unwrap_or(crate::core::mesh_pubsub::MESH_REPLAY_EVENTS)
            .min(crate::core::mesh_pubsub::MESH_REPLAY_EVENTS);
        let deadline = tokio::time::Instant::now() + STORAGE_WAIT;
        loop {
            match self.try_read(&filters, limit) {
                Ok(Some(report)) => return Ok(report),
                Ok(None) => {}
                Err(error) => return Err(nostr_pubsub::PubsubError::Storage(error.to_string())),
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(nostr_pubsub::PubsubError::Storage(
                    "group replay storage busy".into(),
                ));
            }
            // Cancellation and the transport's query timeout remain effective
            // even when foreground persistence holds the shared connection.
            tokio::time::sleep_until((now + STORAGE_RETRY).min(deadline)).await;
        }
    }
}

impl AppStore {
    /// Called only after an authenticated group ciphertext produced a permitted
    /// user message. The row join keeps deferred/failed app persistence private.
    pub(in crate::core) fn retain_group_envelope(
        &self,
        owner: &str,
        chat: &str,
        message: &str,
        event: &Event,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            chat.starts_with("group:") && event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND,
            "only authenticated group message envelopes may be cached"
        );
        VerifiedEvent::try_from(event.clone())?;
        let json = event.as_json();
        if json.len() > MAX_EVENT_BYTES {
            return Ok(());
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("group replay storage poisoned"))?;
        let tx = conn.transaction()?;
        tx.execute("INSERT OR IGNORE INTO mesh_group_envelopes
            (owner_pubkey_hex,event_id,event_author,chat_id,message_id,event_json) VALUES (?1,?2,?3,?4,?5,?6)",
            params![owner,event.id.to_hex(),event.pubkey.to_hex(),chat,message,json])?;
        tx.execute(
            "DELETE FROM mesh_group_envelopes WHERE sequence IN
            (SELECT sequence FROM mesh_group_envelopes ORDER BY sequence DESC LIMIT -1 OFFSET ?1)",
            [MAX_ENVELOPES as i64],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
