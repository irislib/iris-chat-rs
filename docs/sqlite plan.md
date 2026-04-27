# SQLite Plan

## Feasibility

SQLite is a good fit for the Rust core. It should work on every native platform this repo currently ships:

- Android: `cargo ndk -t arm64-v8a -P 26` builds the Rust core into the APK. Bundled SQLite avoids depending on the Android framework SQLite version.
- iOS: the core already builds static libraries for `aarch64-apple-ios` and `aarch64-apple-ios-sim`. Bundled SQLite builds with the same clang setup as the rest of the Rust static library.
- macOS: the current arm64 build, and optional x86_64 build, can use the same Rust dependency.
- Linux: the Docker release path can build bundled SQLite without relying on distro SQLite features.
- Windows: the MSVC target can build bundled SQLite.

The caveat is Web/WASM: this project has no browser target today. If a browser build becomes real, it should use a separate storage backend such as SQLite WASM/OPFS or IndexedDB, behind the same storage interface.

Recommended dependency shape:

```toml
[features]
default = ["sqlite-bundled"]
sqlite-bundled = ["rusqlite/bundled"]

[dependencies]
rusqlite = { version = "0.39", default-features = false, features = ["backup", "hooks", "limits", "serde_json"] }
```

Use plain bundled SQLite. Do not add SQLCipher for the first production storage pass.

## Signal Android Takeaways

Useful local reference files in `~/src/signal-android`:

- `app/src/main/java/org/thoughtcrime/securesms/database/SignalDatabase.kt`
- `app/src/main/java/org/thoughtcrime/securesms/database/helpers/SignalDatabaseMigrations.kt`
- `app/src/main/java/org/thoughtcrime/securesms/database/helpers/migration/SignalDatabaseMigration.kt`
- `app/src/main/java/org/thoughtcrime/securesms/database/SqlCipherDatabaseHook.java`
- `app/src/main/java/org/thoughtcrime/securesms/database/SqlCipherErrorHandler.kt`
- `core/util/src/main/java/org/signal/core/util/SQLiteDatabaseExtensions.kt`
- `app/src/main/java/org/thoughtcrime/securesms/database/SearchTable.kt`

Patterns worth copying conceptually:

- One authoritative database owner, with table-specific modules instead of scattered SQL.
- Explicit database version and ordered migrations, even when early development has no user migration burden.
- `PRAGMA foreign_keys = ON` on open.
- WAL checkpoint helpers and integrity checks as normal operational tools.
- Corruption diagnostics that preserve important data and log `integrity_check` output instead of silently deleting the database.
- FTS5 maintained by triggers for message search, with a full reset/rebuild path.
- Transactions around multi-table writes, especially message insertion plus attachments/reactions/thread summary updates.

Patterns not to copy directly:

- Signal's Android `SQLiteOpenHelper` and Java/Kotlin wrappers are platform-specific. The Rust core should own the database directly through `rusqlite`.
- Signal has hundreds of historical migrations. This repo is greenfield, so there is no JSON-to-SQLite importer or compatibility matrix to maintain.

## Signal iOS Takeaways

Useful local reference files in `~/src/Signal-iOS`:

- `Podfile.lock`
- `SignalServiceKit/Storage/Database/GRDBDatabaseStorageAdapter.swift`
- `SignalServiceKit/Storage/Database/GRDBSchemaMigrator.swift`
- `SignalServiceKit/Storage/Database/SDSDatabaseStorage/SDSDatabaseStorage.swift`
- `SignalServiceKit/Storage/Database/SDSDatabaseStorage/V2/DB.swift`
- `SignalServiceKit/Util/SqliteUtil.swift`
- `Signal/Storage/FullTextSearchOptimizer.swift`

Patterns worth copying conceptually:

- GRDB plus SQLCipher is their iOS production stack, so the iOS reference agrees with the Android reference that encrypted SQLite is a mature option. We are intentionally not copying that part for this app.
- Their database key is created and stored in keychain, and database open fails when protected keychain data is unavailable. Skipping SQLCipher avoids that extra notification-service failure mode.
- They separate schema migrations from data migrations. Even greenfield, this is a useful discipline: schema changes should be fast and required; data backfills can be resumable/lazy.
- They reopen the database pool after schema migrations. In Rust we probably have one core-owned connection, but if we add read pools later, stale pooled connections after DDL are a real concern.
- They actively manage WAL growth with truncating checkpoints outside write transactions.
- They expose quick check, FTS integrity/rebuild, and FTS merge/optimize utilities as normal maintenance operations. The cipher integrity check is specific to their SQLCipher setup and is not needed here.
- Extensions and the main app coordinate around database path/changes. Our current notification preview path is read-only, but if future extensions write, cross-process coordination becomes a first-class requirement.

## Greenfield Decision

Treat SQLite as the first durable storage format:

- Do not migrate `data_dir/core/*.json`.
- Do not migrate `data_dir/ndr_runtime/*`.
- It is acceptable to remove or ignore existing local development state when the SQLite store lands.
- Keep native secure stores for secrets, but move ordinary app state, messages, relay event dedupe, and NDR runtime key-value state into SQLite.
- Do not encrypt the SQLite database with SQLCipher. Rely on OS disk encryption, app sandboxing, and keeping app data out of backups/support bundles.

The Rust core remains authoritative. Android, iOS, macOS, Linux, and Windows shells should keep passing `data_dir` and secure restore inputs over UniFFI; they should not query or mutate the SQLite database directly.

## Local Encryption Stance

SQLCipher is intentionally out of scope. A local database key stored on the same device does not protect message history from a live compromised device, which is the main realistic compromise case. It mostly protects copied database files when the key is not copied with them, but it adds build complexity, key-management failure modes, notification-extension edge cases, and performance cost.

This project should instead:

- Keep account/device secrets in Android Keystore, iOS/macOS Keychain, Windows Credential Manager, and equivalent desktop secure stores.
- Exclude app data from cloud/device backups where the platform allows it.
- Keep support bundles redacted and never include message bodies, ratchet state, or raw database files by default.
- Revisit database encryption only if we later need protection against offline file-copy attacks beyond what OS disk encryption and app sandboxing provide.

## Storage Layout

Use one main database file per app install:

```text
data_dir/
  core.sqlite3
  core.sqlite3-wal
  core.sqlite3-shm
  attachments/
  support/
```

Open-time PRAGMAs:

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
PRAGMA temp_store = MEMORY;
PRAGMA secure_delete = ON;
```

Choose `PRAGMA synchronous = NORMAL` or `FULL` deliberately after the build spike. `NORMAL` is usually the mobile WAL default for performance; `FULL` is safer for power-loss durability. Ratchet state and outbox commits should be tested under whichever mode we choose.

## Rust Module Shape

Add a small storage module rather than spreading SQL through `core/src/core/*`:

```text
core/src/storage/
  mod.rs
  connection.rs
  schema.rs
  store.rs
  ndr_storage.rs
  types.rs
```

Responsibilities:

- `connection.rs`: open database, apply PRAGMAs, run quick integrity checks in debug/test builds.
- `schema.rs`: create schema and apply numbered migrations. Since this is greenfield, version 1 can be the complete initial schema.
- `store.rs`: app-level read/write methods used by `AppCore`.
- `ndr_storage.rs`: SQLite-backed implementation of `nostr_double_ratchet::StorageAdapter`.
- `types.rs`: narrow DB row types and enum mapping helpers.

Keep `rusqlite::Connection` behind the core thread or a dedicated storage actor. Avoid sharing a connection across arbitrary async tasks. Most writes should happen inside explicit transactions and complete before emitting UI state.

## Initial Schema

Core metadata:

```sql
CREATE TABLE app_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE preferences (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  send_typing_indicators INTEGER NOT NULL,
  send_read_receipts INTEGER NOT NULL,
  desktop_notifications_enabled INTEGER NOT NULL,
  startup_at_login_enabled INTEGER NOT NULL,
  nostr_relay_urls_json TEXT NOT NULL,
  image_proxy_enabled INTEGER NOT NULL,
  image_proxy_url TEXT NOT NULL,
  image_proxy_key_hex TEXT NOT NULL,
  image_proxy_salt_hex TEXT NOT NULL,
  mobile_push_server_url TEXT NOT NULL
);
```

Accounts and profiles:

```sql
CREATE TABLE owner_profiles (
  owner_pubkey_hex TEXT PRIMARY KEY,
  name TEXT,
  display_name TEXT,
  picture TEXT,
  updated_at_secs INTEGER NOT NULL
);

CREATE TABLE app_key_owners (
  owner_pubkey_hex TEXT PRIMARY KEY,
  created_at_secs INTEGER NOT NULL
);

CREATE TABLE app_key_devices (
  owner_pubkey_hex TEXT NOT NULL REFERENCES app_key_owners(owner_pubkey_hex) ON DELETE CASCADE,
  device_pubkey_hex TEXT NOT NULL,
  created_at_secs INTEGER NOT NULL,
  PRIMARY KEY (owner_pubkey_hex, device_pubkey_hex)
);
```

Chats and groups:

```sql
CREATE TABLE groups (
  group_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  picture TEXT,
  created_at_ms INTEGER NOT NULL,
  updated_at_secs INTEGER NOT NULL,
  group_json TEXT NOT NULL
);

CREATE TABLE group_members (
  group_id TEXT NOT NULL REFERENCES groups(group_id) ON DELETE CASCADE,
  owner_pubkey_hex TEXT NOT NULL,
  is_admin INTEGER NOT NULL,
  is_creator INTEGER NOT NULL,
  PRIMARY KEY (group_id, owner_pubkey_hex)
);

CREATE TABLE threads (
  chat_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('direct', 'group')),
  group_id TEXT REFERENCES groups(group_id) ON DELETE SET NULL,
  unread_count INTEGER NOT NULL DEFAULT 0,
  updated_at_secs INTEGER NOT NULL DEFAULT 0,
  message_ttl_seconds INTEGER,
  active INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX threads_updated_at_idx ON threads(active, updated_at_secs DESC);
CREATE INDEX threads_group_id_idx ON threads(group_id);
```

Messages:

```sql
CREATE TABLE messages (
  message_rowid INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT NOT NULL UNIQUE,
  chat_id TEXT NOT NULL REFERENCES threads(chat_id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('user', 'system')),
  author TEXT NOT NULL,
  body TEXT NOT NULL,
  is_outgoing INTEGER NOT NULL,
  created_at_secs INTEGER NOT NULL,
  expires_at_secs INTEGER,
  delivery TEXT NOT NULL CHECK (delivery IN ('queued', 'pending', 'sent', 'received', 'seen', 'failed')),
  source_event_id TEXT UNIQUE,
  inserted_at_secs INTEGER NOT NULL
);

CREATE INDEX messages_chat_order_idx ON messages(chat_id, created_at_secs, id);
CREATE INDEX messages_expires_idx ON messages(expires_at_secs) WHERE expires_at_secs IS NOT NULL;

CREATE TABLE message_attachments (
  message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  nhash TEXT NOT NULL,
  filename TEXT NOT NULL,
  filename_encoded TEXT NOT NULL,
  htree_url TEXT NOT NULL,
  is_image INTEGER NOT NULL,
  is_video INTEGER NOT NULL,
  is_audio INTEGER NOT NULL,
  PRIMARY KEY (message_id, position)
);

CREATE TABLE message_reactors (
  message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  author TEXT NOT NULL,
  emoji TEXT NOT NULL,
  PRIMARY KEY (message_id, author)
);
```

Relay and protocol state:

```sql
CREATE TABLE seen_events (
  event_id TEXT PRIMARY KEY,
  seen_at_secs INTEGER NOT NULL,
  sequence INTEGER NOT NULL
);

CREATE INDEX seen_events_sequence_idx ON seen_events(sequence);

CREATE TABLE relay_events (
  event_id TEXT PRIMARY KEY,
  kind INTEGER NOT NULL,
  pubkey TEXT NOT NULL,
  created_at_secs INTEGER NOT NULL,
  tags_json TEXT NOT NULL,
  content TEXT NOT NULL,
  first_seen_secs INTEGER NOT NULL,
  processed_at_secs INTEGER
);

CREATE INDEX relay_events_kind_pubkey_idx ON relay_events(kind, pubkey, created_at_secs DESC);

CREATE TABLE ndr_kv (
  owner_pubkey_hex TEXT NOT NULL,
  device_pubkey_hex TEXT NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  updated_at_secs INTEGER NOT NULL,
  PRIMARY KEY (owner_pubkey_hex, device_pubkey_hex, key)
);

CREATE TABLE outbox (
  id TEXT PRIMARY KEY,
  event_json TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending', 'published', 'failed')),
  created_at_secs INTEGER NOT NULL,
  updated_at_secs INTEGER NOT NULL,
  retry_count INTEGER NOT NULL DEFAULT 0,
  last_error TEXT
);
```

Message search can start after the base store is stable:

```sql
CREATE VIRTUAL TABLE message_fts USING fts5(
  body,
  chat_id UNINDEXED,
  content='messages',
  content_rowid='message_rowid'
);
```

FTS should be maintained by insert/update/delete triggers or by explicit rebuild jobs. Follow Signal's pattern of having a full reset/rebuild path.

## Code Change Plan

1. Build spike.
   Add `rusqlite` behind the feature layout above. Verify `cargo test`, Android `buildRustAndroidDebug`, iOS `ios-rust`, macOS `macos-rust`, Linux release Docker, and Windows `windows-rust`.

2. Add storage module.
   Implement opening, PRAGMAs, schema versioning, transaction helper, enum conversions, and integrity-check helpers. Add unit tests that use real tempdir SQLite files.

3. Replace app JSON persistence.
   Remove `PersistenceCache` and the JSON slice writer. Add `AppStore::load_state` and `AppStore::save_*` methods that update SQLite in transactions. Because there are no users, do not write an importer.

4. Replace NDR file storage.
   Implement `StorageAdapter` over `ndr_kv`. Preserve the notification preview overlay behavior: notification decrypts must read base state and put all writes into an in-memory overlay, leaving `ndr_kv` unchanged.

5. Make writes event-shaped.
   When processing relay events, store raw event/dedupe rows and derived message/thread rows in the same transaction. When sending messages, store the local message and outbox row together.

6. Rebuild projections from SQLite.
   Keep `AppState` in memory for fast UI emission, but make restart load authoritative state from SQLite. The chat list should come from `threads` plus latest message queries; current chat should page from `messages`.

7. Add search.
   Add FTS5 only after base persistence is stable. Include rebuild and repair helpers.

8. Delete old persistence paths.
   Remove JSON-specific helpers and tests. Keep support bundle/debug snapshot generation if it is still useful, but generate it from live state/SQLite.

## Test Plan

Use real SQLite files in tempdirs, not mocks.

- Schema opens on a fresh database and sets `user_version`.
- Reopening the same database is idempotent.
- Foreign keys are enabled and violations fail.
- Message insert writes thread, message, attachments, reactors, and seen-event rows atomically.
- Restart test: create/restore account, create direct chat, create group chat, insert messages, drop core, recreate core, restore bundle, assert `AppState` matches expected chat list/current chat.
- Notification preview test: decrypt a pushed event through the read-only/overlay path and assert `ndr_kv` bytes/rows are unchanged.
- Outbox test: queued local event survives restart and can be marked published.
- Expiring message test: expiry query removes messages and updates thread summaries.
- Integrity test: `PRAGMA quick_check` returns `ok` for normal DBs.

Native confidence lanes:

- `just qa-native-contract`
- Android `AppManagerContractTest`
- Android real-relay harness after storage replacement
- iOS XCTest restore/restart coverage
- Windows `just windows-rust` after dependency landing

## Operational Hardening

- Add a support-bundle section with schema version, page count, WAL checkpoint status, table row counts, `quick_check`, and recent storage errors. Do not include message bodies or secrets.
- Add `VACUUM` only as an explicit maintenance command after large deletes, never on hot paths.
- Add `wal_checkpoint(TRUNCATE)` on logout/reset and before support/export operations that need a compact file.
- Add a lightweight checkpoint budget after writes so the WAL cannot grow indefinitely during heavy relay catch-up. Run truncating checkpoints outside transactions.
- Keep database reset simple in development: close core, delete `core.sqlite3*`, recreate.
- Make corruption handling conservative: log diagnostics and surface a reset/recover path. Do not silently delete production chat data.

## Definition Of Done

- Rust core no longer reads or writes `core/*.json` for durable app state.
- NDR runtime no longer writes its own per-key files.
- A clean install creates `core.sqlite3` and boots on Android, iOS, macOS, Linux, and Windows.
- Restart restores chats, groups, preferences, seen-event dedupe, and ratchet state from SQLite.
- Notification preview decrypt remains read-only with respect to ratchet state.
- Tests cover real restart behavior and transaction atomicity.
- Plain SQLite is the explicit storage decision; SQLCipher is not part of the build.
