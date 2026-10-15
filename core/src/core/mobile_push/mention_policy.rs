use super::*;

pub(super) fn keep_group_subscription(preferences: &PreferencesSnapshot, chat_id: &str) -> bool {
    // Encryption hides mentions from the push server. Receive the group's
    // encrypted events and decide locally after decryption, on both platforms.
    is_group_chat_id(chat_id)
        && !preferences
            .silent_mention_chat_ids
            .iter()
            .any(|id| id == chat_id)
}

pub(in crate::core) fn suppresses(
    conn: &rusqlite::Connection,
    chat_id: &str,
    owner: &str,
    body: &str,
    is_message: bool,
) -> bool {
    if !is_chat_muted_in(conn, chat_id) {
        return false;
    }
    if !is_message || !is_group_chat_id(chat_id) || !crate::mentions::mentions_owner(body, owner) {
        return true;
    }
    let disabled = conn
        .query_row(
            "SELECT silent_mention_chat_ids_json FROM preferences WHERE id = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default();
    disabled.iter().any(|id| id == chat_id)
}

pub(super) fn suppresses_in_data_dir(
    data_dir: &str,
    chat_id: &str,
    owner: &str,
    body: &str,
    is_message: bool,
) -> bool {
    open_lookup_connection(data_dir)
        .as_ref()
        .is_some_and(|conn| suppresses(conn, chat_id, owner, body, is_message))
}
