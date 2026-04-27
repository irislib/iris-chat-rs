// SQLite-backed durable storage for the core. One database file
// (`core.sqlite3`) per app install lives under `data_dir/`. The
// connection is owned by `AppCore` (or by a one-shot helper for the
// notification-preview path) and shared with `SqliteStorageAdapter`,
// which implements the `nostr_double_ratchet::StorageAdapter` trait.
//
// Greenfield decision: there is no migration from the previous JSON
// layout — the new database starts empty on a clean install and the
// JSON files (if present) are ignored.

mod connection;
mod ndr_storage;
mod schema;
mod store;

pub(crate) use connection::{open_database, CORE_DB_FILENAME};
pub(crate) use ndr_storage::SqliteStorageAdapter;
pub(crate) use store::{AppStore, SaveSnapshot};

use std::sync::{Arc, Mutex};

pub(crate) type SharedConnection = Arc<Mutex<rusqlite::Connection>>;
