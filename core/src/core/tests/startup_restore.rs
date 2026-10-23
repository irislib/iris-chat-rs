fn startup_restore_test_connection() -> SharedConnection {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE changes(value INTEGER NOT NULL)")
        .unwrap();
    Arc::new(std::sync::Mutex::new(connection))
}

fn startup_restore_assert_idle(connection: &SharedConnection, expected_rows: i64) {
    let conn = connection.lock().unwrap();
    assert!(
        conn.is_autocommit(),
        "startup must release its transaction before acknowledgement"
    );
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM changes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, expected_rows);
}

#[test]
fn startup_restore_interrupts_real_dml_and_rolls_back_the_whole_transaction() {
    use rusqlite::hooks::Action;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let connection = startup_restore_test_connection();
    let control = Arc::new(startup_restore::StartupRestoreControl::default());
    let guard = control.begin(connection.clone()).unwrap();
    connection
        .lock()
        .unwrap()
        .execute_batch("BEGIN; INSERT INTO changes VALUES(1)")
        .unwrap();
    let callbacks = Arc::new(AtomicUsize::new(0));
    let (cancel, observed) = (control.clone(), callbacks.clone());
    connection
        .lock()
        .unwrap()
        .update_hook(Some(move |action, _: &str, table: &str, _: i64| {
            if action == Action::SQLITE_INSERT && table == "changes" {
                observed.fetch_add(1, Ordering::SeqCst);
                cancel.cancel(cancel.epoch());
            }
        }));
    let result = connection
        .lock()
        .unwrap()
        .execute_batch("INSERT INTO changes VALUES(2),(3),(4)");
    assert!(
        matches!(&result, Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::OperationInterrupted)
    );
    assert!(
        callbacks.load(Ordering::SeqCst) > 0,
        "interrupt must occur after actual DML began"
    );
    let error = guard
        .finish(result.map_err(anyhow::Error::from))
        .unwrap_err();
    assert!(error.is::<startup_restore::StartupRestoreDeferred>());
    connection
        .lock()
        .unwrap()
        .update_hook(None::<fn(Action, &str, &str, i64)>);
    startup_restore_assert_idle(&connection, 0);
}

#[test]
fn startup_restore_between_statements_and_drop_remove_hook_before_rollback() {
    for finish in [false, true] {
        let connection = startup_restore_test_connection();
        let control = Arc::new(startup_restore::StartupRestoreControl::default());
        let guard = control.begin(connection.clone()).unwrap();
        connection
            .lock()
            .unwrap()
            .execute_batch("BEGIN; INSERT INTO changes VALUES(1)")
            .unwrap();
        control.cancel(control.epoch());
        if finish {
            let result =
                connection
                    .lock()
                    .unwrap()
                    .query_row("SELECT count(*) FROM changes", [], |row| {
                        row.get::<_, i64>(0)
                    });
            assert!(
                matches!(&result, Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::OperationInterrupted)
            );
            let error = guard
                .finish(result.map_err(anyhow::Error::from))
                .unwrap_err();
            assert!(error.is::<startup_restore::StartupRestoreDeferred>());
        } else {
            // No next SQLite statement consumes the cancellation. Drop must
            // remove the hook first or its own ROLLBACK would be interrupted.
            drop(guard);
        }
        startup_restore_assert_idle(&connection, 0);
        connection
            .lock()
            .unwrap()
            .execute_batch("BEGIN; INSERT INTO changes VALUES(9); COMMIT")
            .unwrap();
        startup_restore_assert_idle(&connection, 1);
    }
}

#[test]
fn startup_restore_pre_registration_cancel_stale_epochs_and_foreground_are_scoped() {
    let connection = startup_restore_test_connection();
    let control = Arc::new(startup_restore::StartupRestoreControl::default());
    let old_epoch = control.epoch();
    control.cancel(old_epoch);
    let error = control
        .begin(connection.clone())
        .err()
        .expect("paused registration must defer");
    assert!(error.is::<startup_restore::StartupRestoreDeferred>());
    startup_restore_assert_idle(&connection, 0);
    control.foregrounded();
    control.cancel(old_epoch);
    let guard = control.begin(connection.clone()).unwrap();
    connection
        .lock()
        .unwrap()
        .execute_batch("INSERT INTO changes VALUES(1)")
        .unwrap();
    control.cancel(control.epoch());
    control.foregrounded();
    let result = connection
        .lock()
        .unwrap()
        .execute_batch("INSERT INTO changes VALUES(2)");
    assert!(
        result.is_err(),
        "foreground must not revive an already-cancelled scope"
    );
    assert!(guard
        .finish(result.map_err(anyhow::Error::from))
        .unwrap_err()
        .is::<startup_restore::StartupRestoreDeferred>());
    startup_restore_assert_idle(&connection, 1);
    control
        .begin(connection.clone())
        .unwrap()
        .finish(Ok(()))
        .unwrap();
}

#[test]
fn startup_restore_completed_commit_survives_cancel_and_runtime_sql_is_not_interrupted() {
    let connection = startup_restore_test_connection();
    let control = Arc::new(startup_restore::StartupRestoreControl::default());
    let guard = control.begin(connection.clone()).unwrap();
    connection
        .lock()
        .unwrap()
        .execute_batch("BEGIN; INSERT INTO changes VALUES(1); COMMIT")
        .unwrap();
    control.cancel(control.epoch());
    guard.finish(Ok(())).unwrap();
    startup_restore_assert_idle(&connection, 1);
    connection
        .lock()
        .unwrap()
        .execute_batch("BEGIN; INSERT INTO changes VALUES(2)")
        .unwrap();
    control.cancel(control.epoch());
    connection
        .lock()
        .unwrap()
        .execute_batch("INSERT INTO changes VALUES(3); COMMIT")
        .unwrap();
    startup_restore_assert_idle(&connection, 3);
}

#[test]
fn startup_restore_does_not_take_ownership_of_an_existing_transaction() {
    let connection = startup_restore_test_connection();
    let control = Arc::new(startup_restore::StartupRestoreControl::default());
    connection
        .lock()
        .unwrap()
        .execute_batch("BEGIN; INSERT INTO changes VALUES(1)")
        .unwrap();
    assert!(control.begin(connection.clone()).is_err());
    assert!(!connection.lock().unwrap().is_autocommit());
    connection.lock().unwrap().execute_batch("COMMIT").unwrap();
    startup_restore_assert_idle(&connection, 1);
}

#[test]
fn startup_restore_cancellation_does_not_mask_a_genuine_checkpoint_error() {
    let connection = startup_restore_test_connection();
    let control = Arc::new(startup_restore::StartupRestoreControl::default());
    let guard = control.begin(connection.clone()).unwrap();
    let result: anyhow::Result<()> = Err(anyhow::anyhow!("malformed checkpoint"));
    control.cancel(control.epoch());
    let error = guard.finish(result).unwrap_err();
    assert!(!error.is::<startup_restore::StartupRestoreDeferred>());
    assert_eq!(error.to_string(), "malformed checkpoint");
    startup_restore_assert_idle(&connection, 0);
}

#[test]
fn startup_restore_bundle_retries_exact_identity_and_queue_after_repeated_foreground() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _updates, _directory) =
        logged_in_test_core_with_updates("startup-cancel", &owner, &device);
    // Keep the receiver alive without processing follow-up/runtime messages.
    // Otherwise startup falls back to running network follow-up synchronously.
    let (core_sender, _core_receiver) = flume::unbounded();
    core.core_sender = core_sender;
    core.app_store.bind_account(owner.public_key()).unwrap();
    core.preferences.nostr_relay_urls.clear();
    core.device_approval_relay_urls.clear();
    core.persist_best_effort();
    let rumor = EventBuilder::new(
        Kind::from(10451),
        r#"{"type":"startup-cancellation-control","value":1}"#,
    )
    .build(owner.public_key());
    core.protocol_engine
        .as_mut()
        .unwrap()
        .send_local_sibling_unsigned_event(
            owner.public_key(),
            &owner.public_key().to_hex(),
            rumor,
            unix_now(),
        )
        .unwrap();
    let before = saved_sibling_rumors(&core, &owner, &device);
    assert_eq!(before.len(), 1);
    let storage = SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
    );
    let durable = storage.get(TEST_PROTOCOL_ENGINE_STATE_KEY).unwrap();
    let owner_secret = owner.secret_key().to_secret_hex();
    let device_secret = device.secret_key().to_secret_hex();
    let control = core.startup_restore.clone();
    control.cancel(control.epoch());
    core.handle_action(AppAction::RestoreAccountBundle {
        owner_nsec: Some(owner_secret.clone()),
        owner_pubkey_hex: owner.public_key().to_hex(),
        device_nsec: device_secret.clone(),
    });
    let assert_deferred = |core: &AppCore| {
        assert!(core.state.busy.restoring_session);
        assert!(core.state.toast.is_none());
        assert!(core.logged_in.is_none() && core.protocol_engine.is_none());
        assert!(
            matches!(&core.deferred_startup_restore, Some(AppAction::RestoreAccountBundle { owner_nsec: Some(secret), owner_pubkey_hex, device_nsec })
            if secret == &owner_secret && owner_pubkey_hex == &owner.public_key().to_hex() && device_nsec == &device_secret)
        );
        assert!(core.app_store.shared().lock().unwrap().is_autocommit());
    };
    assert_deferred(&core);
    assert_eq!(
        storage.get(TEST_PROTOCOL_ENGINE_STATE_KEY).unwrap(),
        durable
    );

    // The host grants an epoch when enqueueing foreground. If that allowance
    // expires before the actor handles it, the queued action cannot revive it.
    control.foregrounded();
    control.cancel(control.epoch());
    core.handle_action(AppAction::AppForegrounded);
    assert_deferred(&core);
    assert_eq!(
        storage.get(TEST_PROTOCOL_ENGINE_STATE_KEY).unwrap(),
        durable
    );

    let cancelled = Arc::new(AtomicUsize::new(0));
    let (cancel, count) = (control.clone(), cancelled.clone());
    let mut last_cancelled_epoch = None;
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            let epoch = cancel.epoch();
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "ndr_kv",
                    ..
                }
            ) && cancel.is_active_for_test()
                && count.load(Ordering::SeqCst) < 2
                && last_cancelled_epoch != Some(epoch)
            {
                last_cancelled_epoch = Some(epoch);
                count.fetch_add(1, Ordering::SeqCst);
                cancel.cancel(epoch);
            }
            Authorization::Allow
        }));
    for attempt in 1..=2 {
        control.foregrounded();
        core.handle_action(AppAction::AppForegrounded);
        assert_eq!(cancelled.load(Ordering::SeqCst), attempt);
        assert_deferred(&core);
        assert_eq!(
            storage.get(TEST_PROTOCOL_ENGINE_STATE_KEY).unwrap(),
            durable
        );
    }
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    control.foregrounded();
    core.handle_action(AppAction::AppForegrounded);
    assert!(!core.state.busy.restoring_session);
    assert!(core.state.toast.is_none() && core.deferred_startup_restore.is_none());
    let logged_in = core.logged_in.as_ref().unwrap();
    assert_eq!(logged_in.owner_pubkey, owner.public_key());
    assert_eq!(logged_in.device_keys.public_key(), device.public_key());
    assert_eq!(
        logged_in.owner_keys.as_ref().unwrap().public_key(),
        owner.public_key()
    );
    assert!(core.protocol_engine.is_some());
    assert_eq!(saved_sibling_rumors(&core, &owner, &device), before);
    assert!(core.app_store.shared().lock().unwrap().is_autocommit());
    core.shutdown();
}

#[test]
fn startup_restore_ffi_grace_cancels_without_acknowledging_a_busy_database() {
    let directory = tempfile::tempdir().unwrap();
    let app =
        crate::ffi_lifecycle::new_ffi_app_inner(directory.path().to_string_lossy().into_owned());
    let database = app.shared_db_read().as_ref().unwrap().writer.clone();
    let database_work = database.lock().unwrap();
    let control = app.recovery.startup_restore.clone();
    let startup_epoch = control.epoch();
    let (finished_tx, finished_rx) = flume::bounded(1);
    let worker_app = app.clone();
    let worker = std::thread::spawn(move || {
        worker_app.prepare_for_suspend_with_grace(
            true,
            Duration::from_millis(20),
            Some(startup_epoch),
        );
        finished_tx.send(()).unwrap();
    });
    // Observe the real grace-path cancellation on a separate tiny connection;
    // this never creates a second writer on the application's database.
    let probe = startup_restore_test_connection();
    let deadline = Instant::now() + Duration::from_secs(2);
    let cancelled = loop {
        match control.begin(probe.clone()) {
            Err(error) if error.is::<startup_restore::StartupRestoreDeferred>() => break true,
            Err(error) => panic!("unexpected cancellation probe failure: {error}"),
            Ok(guard) => guard.finish(Ok(())).unwrap(),
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let returned_while_busy = finished_rx.try_recv().is_ok();
    drop(database_work);
    let returned_after_drain =
        returned_while_busy || finished_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    worker.join().unwrap();
    assert!(database.lock().unwrap().is_autocommit());
    app.shutdown_and_wait();
    assert!(
        cancelled,
        "the injected grace period must request startup cancellation"
    );
    assert!(
        !returned_while_busy,
        "grace expiration is not a suspension acknowledgement"
    );
    assert!(
        returned_after_drain,
        "suspension must complete once its database barrier drains"
    );
}

#[test]
fn startup_restore_ffi_grace_ignores_an_expired_allowances_stale_epoch() {
    let directory = tempfile::tempdir().unwrap();
    let app =
        crate::ffi_lifecycle::new_ffi_app_inner(directory.path().to_string_lossy().into_owned());
    let database = app.shared_db_read().as_ref().unwrap().writer.clone();
    let database_work = database.lock().unwrap();
    let control = app.recovery.startup_restore.clone();
    let stale_epoch = control.epoch();
    control.foregrounded();
    let (entered_tx, entered_rx) = flume::bounded(1);
    let (finished_tx, finished_rx) = flume::bounded(1);
    let worker_app = app.clone();
    let worker = std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        worker_app.prepare_for_suspend_with_grace(
            true,
            Duration::from_millis(20),
            Some(stale_epoch),
        );
        finished_tx.send(()).unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let returned_while_busy = finished_rx.recv_timeout(Duration::from_millis(100)).is_ok();
    let probe = startup_restore_test_connection();
    let fresh_scope = control.begin(probe.clone());
    let fresh_allowed = fresh_scope.is_ok();
    if let Ok(guard) = fresh_scope {
        guard.finish(Ok(())).unwrap();
    }
    drop(database_work);
    let returned_after_drain =
        returned_while_busy || finished_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    worker.join().unwrap();
    // Also check after the barrier: even a delayed timeout cannot poison the
    // newer foreground epoch after we stop observing the worker.
    let still_allowed = control.begin(probe.clone());
    let allowed_after_drain = still_allowed.is_ok();
    if let Ok(guard) = still_allowed {
        guard.finish(Ok(())).unwrap();
    }
    app.shutdown_and_wait();
    assert!(!returned_while_busy);
    assert!(returned_after_drain);
    assert!(
        fresh_allowed && allowed_after_drain,
        "stale background grace cancelled a newer foreground"
    );
}

#[test]
fn startup_restore_failed_rollback_closes_the_core_before_suspend_returns() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let directory = tempfile::tempdir().unwrap();
    let app =
        crate::ffi_lifecycle::new_ffi_app_inner(directory.path().to_string_lossy().into_owned());
    let database = app.shared_db_read().as_ref().unwrap().writer.clone();
    let released_database = Arc::downgrade(&database);
    database
        .lock()
        .unwrap()
        .execute_batch("CREATE TABLE startup_partial(value INTEGER NOT NULL)")
        .unwrap();
    let control = app.recovery.startup_restore.clone();
    let guard = control.begin(database.clone()).unwrap();
    database
        .lock()
        .unwrap()
        .execute_batch("BEGIN IMMEDIATE; INSERT INTO startup_partial VALUES(1)")
        .unwrap();
    let denied_rollbacks = Arc::new(AtomicUsize::new(0));
    let observed = denied_rollbacks.clone();
    database
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback
                }
            ) {
                observed.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    control.cancel(control.epoch());
    let error = guard
        .finish::<()>(Err(anyhow::anyhow!("injected interrupted restore")))
        .unwrap_err();
    assert!(!error.is::<startup_restore::StartupRestoreDeferred>());
    assert!(denied_rollbacks.load(Ordering::SeqCst) > 0);
    assert!(!control.is_active_for_test());
    {
        let conn = database.lock().unwrap();
        assert!(
            !conn.is_autocommit(),
            "fixture must leave cleanup unable to drain"
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM startup_partial", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
    }
    // Only the actual app retains the SQLite connection during suspension.
    // Its authorizer keeps rejecting rollback; closing it must discard the
    // partial transaction instead of acknowledging a still-locked database.
    drop(database);
    let (finished_tx, finished_rx) = flume::bounded(1);
    let worker_app = app.clone();
    let worker = std::thread::spawn(move || {
        worker_app.prepare_for_suspend_inner(true);
        finished_tx.send(()).unwrap();
    });
    finished_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
    assert!(app.shared_db_read().is_none());
    assert!(app.core_worker.lock().unwrap().is_none());
    assert!(released_database.upgrade().is_none());

    let reopened = rusqlite::Connection::open(directory.path().join("core.sqlite3")).unwrap();
    reopened.busy_timeout(Duration::ZERO).unwrap();
    assert!(reopened.is_autocommit());
    assert_eq!(
        reopened
            .query_row("SELECT count(*) FROM startup_partial", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    reopened
        .execute_batch("BEGIN IMMEDIATE; INSERT INTO startup_partial VALUES(2); COMMIT")
        .unwrap();
    assert!(reopened.is_autocommit());
}
