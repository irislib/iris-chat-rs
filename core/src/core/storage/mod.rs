// SQLite-backed durable storage for the core. One database file
// (`core.sqlite3`) per app install lives under `data_dir/`. The
// connection is owned by `AppCore` (or by a one-shot helper for the
// notification-preview path) and shared with `SqliteStorageAdapter`,
// which implements the `nostr_double_ratchet::StorageAdapter` trait.

mod account;
mod connection;
mod schema;
mod store;
mod store_pending_relay;

pub use account::validate_account_storage;
pub(crate) use connection::{open_database, DataDirLock, CORE_DB_FILENAME};
pub(crate) use iris_chat_protocol::{SharedConnection, SqliteStorageAdapter};
pub(crate) use store::{
    load_messages_around_with_visibility, load_messages_before_with_visibility,
    load_recent_messages_with_visibility, search_messages_fts, AppStore, PersistedMessageSearchHit,
    SaveSnapshot,
};

mod store_block_intervals;
#[cfg(test)]
pub(crate) use store::{load_messages_around, load_recent_messages};
pub(crate) use store_block_intervals::blocked_message_intervals;
