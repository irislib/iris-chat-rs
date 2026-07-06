use super::super::{PendingRelayPublish, APPCORE_PROTOCOL_LABEL};
use super::store::AppStore;
use nostr_double_ratchet::INVITE_RESPONSE_KIND;
use rusqlite::params;

impl AppStore {
    pub(crate) fn load_pending_relay_publishes(
        &self,
        owner_pubkey_hex: &str,
    ) -> anyhow::Result<Vec<PendingRelayPublish>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT owner_pubkey_hex, event_id, label, event_json, inner_event_id,
                    chat_id, created_at_secs, attempt_count, last_error
             FROM pending_relay_publishes
             WHERE owner_pubkey_hex = ?1
             ORDER BY created_at_secs ASC, event_id ASC",
        )?;
        let rows = stmt.query_map([owner_pubkey_hex], |row| {
            Ok(PendingRelayPublish {
                owner_pubkey_hex: row.get(0)?,
                event_id: row.get(1)?,
                label: row.get(2)?,
                event_json: row.get(3)?,
                inner_event_id: row.get(4)?,
                chat_id: row.get(5)?,
                created_at_secs: row.get::<_, i64>(6)?.max(0) as u64,
                attempt_count: row.get::<_, i64>(7)?.max(0) as u64,
                last_error: row.get(8)?,
            })
        })?;
        let mut pending = Vec::new();
        for row in rows {
            pending.push(row?);
        }
        Ok(pending)
    }

    pub(crate) fn prune_superseded_protocol_invite_response_publishes(
        &self,
        owner_pubkey_hex: &str,
    ) -> anyhow::Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let deleted = conn.execute(
            "WITH ranked AS (
                SELECT event_id,
                       ROW_NUMBER() OVER (
                           PARTITION BY owner_pubkey_hex, label, chat_id,
                                        json_extract(event_json, '$.pubkey')
                           ORDER BY created_at_secs DESC, event_id DESC
                       ) AS rn
                FROM pending_relay_publishes
                WHERE owner_pubkey_hex = ?1
                  AND label = ?2
                  AND inner_event_id IS NULL
                  AND chat_id IS NOT NULL
                  AND CAST(json_extract(event_json, '$.kind') AS INTEGER) = ?3
             )
             DELETE FROM pending_relay_publishes
             WHERE event_id IN (SELECT event_id FROM ranked WHERE rn > 1)",
            params![
                owner_pubkey_hex,
                APPCORE_PROTOCOL_LABEL,
                INVITE_RESPONSE_KIND as i64
            ],
        )?;
        Ok(deleted)
    }

    pub(crate) fn prune_superseded_protocol_control_publishes_for(
        &self,
        owner_pubkey_hex: &str,
        chat_id: &str,
        event_kind: u32,
        event_pubkey_hex: &str,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        let superseded_ids = {
            let mut stmt = tx.prepare(
                "WITH ranked AS (
                    SELECT event_id,
                           ROW_NUMBER() OVER (
                               ORDER BY created_at_secs DESC, event_id DESC
                           ) AS rn
                    FROM pending_relay_publishes
                    WHERE owner_pubkey_hex = ?1
                      AND label = ?2
                      AND inner_event_id IS NULL
                      AND chat_id = ?3
                      AND CAST(json_extract(event_json, '$.kind') AS INTEGER) = ?4
                      AND json_extract(event_json, '$.pubkey') = ?5
                 )
                 SELECT event_id FROM ranked WHERE rn > 1",
            )?;
            let rows = stmt.query_map(
                params![
                    owner_pubkey_hex,
                    APPCORE_PROTOCOL_LABEL,
                    chat_id,
                    event_kind as i64,
                    event_pubkey_hex,
                ],
                |row| row.get::<_, String>(0),
            )?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        if !superseded_ids.is_empty() {
            {
                let mut delete_stmt =
                    tx.prepare("DELETE FROM pending_relay_publishes WHERE event_id = ?1")?;
                for event_id in &superseded_ids {
                    delete_stmt.execute([event_id])?;
                }
            }
        }
        tx.commit()?;
        Ok(superseded_ids)
    }

    pub(crate) fn prune_pending_relay_control_publishes_to_limit(
        &self,
        owner_pubkey_hex: &str,
        max_rows: usize,
    ) -> anyhow::Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let deleted = conn.execute(
            "WITH ranked AS (
                SELECT event_id,
                       ROW_NUMBER() OVER (
                           ORDER BY
                               (label = ?2) ASC,
                               created_at_secs DESC,
                               event_id DESC
                       ) AS rn
                FROM pending_relay_publishes
                WHERE owner_pubkey_hex = ?1
                  AND NOT (inner_event_id IS NOT NULL AND chat_id IS NOT NULL)
             )
             DELETE FROM pending_relay_publishes
             WHERE event_id IN (SELECT event_id FROM ranked WHERE rn > ?3)",
            params![owner_pubkey_hex, APPCORE_PROTOCOL_LABEL, max_rows as i64],
        )?;
        Ok(deleted)
    }

    pub(crate) fn upsert_pending_relay_publish(
        &self,
        pending: &PendingRelayPublish,
    ) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        conn.execute(
            "INSERT INTO pending_relay_publishes(
                event_id, owner_pubkey_hex, label, event_json, inner_event_id,
                chat_id, created_at_secs, attempt_count, last_error
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(event_id) DO UPDATE SET
                owner_pubkey_hex = excluded.owner_pubkey_hex,
                label = excluded.label,
                event_json = excluded.event_json,
                inner_event_id = COALESCE(excluded.inner_event_id, pending_relay_publishes.inner_event_id),
                chat_id = COALESCE(excluded.chat_id, pending_relay_publishes.chat_id),
                created_at_secs = excluded.created_at_secs,
                attempt_count = excluded.attempt_count,
                last_error = excluded.last_error",
            params![
                &pending.event_id,
                &pending.owner_pubkey_hex,
                &pending.label,
                &pending.event_json,
                &pending.inner_event_id,
                &pending.chat_id,
                pending.created_at_secs as i64,
                pending.attempt_count as i64,
                &pending.last_error,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn delete_pending_relay_publish(&self, event_id: &str) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        conn.execute(
            "DELETE FROM pending_relay_publishes WHERE event_id = ?1",
            [event_id],
        )?;
        Ok(())
    }
}
