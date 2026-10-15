//! Match desktop notification metadata against durable private read state.
//! No timestamp cutoff or unread-count change proves another message was seen.

use rusqlite::OptionalExtension;

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DesktopNotificationMessage {
    pub chat_id: String,
    pub message_id: String,
}

pub fn read_desktop_notification_indexes(
    data_dir: String,
    owner_pubkey_hex: String,
    messages: Vec<DesktopNotificationMessage>,
) -> Vec<u64> {
    let Some(conn) = super::mobile_push::open_lookup_connection(&data_dir) else {
        return Vec::new();
    };
    if super::storage::validate_account_storage(&conn, &owner_pubkey_hex).is_err() {
        return Vec::new();
    }
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| message_is_read(&conn, message).then_some(index as u64))
        .collect()
}

fn message_is_read(conn: &rusqlite::Connection, message: &DesktopNotificationMessage) -> bool {
    if message.chat_id.is_empty() || message.message_id.is_empty() {
        return false;
    }
    let stored = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE chat_id = ?1 AND id = ?2
         AND is_outgoing = 0 AND delivery = 'seen')",
            rusqlite::params![message.chat_id, message.message_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);
    if stored {
        return true;
    }
    // A linked-device read can arrive before its matching message. Event IDs
    // bind message contents/timestamps; only an exact chat and ID match counts.
    let pending = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pending_message_reads WHERE chat_id = ?1 AND id = ?2)",
            rusqlite::params![message.chat_id, message.message_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);
    if pending {
        return true;
    }
    let legacy: Option<String> = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            [format!("chat_read_state:{}", message.chat_id)],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    legacy
        .and_then(|json| serde_json::from_str::<super::ChatReadState>(&json).ok())
        .is_some_and(|state| state.exact_messages().contains_key(&message.message_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    #[test]
    fn exact_read_with_unread_siblings_survives_restart_and_stays_account_scoped() {
        let directory = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open(
            directory
                .path()
                .join(super::super::storage::CORE_DB_FILENAME),
        )
        .unwrap();
        conn.execute_batch("CREATE TABLE app_meta(key TEXT PRIMARY KEY, value TEXT);
            CREATE TABLE messages(chat_id TEXT,id TEXT,is_outgoing INTEGER,delivery TEXT);
            CREATE TABLE pending_message_reads(chat_id TEXT,id TEXT,created_at_secs INTEGER);
            INSERT INTO messages VALUES('chat','seen',0,'seen'),('chat','older-unread',0,'received'),
              ('other-chat','same-id',0,'seen'),('chat','outgoing',1,'seen');
            INSERT INTO pending_message_reads VALUES('chat','arrives-later',50);").unwrap();
        conn.execute(
            "INSERT INTO app_meta VALUES('account_owner_pubkey_hex',?1)",
            [OWNER],
        )
        .unwrap();
        conn.execute("INSERT INTO app_meta VALUES('chat_read_state:legacy',?1)", [r#"{"updatedAtMs":1,"deviceId":"own-device","seenThroughSecs":99,"seenAtBoundary":["exact-boundary"],"seenMessages":{}}"#]).unwrap();
        drop(conn);
        let targets = [
            ("chat", "seen"),
            ("chat", "older-unread"),
            ("chat", "same-id"),
            ("chat", "outgoing"),
            ("chat", "arrives-later"),
            ("legacy", "exact-boundary"),
            ("legacy", "older-unlisted"),
        ]
        .map(|(chat_id, message_id)| DesktopNotificationMessage {
            chat_id: chat_id.into(),
            message_id: message_id.into(),
        })
        .to_vec();
        let path = directory.path().to_string_lossy().into_owned();
        assert_eq!(
            read_desktop_notification_indexes(path.clone(), OWNER.into(), targets.clone()),
            vec![0, 4, 5]
        );
        let other_owner = nostr::Keys::generate().public_key().to_hex();
        assert!(read_desktop_notification_indexes(path, other_owner, targets).is_empty());
    }

    #[test]
    fn missing_store_is_not_read_proof() {
        let directory = tempfile::tempdir().unwrap();
        assert!(read_desktop_notification_indexes(
            directory.path().to_string_lossy().into(),
            OWNER.into(),
            vec![DesktopNotificationMessage {
                chat_id: "chat".into(),
                message_id: "message".into()
            }]
        )
        .is_empty());
    }
}
