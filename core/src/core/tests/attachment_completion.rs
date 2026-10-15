fn attachment_completion_fixture() -> (AppCore, String, tempfile::TempDir) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let chat_id = Keys::generate().public_key().to_hex();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("attachment-completion", &owner, &device);
    core.active_chat_id = Some(chat_id.clone());
    core.screen_stack = vec![Screen::Chat {
        chat_id: chat_id.clone(),
    }];
    // Model the time between starting an upload and receiving its result. The
    // upload itself is covered by attachment_upload::upload_tests, without
    // requiring a network upload to exercise this delayed completion path.
    core.state.busy.uploading_attachment = true;
    core.state.busy.upload_progress = Some(crate::state::UploadProgress {
        bytes_uploaded: 4,
        total_bytes: 8,
    });
    (core, chat_id, directory)
}

#[test]
fn attachment_upload_completion_preserves_the_next_persisted_draft() {
    let (mut core, chat_id, directory) = attachment_completion_fixture();
    core.set_chat_draft(&chat_id, "The next message, still being written");

    core.handle_attachment_upload_finished(
        chat_id.clone(),
        Ok("The original attachment caption".into()),
    );

    assert_eq!(
        core.threads[&chat_id].draft,
        "The next message, still being written"
    );
    assert_eq!(core.threads[&chat_id].messages.len(), 1);
    assert_eq!(
        core.threads[&chat_id].messages[0].body,
        "The original attachment caption"
    );
    assert!(!core.state.busy.uploading_attachment);
    assert!(core.state.busy.upload_progress.is_none());

    // Reopen the actual database rather than relying on composer-local state.
    drop(core);
    let mut reopened = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        directory.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let persisted = reopened.load_persisted().unwrap().unwrap();
    let thread = persisted
        .threads
        .iter()
        .find(|thread| thread.chat_id == chat_id)
        .unwrap();
    assert_eq!(thread.draft, "The next message, still being written");
}

#[test]
fn attachment_upload_completion_does_not_return_to_the_upload_chat() {
    let other_chat = Keys::generate().public_key().to_hex();
    for (active_chat, expected_stack) in [
        (
            Some(other_chat.clone()),
            vec![Screen::Chat {
                chat_id: other_chat.clone(),
            }],
        ),
        (None, vec![Screen::ChatList]),
        (Some(other_chat.clone()), vec![Screen::Settings]),
        (
            Some(other_chat.clone()),
            vec![
                Screen::Chat {
                    chat_id: other_chat.clone(),
                },
                Screen::DirectChatInfo {
                    chat_id: other_chat.clone(),
                },
            ],
        ),
    ] {
        let (mut core, upload_chat, _directory) = attachment_completion_fixture();
        core.set_chat_draft(&other_chat, "Draft in another chat");
        core.active_chat_id = active_chat.clone();
        core.screen_stack = expected_stack.clone();

        core.handle_attachment_upload_finished(upload_chat.clone(), Ok("Uploaded file".into()));

        assert_eq!(core.active_chat_id, active_chat);
        assert_eq!(core.screen_stack, expected_stack);
        assert_eq!(core.threads[&other_chat].draft, "Draft in another chat");
        assert_eq!(core.threads[&upload_chat].messages.len(), 1);
        let persisted = core.load_persisted().unwrap().unwrap();
        assert_eq!(persisted.active_chat_id, active_chat);
    }
}

#[test]
fn attachment_upload_failure_preserves_drafts_and_navigation() {
    let (mut core, chat_id, _directory) = attachment_completion_fixture();
    core.set_chat_draft(&chat_id, "The next message");
    core.active_chat_id = None;
    core.screen_stack = vec![Screen::Settings];

    core.handle_attachment_upload_finished(chat_id.clone(), Err("Upload interrupted".into()));

    assert_eq!(core.threads[&chat_id].draft, "The next message");
    assert!(core.threads[&chat_id].messages.is_empty());
    assert_eq!(core.active_chat_id, None);
    assert_eq!(core.screen_stack, vec![Screen::Settings]);
    assert!(!core.state.busy.uploading_attachment);
    assert!(core.state.busy.upload_progress.is_none());
    assert_eq!(core.state.toast.as_deref(), Some("Attachment upload failed."));
}

#[test]
fn attachment_upload_group_completion_preserves_draft_and_navigation() {
    let (mut core, other_chat, _directory) = attachment_completion_fixture();
    let owner = core.logged_in.as_ref().unwrap().owner_pubkey;
    let group = test_group_snapshot(
        "attachment-completion-group",
        "Friends",
        owner,
        vec![owner, Keys::generate().public_key()],
        vec![owner],
        1,
    );
    core.protocol_engine
        .as_mut()
        .unwrap()
        .install_device_sync_group(group.clone())
        .unwrap();
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(group.clone()));
    let group_chat = group_chat_id(&group.group_id);
    core.set_chat_draft(&group_chat, "The next group message");
    core.active_chat_id = Some(other_chat.clone());
    core.screen_stack = vec![Screen::Chat {
        chat_id: other_chat.clone(),
    }];

    core.handle_attachment_upload_finished(group_chat.clone(), Ok("Group attachment caption".into()));

    assert_eq!(core.threads[&group_chat].draft, "The next group message");
    assert!(core.threads[&group_chat]
        .messages
        .iter()
        .any(|message| message.body == "Group attachment caption"));
    assert_eq!(core.active_chat_id.as_deref(), Some(other_chat.as_str()));
    assert_eq!(
        core.screen_stack,
        vec![Screen::Chat {
            chat_id: other_chat,
        }]
    );
}

#[test]
fn direct_files_preparation_preserves_the_next_draft_and_navigation() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let chat_id = Keys::generate().public_key().to_hex();
    let (mut core, _updates, directory) =
        logged_in_test_core_with_updates("direct-file-draft", &owner, &device);
    core.preferences.nostr_relay_urls.clear();
    core.preferences.nearby_enabled = false;
    let (sender, pending) = flume::unbounded();
    core.core_sender = sender.clone();
    core.priority_sender = sender;
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(rendezvous) = socket.local_addr().unwrap() else {
        unreachable!()
    };
    core.reconcile_device_sync_at_rendezvous_for_test(rendezvous);
    let source = directory.path().join("draft.txt");
    std::fs::write(&source, b"Private direct-file draft test").unwrap();
    core.set_chat_draft(&chat_id, "The submitted file caption");
    core.handle_action(AppAction::SendDirectFiles {
        chat_id: chat_id.clone(),
        attachments: vec![OutgoingAttachment {
            file_path: source.to_string_lossy().into_owned(),
            filename: "draft.txt".into(),
        }],
        caption: "The submitted file caption".into(),
    });
    assert!(core.state.busy.sending_message);
    // The composer consumes its submitted draft immediately. Hold the actual
    // worker result in the core queue while the user edits and navigates away.
    core.handle_action(AppAction::SetChatDraft {
        chat_id: chat_id.clone(),
        text: String::new(),
    });
    assert!(core.threads[&chat_id].draft.is_empty());
    core.set_chat_draft(&chat_id, "A later draft while the file is prepared");
    core.active_chat_id = None;
    core.screen_stack = vec![Screen::Settings];

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while core.state.busy.sending_message {
        for message in pending.try_iter().take(128) {
            core.handle_message(message);
        }
        assert!(std::time::Instant::now() < deadline, "file preparation timed out");
        std::thread::sleep(Duration::from_millis(2));
    }

    assert_eq!(core.threads[&chat_id].draft, "A later draft while the file is prepared");
    assert_eq!(core.active_chat_id, None);
    assert_eq!(core.screen_stack, vec![Screen::Settings]);
    let records = direct_files_test_records(&core);
    assert_eq!(records.len(), 1);
    let message = direct_files_test_message(&core, &chat_id, &records[0].offer.id);
    assert_eq!(message.body, "The submitted file caption");
    assert_eq!(message.direct_transfer.unwrap().files[0].filename, "draft.txt");
    let persisted = core.load_persisted().unwrap().unwrap();
    assert_eq!(
        persisted.threads.iter().find(|thread| thread.chat_id == chat_id).unwrap().draft,
        "A later draft while the file is prepared"
    );
}
