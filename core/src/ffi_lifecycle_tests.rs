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
