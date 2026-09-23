use super::*;
use crate::core::chats::chat_message_from_persisted;
use crate::state::CallHistorySnapshot;

impl Fixture {
    fn history(&self) -> &CallHistorySnapshot {
        self.core.threads[&self.owner].messages[0]
            .call
            .as_ref()
            .unwrap()
    }

    fn saved_history(&self) -> CallHistorySnapshot {
        let store = storage::AppStore::new(storage::open_database(self._directory.path()).unwrap());
        store
            .load_recent_messages(&self.owner, 80)
            .unwrap()
            .into_iter()
            .find_map(|message| message.call)
            .unwrap()
    }

    fn ready(&mut self) {
        self.core.handle_action(AppAction::SetCallMediaConnected {
            call_id: CALL_ID.into(),
            connected: true,
        });
    }
}

#[test]
fn call_history_ringing_is_durable_missed_and_delayed_offer_cannot_duplicate_or_revive_it() {
    let mut f = Fixture::new();
    f.core.active_chat_id = Some(f.owner.clone());
    f.receive(0, "offer", true);
    assert!(f
        .core
        .state
        .current_chat
        .as_ref()
        .unwrap()
        .messages
        .is_empty());
    assert_eq!(f.core.state.chat_list[0].last_message_preview, None);
    assert_eq!(f.saved_history().outcome, "missed");
    assert!(f.history().video);
    f.receive(0, "offer", true);
    assert_eq!(f.core.threads[&f.owner].messages.len(), 1);
    f.core.calls.active.as_mut().unwrap().started = Clock::now() - Duration::from_secs(31);
    f.core.call_tick(CALL_ID);
    let missed = f.saved_history();
    assert_eq!(
        f.core.state.current_chat.as_ref().unwrap().messages.len(),
        1
    );
    assert_eq!(missed.direction, "incoming");
    assert_eq!(missed.answered_at_secs, None);
    assert_eq!(missed.duration_secs, 0);
    assert_eq!(
        f.core.state.chat_list[0].last_message_preview.as_deref(),
        Some("Missed video call")
    );
    assert_eq!(f.core.state.chat_list[0].last_message_delivery, None);
    // Simulate a cold start / paged-out row: persisted IDs must still dedupe.
    f.core.calls = CallRuntime::default();
    f.core.state.call = None;
    f.core.threads.get_mut(&f.owner).unwrap().messages.clear();
    f.receive(0, "offer", true);
    assert!(f.core.calls.active.is_none());
    assert_eq!(f.saved_history(), missed);
}

#[test]
fn call_history_voice_answer_waits_for_media_readiness_and_counts_muted_call_duration() {
    let mut f = Fixture::new();
    f.receive(0, "offer", true);
    f.core
        .handle_action(AppAction::SetCallMuted { muted: true });
    f.core.handle_action(AppAction::AnswerCallWithVoice {
        call_id: CALL_ID.into(),
    });
    assert_eq!(f.snapshot().phase, "connected");
    assert_eq!(
        f.saved_history().outcome,
        "missed",
        "acceptance alone is not media readiness"
    );
    assert!(!f.history().video);
    f.ready();
    assert_eq!(f.saved_history().outcome, "answered");
    let answered_at = f.history().answered_at_secs.unwrap();
    f.core.calls.active.as_mut().unwrap().answered =
        Some((answered_at, Clock::now() - Duration::from_secs(42)));
    f.core.handle_action(AppAction::SetCallMediaConnected {
        call_id: CALL_ID.into(),
        connected: false,
    });
    f.ready();
    f.core.handle_action(AppAction::EndCall {
        call_id: CALL_ID.into(),
    });
    let call = f.saved_history();
    assert_eq!(call.outcome, "answered");
    assert_eq!(call.direction, "incoming");
    assert!(!call.video);
    assert_eq!(call.answered_at_secs, Some(answered_at));
    assert!((42..44).contains(&call.duration_secs));
    assert_eq!(f.core.threads[&f.owner].messages.len(), 1);
}

#[test]
fn call_history_distinguishes_local_decline_remote_rejection_and_unanswered_cancellation() {
    for outgoing in [false, true] {
        for declined in [false, true] {
            let mut f = Fixture::new();
            if outgoing {
                f.outgoing(false);
            } else {
                f.receive(0, "offer", false);
            }
            if outgoing && declined {
                f.receive(0, "reject", false);
                assert!(
                    f.core.calls.active.is_some(),
                    "other device may still answer"
                );
                f.receive(1, "reject", false);
            } else if !outgoing && !declined {
                f.receive(0, "end", false);
            } else {
                f.core.handle_action(AppAction::EndCall {
                    call_id: CALL_ID.into(),
                });
            }
            let expected = if declined {
                "declined"
            } else if outgoing {
                "canceled"
            } else {
                "missed"
            };
            assert_eq!(f.saved_history().outcome, expected);
        }
    }
    let mut f = Fixture::new();
    f.outgoing(true);
    f.receive(0, "answer", true);
    f.core.handle_action(AppAction::EndCall {
        call_id: CALL_ID.into(),
    });
    assert_eq!(
        f.saved_history().outcome,
        "canceled",
        "no media engine became ready"
    );
}

#[test]
fn call_history_update_keeps_timeline_order_and_is_not_device_synced() {
    let mut f = Fixture::new();
    f.outgoing(true);
    f.receive(0, "answer", true);
    f.ready();
    f.core
        .handle_action(AppAction::SetCallVideoEnabled { enabled: false });
    let mut text = f.core.threads[&f.owner].messages[0].clone();
    text.id = "message-after-call".into();
    text.body = "After the call started".into();
    text.kind = ChatMessageKind::User;
    text.call = None;
    // Same-second insert order must survive both upsert and SQLite reload.
    f.core
        .threads
        .get_mut(&f.owner)
        .unwrap()
        .insert_message_sorted(text);
    f.core.handle_action(AppAction::EndCall {
        call_id: CALL_ID.into(),
    });
    assert_eq!(f.saved_history().outcome, "answered");
    assert!(
        f.history().video,
        "camera toggle does not change negotiated call type"
    );
    let page = f.core.app_store.load_recent_messages(&f.owner, 10).unwrap();
    assert_eq!(page[0].id, format!("call:{CALL_ID}"));
    assert_eq!(page[1].id, "message-after-call");
    assert_eq!(
        chat_message_from_persisted(&page[0]).call.as_ref(),
        Some(f.history())
    );
    assert_eq!(
        f.core.state.chat_list[0].last_message_preview.as_deref(),
        Some("After the call started")
    );
    let sync = f
        .core
        .app_store
        .load_device_sync_messages_page(0, unix_now().get(), 0, "", "", 10)
        .unwrap();
    assert_eq!(sync.len(), 1);
    assert_eq!(sync[0].id, "message-after-call");
    let before = f
        .core
        .app_store
        .load_messages_before(&f.owner, "message-after-call", 10)
        .unwrap();
    assert_eq!(before[0].call.as_ref(), Some(f.history()));
    let around = f
        .core
        .app_store
        .load_messages_around(&f.owner, &format!("call:{CALL_ID}"), 1, 1)
        .unwrap();
    assert_eq!(around[0].call.as_ref(), Some(f.history()));
}

#[test]
fn call_history_finishing_active_call_does_not_recreate_deleted_chat() {
    let mut f = Fixture::new();
    f.connected_incoming(false);
    f.ready();
    f.core.delete_chat(&f.owner);
    f.core.handle_action(AppAction::EndCall {
        call_id: CALL_ID.into(),
    });
    assert!(!f.core.threads.contains_key(&f.owner));
    assert!(f
        .core
        .app_store
        .load_recent_messages(&f.owner, 10)
        .unwrap()
        .is_empty());
}

#[test]
fn call_history_answered_elsewhere_overrides_losing_devices_local_media_readiness() {
    let mut f = Fixture::new();
    f.connected_incoming(true);
    f.ready();
    let mut ended = Signal::new("end", CALL_ID, false, false);
    ended.reason = Some("answered_elsewhere".into());
    f.core
        .handle_call_packet(&f.devices[0], PORT, &serde_json::to_vec(&ended).unwrap());
    let call = f.saved_history();
    assert_eq!(call.outcome, "answered_elsewhere");
    assert_eq!(call.answered_at_secs, None);
    assert_eq!(call.duration_secs, 0);
    assert!(!call.video);
    assert_eq!(
        f.core.state.chat_list[0].last_message_preview.as_deref(),
        Some("Answered on another device")
    );
}
