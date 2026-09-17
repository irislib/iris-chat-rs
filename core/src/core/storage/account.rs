use super::AppStore;
use nostr::PublicKey;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

const ACCOUNT_OWNER_KEY: &str = "account_owner_pubkey_hex";
const DIFFERENT_ACCOUNT: &str =
    "This data folder belongs to a different account. Use a separate data folder for this account.";
const MIXED_ACCOUNTS: &str =
    "This data folder contains multiple accounts. Use a separate data folder. Existing chats have been kept.";
const UNKNOWN_ACCOUNT: &str =
    "This data folder's account could not be verified. Use a separate data folder. Existing chats have been kept.";

/// Check account ownership without changing the database or opening a live core.
/// CLI history readers use this while another process may own the session lock.
pub fn validate_account_storage(conn: &Connection, owner_pubkey_hex: &str) -> anyhow::Result<()> {
    let requested_owner = PublicKey::from_hex(owner_pubkey_hex)?;
    let bound_owner: Option<String> = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            [ACCOUNT_OWNER_KEY],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(bound_owner) = bound_owner {
        anyhow::ensure!(
            PublicKey::from_hex(&bound_owner)? == requested_owner,
            DIFFERENT_ACCOUNT
        );
        return Ok(());
    }

    // Before the account marker existed, only these records identify the local
    // account. Peer profiles and incoming authors do not establish ownership.
    // Preserve ambiguous legacy stores instead of adopting or clearing them.
    // Read-only CLI commands may run before the next core startup migrates an
    // old schema. Only include identity columns that existed in that version.
    let mut owner_query = "SELECT owner_pubkey_hex FROM ndr_kv".to_string();
    for (table, column, predicate) in [
        ("messages", "author_owner_pubkey_hex", "is_outgoing = 1"),
        ("pending_relay_publishes", "owner_pubkey_hex", "1"),
        ("user_discovery_state", "owner_pubkey_hex", "1"),
    ] {
        let has_column: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
            [table, column],
            |row| row.get(0),
        )?;
        if has_column {
            owner_query.push_str(&format!(
                " UNION SELECT {column} FROM {table} WHERE {column} IS NOT NULL AND {predicate}"
            ));
        }
    }
    let mut stmt = conn.prepare(&owner_query)?;
    let mut legacy_owner = None;
    for owner in stmt.query_map([], |row| row.get::<_, String>(0))? {
        let owner = PublicKey::from_hex(&owner?).map_err(|_| anyhow::anyhow!(UNKNOWN_ACCOUNT))?;
        anyhow::ensure!(
            legacy_owner.is_none_or(|previous| previous == owner),
            MIXED_ACCOUNTS
        );
        legacy_owner = Some(owner);
    }
    if let Some(legacy_owner) = legacy_owner {
        anyhow::ensure!(legacy_owner == requested_owner, DIFFERENT_ACCOUNT);
    } else {
        let has_history: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM threads)
                 OR EXISTS(SELECT 1 FROM groups)
                 OR EXISTS(SELECT 1 FROM app_keys)
                 OR EXISTS(SELECT 1 FROM owner_profiles)
                 OR EXISTS(SELECT 1 FROM seen_events)",
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(!has_history, UNKNOWN_ACCOUNT);
    }
    Ok(())
}

impl AppStore {
    pub(crate) fn bind_account(&mut self, owner_pubkey: PublicKey) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner_hex = owner_pubkey.to_hex();
        validate_account_storage(&tx, &owner_hex)?;
        tx.execute(
            "INSERT INTO app_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO NOTHING",
            [ACCOUNT_OWNER_KEY, &owner_hex],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::storage::open_database;
    use nostr::Keys;

    fn seed_legacy_owner(store: &AppStore, owner: PublicKey, device: PublicKey) {
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO ndr_kv(owner_pubkey_hex, device_pubkey_hex, key, value)
                 VALUES (?1, ?2, 'legacy-state', '{}')",
                [owner.to_hex(), device.to_hex()],
            )
            .unwrap();
    }

    #[test]
    fn account_storage_binding_survives_reopen_and_is_removed_by_logout() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Keys::generate().public_key();
        let other = Keys::generate().public_key();
        let mut store = AppStore::new(open_database(dir.path()).unwrap());
        store.bind_account(owner).unwrap();
        drop(store);
        let mut store = AppStore::new(open_database(dir.path()).unwrap());
        assert_eq!(
            store.bind_account(other).unwrap_err().to_string(),
            DIFFERENT_ACCOUNT
        );
        store.bind_account(owner).unwrap();
        store.clear().unwrap();
        store.bind_account(other).unwrap();
    }

    #[test]
    fn account_storage_adopts_one_legacy_owner_with_multiple_devices() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Keys::generate().public_key();
        let mut store = AppStore::new(open_database(dir.path()).unwrap());
        seed_legacy_owner(&store, owner, Keys::generate().public_key());
        seed_legacy_owner(&store, owner, Keys::generate().public_key());
        assert_eq!(
            store
                .bind_account(Keys::generate().public_key())
                .unwrap_err()
                .to_string(),
            DIFFERENT_ACCOUNT
        );
        store.bind_account(owner).unwrap();
        assert_eq!(
            store
                .conn
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM ndr_kv", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn account_storage_keeps_ambiguous_legacy_data_unclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Keys::generate().public_key();
        let mut store = AppStore::new(open_database(dir.path()).unwrap());
        seed_legacy_owner(&store, owner, Keys::generate().public_key());
        seed_legacy_owner(
            &store,
            Keys::generate().public_key(),
            Keys::generate().public_key(),
        );
        assert_eq!(
            store.bind_account(owner).unwrap_err().to_string(),
            MIXED_ACCOUNTS
        );
        let conn = store.conn.lock().unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM ndr_kv", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM app_meta WHERE key = ?1",
                [ACCOUNT_OWNER_KEY],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn account_storage_does_not_assign_unattributed_legacy_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AppStore::new(open_database(dir.path()).unwrap());
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO threads(chat_id, draft) VALUES ('peer', 'private draft')",
                [],
            )
            .unwrap();
        assert_eq!(
            store
                .bind_account(Keys::generate().public_key())
                .unwrap_err()
                .to_string(),
            UNKNOWN_ACCOUNT
        );
        assert_eq!(
            store
                .conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT draft FROM threads WHERE chat_id = 'peer'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "private draft"
        );
    }

    #[test]
    fn account_storage_validates_legacy_readers_before_schema_migration() {
        let owner = Keys::generate().public_key();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE ndr_kv(owner_pubkey_hex TEXT NOT NULL);
             CREATE TABLE messages(is_outgoing INTEGER NOT NULL);",
        )
        .unwrap();
        conn.execute("INSERT INTO ndr_kv VALUES (?1)", [owner.to_hex()])
            .unwrap();
        validate_account_storage(&conn, &owner.to_hex()).unwrap();
        assert_eq!(
            validate_account_storage(&conn, &Keys::generate().public_key().to_hex())
                .unwrap_err()
                .to_string(),
            DIFFERENT_ACCOUNT
        );
    }
}
