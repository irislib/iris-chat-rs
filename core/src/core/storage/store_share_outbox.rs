//! Durable local share submissions. Acceptance and transport delivery are
//! separate: a recipient is acknowledged only in the transaction that stores
//! its authored message. Retries retain that event's exact ID and ciphertext
//! preparation must continue through the normal protocol outbox.

use super::*;
use nostr::{PublicKey, UnsignedEvent};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ShareAttachment {
    // Relative to the request's private staging directory, never an arbitrary
    // file supplied by an application sharing a URL or attachment with Iris.
    pub path: String,
    pub filename: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ShareDraft {
    pub id: String,
    pub account_owner_pubkey_hex: String,
    pub chat_ids: Vec<String>,
    pub text: String,
    pub attachments: Vec<ShareAttachment>,
}

pub(crate) struct ShareRecipient {
    pub chat_id: String,
    pub message_id: Option<String>,
    pub event: Option<UnsignedEvent>,
    pub queued: bool,
}

pub(crate) struct ShareSubmission {
    // Completed requests retain only the fingerprint and per-recipient receipt,
    // so reopening an old share cannot send it again or retain a second body.
    pub draft: Option<ShareDraft>,
    pub prepared_text: Option<String>,
    pub recipients: Vec<ShareRecipient>,
}

impl ShareDraft {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.id.is_empty()
                && self.id.len() <= 128
                && self
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "Invalid share request"
        );
        anyhow::ensure!(
            PublicKey::from_hex(&self.account_owner_pubkey_hex)?.to_hex()
                == self.account_owner_pubkey_hex,
            "Invalid share account"
        );
        let unique: HashSet<_> = self.chat_ids.iter().collect();
        anyhow::ensure!(
            !self.chat_ids.is_empty()
                && self.chat_ids.len() <= 20
                && unique.len() == self.chat_ids.len()
                && self
                    .chat_ids
                    .iter()
                    .all(|id| !id.is_empty() && id.len() <= 512 && id.trim() == id),
            "Invalid share recipients"
        );
        anyhow::ensure!(
            self.text.len() <= 32 * 1024
                && self.attachments.len() <= 32
                && (!self.text.trim().is_empty() || !self.attachments.is_empty()),
            "Invalid share content"
        );
        for attachment in &self.attachments {
            let path = std::path::Path::new(&attachment.path);
            anyhow::ensure!(
                !attachment.path.is_empty()
                    && !attachment.path.contains('\0')
                    && attachment.path.len() <= 1024
                    && path
                        .components()
                        .all(|part| matches!(part, std::path::Component::Normal(_)))
                    && !attachment.filename.is_empty()
                    && !attachment.filename.contains('\0')
                    && attachment.filename.len() <= 1024,
                "Invalid shared attachment"
            );
        }
        Ok(())
    }
}

fn require_account(conn: &rusqlite::Connection, owner: &str) -> anyhow::Result<()> {
    // An empty database must not be adopted merely because an old extension
    // payload names an account. Only an authenticated core binds this marker.
    let bound: Option<String> = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'account_owner_pubkey_hex'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    anyhow::ensure!(
        bound.as_deref() == Some(owner),
        "Share belongs to another account"
    );
    Ok(())
}

impl AppStore {
    pub(crate) fn accept_share(&mut self, draft: &ShareDraft) -> anyhow::Result<()> {
        draft.validate()?;
        let json = serde_json::to_string(draft)?;
        let fingerprint = Sha256::digest(json.as_bytes()).to_vec();
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_account(&tx, &draft.account_owner_pubkey_hex)?;
        let existing: Option<(String, Vec<u8>)> = tx
            .query_row(
                "SELECT account_owner_pubkey_hex, fingerprint FROM share_requests WHERE id = ?1",
                [&draft.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((owner, previous)) = existing {
            anyhow::ensure!(
                owner == draft.account_owner_pubkey_hex && previous == fingerprint,
                "Share request changed after submission"
            );
            return Ok(());
        }
        let prepared = draft.attachments.is_empty().then(|| draft.text.trim());
        tx.execute("INSERT INTO share_requests(id, account_owner_pubkey_hex, fingerprint, payload_json, prepared_text)
            VALUES (?1, ?2, ?3, ?4, ?5)",
            params![draft.id, draft.account_owner_pubkey_hex, fingerprint, json, prepared])?;
        for (position, chat) in draft.chat_ids.iter().enumerate() {
            tx.execute(
                "INSERT INTO share_recipients(share_id, chat_id, position) VALUES (?1, ?2, ?3)",
                params![draft.id, chat, position as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn shared_submission(
        &self,
        id: &str,
        owner: &str,
    ) -> anyhow::Result<ShareSubmission> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        require_account(&conn, owner)?;
        let (json, prepared_text): (Option<String>, Option<String>) = conn.query_row(
            "SELECT payload_json, prepared_text FROM share_requests WHERE id = ?1 AND account_owner_pubkey_hex = ?2",
            params![id, owner], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut statement = conn.prepare(
            "SELECT chat_id, message_id, event_json, queued FROM share_recipients
            WHERE share_id = ?1 ORDER BY position",
        )?;
        let rows = statement.query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })?;
        let mut recipients = Vec::new();
        for row in rows {
            let (chat_id, message_id, event, queued) = row?;
            recipients.push(ShareRecipient {
                chat_id,
                message_id,
                queued,
                event: event.map(|json| serde_json::from_str(&json)).transpose()?,
            });
        }
        Ok(ShareSubmission {
            draft: json.map(|json| serde_json::from_str(&json)).transpose()?,
            prepared_text,
            recipients,
        })
    }

    /// Preserve prepared blob references before authoring any recipient event.
    /// A retry reuses this text instead of re-uploading or changing encryption
    /// keys. The ordinary attachment store owns the referenced durable pins.
    pub(crate) fn prepare_shared_text(
        &mut self,
        id: &str,
        owner: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !text.trim().is_empty() && text.len() <= 32 * 1024,
            "Invalid shared message"
        );
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_account(&tx, owner)?;
        let (payload, previous): (Option<String>, Option<String>) = tx.query_row(
            "SELECT payload_json, prepared_text FROM share_requests WHERE id = ?1 AND account_owner_pubkey_hex = ?2",
            params![id, owner], |row| Ok((row.get(0)?, row.get(1)?)))?;
        anyhow::ensure!(payload.is_some(), "Share is already queued");
        anyhow::ensure!(
            previous.as_deref().is_none_or(|previous| previous == text),
            "Share content already prepared"
        );
        tx.execute(
            "UPDATE share_requests SET prepared_text = ?2 WHERE id = ?1",
            params![id, text],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Persist the exact authored event before encryption/publication. This
    /// method cannot replace an event or add a destination after user approval.
    pub(crate) fn prepare_shared_event(
        &mut self,
        id: &str,
        chat: &str,
        event: &UnsignedEvent,
    ) -> anyhow::Result<()> {
        event.verify_id()?;
        anyhow::ensure!(
            event.kind.as_u16() as u32 == nostr_double_ratchet::CHAT_MESSAGE_KIND,
            "Shared event is not a message"
        );
        let message_id = event
            .id
            .ok_or_else(|| anyhow::anyhow!("Shared event has no ID"))?
            .to_hex();
        let event_json = serde_json::to_string(event)?;
        let owner = event.pubkey.to_hex();
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_account(&tx, &owner)?;
        let text: Option<String> = tx.query_row(
            "SELECT prepared_text FROM share_requests
            WHERE id = ?1 AND account_owner_pubkey_hex = ?2",
            params![id, owner],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            text.as_deref() == Some(event.content.as_str()),
            "Share content is not prepared"
        );
        let (previous_id, previous_json, queued): (Option<String>, Option<String>, bool) = tx.query_row(
            "SELECT message_id, event_json, queued FROM share_recipients WHERE share_id = ?1 AND chat_id = ?2",
            params![id, chat], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        anyhow::ensure!(!queued, "Share recipient is already queued");
        if let Some(previous_id) = previous_id {
            anyhow::ensure!(
                previous_id == message_id && previous_json.as_deref() == Some(&event_json),
                "Share recipient already has an authored event"
            );
            return Ok(());
        }
        tx.execute(
            "UPDATE share_recipients SET message_id = ?3, event_json = ?4
            WHERE share_id = ?1 AND chat_id = ?2",
            params![id, chat, message_id, event_json],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The receipt and visible/core-retry message are one transaction. The
    /// extension may remove its staging payload only after all receipts exist.
    pub(crate) fn save_shared_outgoing_event(
        &mut self,
        share: &str,
        thread: &ThreadRecord,
        message: &ChatMessageSnapshot,
        event: &UnsignedEvent,
    ) -> anyhow::Result<bool> {
        event.verify_id()?;
        anyhow::ensure!(
            event.kind.as_u16() as u32 == nostr_double_ratchet::CHAT_MESSAGE_KIND,
            "Shared event is not a message"
        );
        let event_json = serde_json::to_string(event)?;
        let owner = event.pubkey.to_hex();
        anyhow::ensure!(
            message.is_outgoing
                && message.chat_id == thread.chat_id
                && message.author_owner_pubkey_hex.as_deref() == Some(owner.as_str())
                && message.created_at_secs == event.created_at.as_secs()
                && message.body == event.content
                && event.id.is_some_and(|id| id.to_hex() == message.id),
            "Invalid shared message"
        );
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_account(&tx, &owner)?;
        let (stored_owner, stored_id, stored_event, queued): (String, Option<String>, Option<String>, bool) = tx.query_row(
            "SELECT request.account_owner_pubkey_hex, recipient.message_id, recipient.event_json, recipient.queued
            FROM share_requests request JOIN share_recipients recipient ON request.id = recipient.share_id
            WHERE request.id = ?1 AND recipient.chat_id = ?2", params![share, thread.chat_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;
        anyhow::ensure!(
            stored_owner == owner && stored_id.as_deref() == Some(&message.id),
            "Share event changed"
        );
        if queued {
            return Ok(false);
        }
        anyhow::ensure!(
            stored_event.as_deref() == Some(&event_json),
            "Share event was not prepared"
        );
        super::store_outgoing_events::write_outgoing_event(&tx, thread, message, &event_json)?;
        let persisted: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE chat_id = ?1 AND id = ?2)",
            params![thread.chat_id, message.id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(persisted, "Shared message was deleted before it was queued");
        tx.execute("UPDATE share_recipients SET queued = 1, event_json = NULL WHERE share_id = ?1 AND chat_id = ?2",
            params![share, thread.chat_id])?;
        tx.execute(
            "UPDATE share_requests SET payload_json = NULL, prepared_text = NULL WHERE id = ?1
            AND NOT EXISTS(SELECT 1 FROM share_recipients WHERE share_id = ?1 AND queued = 0)",
            [share],
        )?;
        tx.commit()?;
        self.cache.threads.remove(&thread.chat_id);
        Ok(true)
    }
}

#[cfg(test)]
#[path = "store_share_outbox_tests.rs"]
mod tests;
