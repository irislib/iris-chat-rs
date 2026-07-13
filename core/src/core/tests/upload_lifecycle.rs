use super::*;

struct UploadHarness {
    core: AppCore,
    core_rx: flume::Receiver<CoreMsg>,
    _update_rx: flume::Receiver<AppUpdate>,
    _temp_dir: tempfile::TempDir,
}

impl UploadHarness {
    fn new() -> Self {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let (update_tx, update_rx) = flume::unbounded();
        let (core_tx, core_rx) = flume::unbounded();
        let core = AppCore::new(
            update_tx,
            core_tx,
            temp_dir.path().to_string_lossy().to_string(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        Self {
            core,
            core_rx,
            _update_rx: update_rx,
            _temp_dir: temp_dir,
        }
    }

    fn start_pending(&mut self, target: UploadTarget) -> u64 {
        self.core
            .start_upload(target, std::future::pending())
            .expect("upload should start")
    }

    fn dispatch_next_completion(&mut self) {
        let message = self
            .core_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("completion event");
        assert!(self.core.handle_message(message));
    }
}

impl Drop for UploadHarness {
    fn drop(&mut self) {
        self.core.cancel_upload();
    }
}

#[test]
fn profile_completion_does_not_clear_an_unrelated_attachment_upload() {
    let mut harness = UploadHarness::new();
    let profile_id = harness.start_pending(UploadTarget::ProfilePicture);
    harness.core.cancel_upload();
    let chat_id = harness.start_pending(UploadTarget::ChatAttachments {
        chat_id: "chat-b".to_string(),
    });

    harness
        .core
        .handle_upload_finished(profile_id, Err("late profile failure".to_string()));

    let active = harness
        .core
        .upload_runtime
        .active
        .as_ref()
        .expect("chat upload remains active");
    assert_eq!(active.id, chat_id);
    assert_eq!(
        active.target,
        UploadTarget::ChatAttachments {
            chat_id: "chat-b".to_string()
        }
    );
    assert!(harness.core.state.busy.uploading_attachment);
    assert_eq!(harness.core.state.busy.upload_progress, None);
    assert_eq!(harness.core.state.toast, None);
}

#[test]
fn upload_slot_is_shared_and_a_rejected_future_is_never_polled() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut harness = UploadHarness::new();
    let chat_id = harness.start_pending(UploadTarget::ChatAttachments {
        chat_id: "chat-a".to_string(),
    });
    let polled = Arc::new(AtomicBool::new(false));
    let polled_by_future = polled.clone();

    let rejected = harness
        .core
        .start_upload(UploadTarget::ProfilePicture, async move {
            polled_by_future.store(true, Ordering::SeqCst);
            std::future::pending().await
        });

    assert_eq!(rejected, None);
    assert!(!polled.load(Ordering::SeqCst));
    assert_eq!(
        harness
            .core
            .upload_runtime
            .active
            .as_ref()
            .map(|active| active.id),
        Some(chat_id)
    );
    assert_eq!(
        harness.core.state.toast.as_deref(),
        Some("Attachment upload already in progress.")
    );
}

#[test]
fn profile_picture_entry_point_uses_the_shared_upload_slot() {
    let mut harness = UploadHarness::new();
    let owner = Keys::generate();
    let device = Keys::generate();
    harness.core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner),
        device_keys: device.clone(),
        client: Client::new(device),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    let chat_id = harness.start_pending(UploadTarget::ChatAttachments {
        chat_id: "chat-a".to_string(),
    });
    let picture_path = harness._temp_dir.path().join("profile.png");
    fs::write(&picture_path, b"not polled by the rejected upload").expect("write picture");

    harness
        .core
        .upload_profile_picture(&picture_path.to_string_lossy());

    let active = harness
        .core
        .upload_runtime
        .active
        .as_ref()
        .expect("chat upload remains active");
    assert_eq!(active.id, chat_id);
    assert_eq!(
        active.target,
        UploadTarget::ChatAttachments {
            chat_id: "chat-a".to_string()
        }
    );
    assert_eq!(
        harness.core.state.toast.as_deref(),
        Some("Attachment upload already in progress.")
    );
    assert!(harness.core_rx.try_recv().is_err());
}

#[test]
fn matching_failures_finish_the_target_that_owns_the_slot() {
    let cases = [
        (
            UploadTarget::ChatAttachments {
                chat_id: "chat-a".to_string(),
            },
            "Attachment upload failed.",
        ),
        (
            UploadTarget::GroupPicture {
                group_id: "group-a".to_string(),
            },
            "Group photo upload failed.",
        ),
        (
            UploadTarget::ProfilePicture,
            "Profile picture upload failed: boom",
        ),
    ];

    let mut harness = UploadHarness::new();
    for (target, expected_toast) in cases {
        harness
            .core
            .start_upload(target, std::future::ready(Err("boom".to_string())))
            .expect("upload should start");
        harness.dispatch_next_completion();

        assert!(harness.core.upload_runtime.active.is_none());
        assert!(!harness.core.state.busy.uploading_attachment);
        assert_eq!(harness.core.state.busy.upload_progress, None);
        assert_eq!(harness.core.state.toast.as_deref(), Some(expected_toast));
    }
}

#[test]
fn duplicate_completion_is_ignored() {
    let mut harness = UploadHarness::new();
    let operation_id = harness
        .core
        .start_upload(
            UploadTarget::ProfilePicture,
            std::future::ready(Err("first".to_string())),
        )
        .expect("upload should start");
    harness.dispatch_next_completion();
    let state_after_first = harness.core.state.clone();
    let log_count_after_first = harness.core.debug_log.len();

    harness
        .core
        .handle_upload_finished(operation_id, Err("duplicate".to_string()));

    assert_eq!(harness.core.state, state_after_first);
    assert_eq!(harness.core.debug_log.len(), log_count_after_first);
}

#[test]
fn panicking_upload_releases_the_slot() {
    let mut harness = UploadHarness::new();
    let panicking_upload = std::future::poll_fn(|_| -> std::task::Poll<Result<String, String>> {
        panic!("simulated upload panic")
    });
    harness
        .core
        .start_upload(UploadTarget::ProfilePicture, panicking_upload)
        .expect("upload should start");

    harness.dispatch_next_completion();

    assert!(harness.core.upload_runtime.active.is_none());
    assert!(!harness.core.state.busy.uploading_attachment);
    assert_eq!(
        harness.core.state.toast.as_deref(),
        Some("Profile picture upload failed: upload task panicked")
    );
}

#[test]
fn logout_invalidates_an_upload_without_reusing_its_id() {
    let mut harness = UploadHarness::new();
    let old_id = harness.start_pending(UploadTarget::ProfilePicture);

    harness.core.logout();
    harness
        .core
        .handle_upload_finished(old_id, Err("late".to_string()));

    assert!(harness.core.upload_runtime.active.is_none());
    assert!(!harness.core.state.busy.uploading_attachment);
    assert_eq!(harness.core.state.toast, None);

    let new_id = harness.start_pending(UploadTarget::ProfilePicture);
    assert_ne!(new_id, old_id);
}

#[test]
fn session_replacement_invalidates_an_upload_without_reusing_its_id() {
    let mut harness = UploadHarness::new();
    let old_id = harness.start_pending(UploadTarget::ProfilePicture);
    let owner = Keys::generate();
    harness.core.preferences.nostr_relay_urls.clear();

    harness
        .core
        .start_session(
            owner.public_key(),
            Some(owner),
            Keys::generate(),
            false,
            false,
        )
        .expect("session should start");
    harness
        .core
        .handle_upload_finished(old_id, Err("late".to_string()));

    assert!(harness.core.upload_runtime.active.is_none());
    assert!(!harness.core.state.busy.uploading_attachment);

    let new_id = harness.start_pending(UploadTarget::ProfilePicture);
    assert_ne!(new_id, old_id);
}

#[test]
fn entity_deletion_cancels_only_the_upload_for_that_entity() {
    let mut harness = UploadHarness::new();
    let profile_id = harness.start_pending(UploadTarget::ProfilePicture);

    harness.core.cancel_upload_for_chat("chat-a");
    assert_eq!(
        harness
            .core
            .upload_runtime
            .active
            .as_ref()
            .map(|active| active.id),
        Some(profile_id)
    );
    harness.core.cancel_profile_picture_upload();
    assert!(harness.core.upload_runtime.active.is_none());

    harness.start_pending(UploadTarget::GroupPicture {
        group_id: "group-a".to_string(),
    });
    harness.core.cancel_upload_for_chat("group:group-b");
    assert!(harness.core.upload_runtime.active.is_some());
    harness.core.cancel_upload_for_chat("group:group-a");
    assert!(harness.core.upload_runtime.active.is_none());
}

#[test]
fn cancellation_aborts_the_owned_task() {
    struct NotifyOnDrop(Option<std::sync::mpsc::Sender<()>>);

    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    let mut harness = UploadHarness::new();
    let (drop_tx, drop_rx) = std::sync::mpsc::channel();
    let notify_on_drop = NotifyOnDrop(Some(drop_tx));
    harness
        .core
        .start_upload(UploadTarget::ProfilePicture, async move {
            let _notify_on_drop = notify_on_drop;
            std::future::pending().await
        })
        .expect("upload should start");

    harness.core.cancel_upload();

    drop_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("upload future should be dropped after cancellation");
}

#[test]
fn suspend_cancels_the_upload_before_internal_events_are_gated() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut harness = UploadHarness::new();
    let operation_id = harness.start_pending(UploadTarget::ProfilePicture);

    harness.core.prepare_for_suspend();
    harness
        .core
        .handle_upload_finished(operation_id, Err("late".to_string()));

    assert!(harness.core.suspended);
    assert!(harness.core.upload_runtime.active.is_none());
    assert!(!harness.core.state.busy.uploading_attachment);
    assert_eq!(harness.core.state.busy.upload_progress, None);
    assert_eq!(harness.core.state.toast, None);

    let polled = Arc::new(AtomicBool::new(false));
    let polled_by_future = polled.clone();
    let rejected = harness
        .core
        .start_upload(UploadTarget::ProfilePicture, async move {
            polled_by_future.store(true, Ordering::SeqCst);
            std::future::pending().await
        });
    assert_eq!(rejected, None);
    assert!(!polled.load(Ordering::SeqCst));
    assert!(harness.core.upload_runtime.active.is_none());
}
