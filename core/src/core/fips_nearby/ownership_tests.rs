use super::*;
use crate::fips_ble_ffi::{FfiFipsBle, FipsBleBridgeError};

fn core() -> (AppCore, tempfile::TempDir) {
    let directory = tempfile::TempDir::new().unwrap();
    let core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        directory.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    (core, directory)
}

fn receive(queue: &flume::Receiver<CoreMsg>) -> CoreMsg {
    queue
        .recv_timeout(Duration::from_secs(2))
        .expect("queued BLE lifecycle command")
}

fn connect(
    core: &mut AppCore,
    tx: &flume::Sender<CoreMsg>,
    rx: &flume::Receiver<CoreMsg>,
) -> Arc<FfiFipsBle> {
    let tx = tx.clone();
    let worker = std::thread::spawn(move || FfiFipsBle::connect(tx, Duration::from_secs(2)));
    assert!(core.handle_message(receive(rx)));
    worker.join().unwrap().unwrap()
}

#[test]
fn timed_out_queued_ffi_attachment_is_cancelled_before_core_accepts_it() {
    let (mut core, _directory) = core();
    let (tx, rx) = flume::unbounded();
    // Hold actual core delivery until the real constructor's deadline expires.
    assert!(FfiFipsBle::connect(tx.clone(), Duration::ZERO).is_err());
    let attach = receive(&rx);
    let CoreMsg::AttachHostBle { ref attachment, .. } = attach else {
        panic!("attach")
    };
    assert!(attachment.ownership().is_cancelled());
    let cancelled_owner = attachment.ownership();
    let cleanup = receive(&rx);
    assert!(core.handle_message(attach));
    assert!(core.pending_host_ble.is_none());
    assert!(core.host_ble_ownership.is_none());

    // Deliver the abandoned constructor's cleanup after a different bridge
    // has attached. It must not tear down that replacement.
    let replacement = connect(&mut core, &tx, &rx);
    let current_owner = core.host_ble_ownership.clone().unwrap();
    assert!(!current_owner.matches(&cancelled_owner));
    assert!(core.handle_message(cleanup));
    assert!(core
        .host_ble_ownership
        .as_ref()
        .unwrap()
        .matches(&current_owner));
    assert!(core.pending_host_ble.is_some());
    drop(replacement);
    assert!(core.handle_message(receive(&rx)));
    assert!(core.pending_host_ble.is_none());
}

#[test]
fn ffi_detach_timeout_is_not_success_and_can_retry_until_core_acknowledges() {
    let (mut core, _directory) = core();
    let (tx, rx) = flume::unbounded();
    let bridge = connect(&mut core, &tx, &rx);
    assert!(matches!(
        bridge.detach_with_timeout(Duration::ZERO),
        Err(FipsBleBridgeError::Teardown(_))
    ));
    assert!(
        core.pending_host_ble.is_some(),
        "the held detach has not executed"
    );
    assert!(core.handle_message(receive(&rx)));
    assert!(core.pending_host_ble.is_none());

    let retry = bridge.clone();
    let worker = std::thread::spawn(move || retry.detach());
    assert!(core.handle_message(receive(&rx)));
    assert!(worker.join().unwrap().is_ok());
    assert!(bridge.detach_with_timeout(Duration::ZERO).is_ok());
    drop(bridge);
    assert!(
        rx.try_recv().is_err(),
        "acknowledged close must not enqueue teardown again"
    );
}

#[test]
fn dropping_an_acknowledged_ffi_bridge_cleans_only_its_own_attachment() {
    let (mut core, _directory) = core();
    let (tx, rx) = flume::unbounded();
    let first = connect(&mut core, &tx, &rx);
    let first_owner = core.host_ble_ownership.clone().unwrap();
    drop(first);
    let cleanup = receive(&rx);
    assert!(first_owner.is_cancelled());
    assert!(core.handle_message(cleanup));
    assert!(core.pending_host_ble.is_none());
    let replacement = connect(&mut core, &tx, &rx);
    let replacement_owner = core.host_ble_ownership.clone().unwrap();
    let (reply_tx, reply_rx) = flume::bounded(1);
    assert!(core.handle_message(CoreMsg::DetachHostBle {
        ownership: first_owner,
        reply_tx
    }));
    reply_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(core
        .host_ble_ownership
        .as_ref()
        .unwrap()
        .matches(&replacement_owner));
    drop(replacement);
    assert!(core.handle_message(receive(&rx)));
}

#[test]
fn rejected_duplicate_ffi_attachment_cleanup_cannot_detach_the_live_bridge() {
    let (mut core, _directory) = core();
    let (tx, rx) = flume::unbounded();
    let first = connect(&mut core, &tx, &rx);
    let owner = core.host_ble_ownership.clone().unwrap();
    let duplicate_tx = tx.clone();
    let worker =
        std::thread::spawn(move || FfiFipsBle::connect(duplicate_tx, Duration::from_secs(2)));
    assert!(core.handle_message(receive(&rx)));
    assert!(worker.join().unwrap().is_err());
    assert!(core.handle_message(receive(&rx)));
    assert!(core.host_ble_ownership.as_ref().unwrap().matches(&owner));
    assert!(!owner.is_cancelled());
    drop(first);
    assert!(core.handle_message(receive(&rx)));
}

#[test]
fn cancellation_after_acceptance_prevents_consuming_single_use_ble_io() {
    let (mut core, _directory) = core();
    let (io, _adapter) = core
        .runtime
        .block_on(async { HostBleIo::channel("mobile", "cancel-before-bind", 8) })
        .unwrap();
    let attachment = HostBleAttachment::new(io);
    let owner = attachment.ownership();
    core.attach_host_ble(attachment).unwrap();
    owner.cancel();
    assert!(core.pending_host_ble.as_mut().unwrap().take().is_none());
    core.detach_owned_host_ble(&owner);
    assert!(core.pending_host_ble.is_none());
}

#[test]
fn disconnected_attachment_reply_does_not_leave_a_pending_bridge() {
    let (mut core, _directory) = core();
    let (io, _adapter) = core
        .runtime
        .block_on(async { HostBleIo::channel("mobile", "abandoned-reply", 8) })
        .unwrap();
    let (reply_tx, reply_rx) = flume::bounded(1);
    drop(reply_rx);
    assert!(core.handle_message(CoreMsg::AttachHostBle {
        attachment: HostBleAttachment::new(io),
        reply_tx
    }));
    assert!(core.pending_host_ble.is_none());
    assert!(core.host_ble_ownership.is_none());
}
