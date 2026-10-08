use super::*;
use rusqlite::OptionalExtension;

pub(super) fn suppresses_in_data_dir(data_dir: &str, author: &str, created: Option<u64>) -> bool {
    open_lookup_connection(data_dir).is_none_or(|conn| suppresses(&conn, author, created))
}

pub(super) fn suppresses(conn: &rusqlite::Connection, author: &str, created: Option<u64>) -> bool {
    let check = || -> anyhow::Result<bool> {
        let blocked: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM preferences,json_each(blocked_owner_pubkeys_json) WHERE preferences.id=1 AND json_each.value=?1)",[author],|row|row.get(0))?;
        if blocked {
            return Ok(true);
        }
        let owner: Option<String> = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key='account_owner_pubkey_hex'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(owner) = owner else {
            return Ok(false);
        };
        let intervals = super::super::storage::blocked_message_intervals(conn, &owner)?;
        // The signed policy and intervals commit before preferences. An open
        // interval therefore also proves a current block during that crash
        // window, even for a newly delivered old or missing message timestamp.
        Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM json_each(?1) WHERE json_extract(value,'$.author')=?2 AND (json_extract(value,'$.until') IS NULL OR (?3>=json_extract(value,'$.since') AND ?3<json_extract(value,'$.until'))))",rusqlite::params![intervals,author,created],|row|row.get(0))?)
    };
    check().unwrap_or(true)
}
