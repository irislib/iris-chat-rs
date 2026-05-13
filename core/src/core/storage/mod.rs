// SQLite-backed durable storage for the core. One database file
// (`core.sqlite3`) per app install lives under `data_dir/`. The
// connection is owned by `AppCore` (or by a one-shot helper for the
// notification-preview path) and shared with `SqliteStorageAdapter`,
// which implements the `nostr_double_ratchet::StorageAdapter` trait.

mod connection;
mod ndr_storage;
mod schema;
mod store;

pub(crate) use connection::{open_database, DataDirLock, CORE_DB_FILENAME};
pub(crate) use ndr_storage::SqliteStorageAdapter;
pub(crate) use store::{
    load_messages_around, load_messages_before, load_recent_messages, search_messages_fts,
    AppStore, PersistedMessageSearchHit, SaveSnapshot,
};

use std::sync::{Arc, Mutex};

pub(crate) type SharedConnection = Arc<Mutex<rusqlite::Connection>>;
