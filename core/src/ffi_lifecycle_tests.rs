use super::*;

#[test]
fn support_bundle_replies_through_the_core_with_explicit_unavailable_traffic() {
    let temp = tempfile::TempDir::new().unwrap();
    let app = new_ffi_app_inner(temp.path().to_string_lossy().to_string());
    let bundle: serde_json::Value =
        serde_json::from_str(&app.export_support_bundle_json()).unwrap();
    assert_eq!(bundle["ffi_queue"]["core_support_bundle_timed_out"], false);
    assert_eq!(bundle["fips_transport"]["valid"], false);
    assert_eq!(bundle["fips_transport"]["status"], "unavailable");
    assert!(bundle["fips_transport"]["connected_peer_count"].is_null());
    assert!(bundle["relay_transport"].is_object());
    app.shutdown_and_wait();
}

#[test]
fn shutdown_drains_a_direct_search_waiting_for_the_database() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let app = new_ffi_app_inner(temp_dir.path().to_string_lossy().to_string());
    let db = app.shared_db_read().as_ref().unwrap().writer.clone();
    let database_work = db.lock().unwrap();
    let reader_app = app.clone();
    let reader =
        thread::spawn(move || reader_app.search("message".into(), Some("chat".into()), 20));
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let reader_holds_slot = loop {
        if app.shared_db.try_write().is_err() {
            break true;
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(1));
    };
    let worker_app = app.clone();
    let (finished_tx, finished_rx) = flume::bounded(1);
    let worker = thread::spawn(move || {
        worker_app.shutdown_and_wait();
        finished_tx.send(()).unwrap();
    });
    let returned_while_reading = finished_rx.recv_timeout(Duration::from_millis(100)).is_ok();
    drop(database_work);
    reader.join().unwrap();
    drop(db);
    worker.join().unwrap();
    assert!(
        reader_holds_slot,
        "direct search released the database slot before its query finished"
    );
    assert!(
        !returned_while_reading,
        "shutdown returned while a direct reader was using the database"
    );
    assert!(app.shared_db_read().is_none());
}

#[test]
fn shutdown_and_wait_releases_core_before_terminal_cleanup() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let app = new_ffi_app_inner(data_dir.clone());
    let db = app.shared_db_read().as_ref().unwrap().writer.clone();
    let database_work = db.lock().unwrap();
    let (suspend_tx, _suspend_rx) = flume::bounded(1);
    app.foreground_tx
        .send(CoreMsg::PrepareForSuspend(suspend_tx))
        .unwrap();
    let (finished_tx, finished_rx) = flume::bounded(1);
    let worker_app = app.clone();
    let worker = thread::spawn(move || {
        worker_app.shutdown_and_wait();
        finished_tx.send(()).unwrap();
    });
    let returned_while_busy = finished_rx
        .recv_timeout(Duration::from_millis(2300))
        .is_ok();
    drop(database_work);
    drop(db);
    worker.join().unwrap();
    assert!(
        !returned_while_busy,
        "shutdown returned before core work finished"
    );
    assert!(app.shared_db_read().is_none());
    // No sleep or retry: shutdown must release the directory lock before
    // a host deletes files and creates the replacement core.
    let replacement = new_ffi_app_inner(data_dir);
    assert!(replacement.shared_db_read().is_some());
    replacement.shutdown_and_wait();
    app.shutdown_and_wait();
}

#[test]
fn shutdown_returns_when_startup_failed_without_a_core_worker() {
    let app = ffi_app_failure("startup failed".into());
    let (finished_tx, finished_rx) = flume::bounded(1);
    thread::spawn(move || {
        app.shutdown_and_wait();
        let _ = finished_tx.send(());
    });
    assert!(finished_rx.recv_timeout(Duration::from_secs(1)).is_ok());
}

#[test]
fn suspend_waits_for_database_work_before_releasing_ios_background_time() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let app = new_ffi_app_inner(temp_dir.path().to_string_lossy().to_string());
    let db = app.shared_db_read().as_ref().unwrap().writer.clone();
    let database_work = db.lock().unwrap();
    let (finished_tx, finished_rx) = flume::bounded(1);
    let worker_app = app.clone();
    let worker = thread::spawn(move || {
        worker_app.prepare_for_suspend_inner(true);
        finished_tx.send(()).unwrap();
    });
    // The previous two-second FFI timeout reported completion even while
    // the core was still waiting for its database lock.
    let returned_while_busy = finished_rx
        .recv_timeout(Duration::from_millis(2300))
        .is_ok();
    drop(database_work);
    worker.join().unwrap();
    app.shutdown();
    assert!(
        !returned_while_busy,
        "suspend returned before database work finished"
    );
}

#[test]
fn suspend_returns_when_startup_failed_without_a_core_worker() {
    let app = ffi_app_failure("startup failed".into());
    let (finished_tx, finished_rx) = flume::bounded(1);
    thread::spawn(move || {
        app.prepare_for_suspend_inner(true);
        let _ = finished_tx.send(());
    });
    assert!(finished_rx.recv_timeout(Duration::from_secs(1)).is_ok());
}

#[test]
fn suspension_queued_after_shutdown_waits_for_worker_close_not_its_retained_reply() {
    fn wait_until(condition: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !condition() {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
        true
    }

    let directory = tempfile::tempdir().unwrap();
    let app = new_ffi_app_inner(directory.path().to_string_lossy().into_owned());
    let database = app.shared_db_read().as_ref().unwrap().writer.clone();
    let released_database = Arc::downgrade(&database);
    let database_work = database.lock().unwrap();
    let (first_drain_tx, _first_drain_rx) = flume::bounded(1);
    app.foreground_tx
        .send(CoreMsg::PrepareForSuspend(first_drain_tx))
        .unwrap();
    let draining = wait_until(|| {
        app.queue_metrics.batch_active.load(Ordering::Acquire) && app.foreground_rx.is_empty()
    });

    let shutdown_app = app.clone();
    let (shutdown_tx, shutdown_rx) = flume::bounded(1);
    let shutdown = thread::spawn(move || {
        shutdown_app.shutdown_and_wait();
        let database_closed = shutdown_app.shared_db_read().is_none();
        shutdown_tx.send(database_closed).unwrap();
    });
    let shutdown_queued = wait_until(|| app.foreground_rx.len() == 1);
    // Keep the second suspension beyond the next prefetched batch. Otherwise
    // its reply would be dropped with that batch and miss the retained-receiver
    // failure entirely. None of these trailing shutdown messages will run.
    for _ in 1..CORE_FOREGROUND_BATCH_LIMIT {
        app.foreground_tx.send(CoreMsg::Shutdown(None)).unwrap();
    }
    let suspend_app = app.clone();
    let (suspend_tx, suspend_rx) = flume::bounded(1);
    let suspension = thread::spawn(move || {
        suspend_app.prepare_for_suspend_with_grace(true, Duration::from_millis(20), None);
        let database_closed = suspend_app.shared_db_read().is_none();
        suspend_tx.send(database_closed).unwrap();
    });
    let suspension_queued =
        wait_until(|| app.foreground_rx.len() == CORE_FOREGROUND_BATCH_LIMIT + 1);
    // Keep both requests pending beyond the second request's short grace.
    // Shutdown is ahead of it, so its reply need never be processed; the app's
    // retained foreground receiver must not keep the FFI wait alive forever.
    let early_suspend = suspend_rx.recv_timeout(Duration::from_millis(60)).ok();
    let early_shutdown = shutdown_rx.try_recv().ok();
    drop(database_work);
    drop(database);

    let shutdown_closed =
        early_shutdown.or_else(|| shutdown_rx.recv_timeout(Duration::from_secs(2)).ok());
    let suspension_closed =
        early_suspend.or_else(|| suspend_rx.recv_timeout(Duration::from_secs(2)).ok());
    assert!(
        draining && shutdown_queued && suspension_queued,
        "queue order not established"
    );
    assert!(early_suspend.is_none() && early_shutdown.is_none());
    assert_eq!(shutdown_closed, Some(true));
    assert_eq!(suspension_closed, Some(true));
    shutdown.join().unwrap();
    suspension.join().unwrap();
    assert!(app.core_worker.lock().unwrap().is_none());
    assert!(matches!(
        app.foreground_rx.try_recv(),
        Ok(CoreMsg::PrepareForSuspend(_))
    ));
    assert!(released_database.upgrade().is_none());
    let reopened = rusqlite::Connection::open(directory.path().join("core.sqlite3")).unwrap();
    reopened.busy_timeout(Duration::ZERO).unwrap();
    reopened.execute_batch("BEGIN IMMEDIATE; ROLLBACK").unwrap();
    assert!(reopened.is_autocommit());
}
