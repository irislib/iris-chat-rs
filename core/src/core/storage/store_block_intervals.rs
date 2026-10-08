use nostr::PublicKey;
use rusqlite::{Connection, OptionalExtension};

/// Read the normalized projection committed atomically with every signed
/// transition. No signature scans or event-log replay occur during UI queries.
/// Callers use json_each before pagination/search LIMIT on all platforms.
pub(crate) fn blocked_message_intervals(conn: &Connection, owner: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        PublicKey::from_hex(owner)?.to_hex() == owner,
        "Invalid block account"
    );
    let key = format!("iris-chat-private-block-intervals-v1:{owner}");
    let json: Option<String> = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key=?1 AND length(value)<=16777216",
            [&key],
            |row| row.get(0),
        )
        .optional()?;
    match json {
        Some(json) => Ok(json),
        None => {
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM app_meta WHERE key=?1)",
                [key],
                |row| row.get(0),
            )?;
            anyhow::ensure!(!exists, "Private block interval projection exceeds limit");
            Ok("[]".into())
        }
    }
}
