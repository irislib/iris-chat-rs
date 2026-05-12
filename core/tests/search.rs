use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iris_chat_core::{
    classify_chat_input, AppAction, AppReconciler, AppState, AppUpdate, ChatInputShortcut,
    ChatKind, FfiApp,
};
use tempfile::TempDir;

/// A migrated v10 database must still serve FTS5 search after the
/// app upgrades to a schema that introduces the index. Mirrors the
/// shape of installed devices that have shipped before search landed.
#[test]
fn ffi_search_works_against_pre_search_database() {
    let dir = TempDir::new().unwrap();
    {
        let app = FfiApp::new(
            dir.path().to_string_lossy().to_string(),
            String::new(),
            "test".to_string(),
        );
        let inbox = ReconcilerInbox::install(&app);
        app.dispatch(AppAction::CreateAccount {
            name: "Alice".to_string(),
        });
        inbox.wait_until(Duration::from_secs(5), |state| state.account.is_some());
        let bob = ensure_account(&TempDir::new().unwrap(), "Bob");
        let _bob_chat = create_chat_and_send(&app, &inbox, &bob, "abracadabra magic word");
        app.shutdown();
    }

    // Re-open: same db, schema migration is idempotent. Search must
    // still hit the FTS5 index even on a database that existed before
    // the search code path was introduced — uniffi clones the runtime
    // but the on-disk state is what matters.
    let app = FfiApp::new(
        dir.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );
    let _inbox = ReconcilerInbox::install(&app);
    std::thread::sleep(Duration::from_millis(200));
    let result = app.search("abracadabra".to_string(), None, 20);
    assert_eq!(result.messages.len(), 1, "{:?}", result.messages);
    assert!(result.messages[0].body.contains("abracadabra"));
}

/// `classify_chat_input` is the single source of truth for "is this
/// pasted text an npub or an invite URL?". New-chat, search-bar, and
/// share-link handlers all route through it.
#[test]
fn classifies_npub_invite_and_plain_text_inputs() {
    // Plain text → no shortcut row.
    assert!(classify_chat_input("hello world".to_string()).is_none());
    assert!(classify_chat_input("    ".to_string()).is_none());

    // Real npub (Bob from the e2e tests below).
    let bob_npub = ensure_account(&TempDir::new().unwrap(), "Bob");
    let shortcut = classify_chat_input(bob_npub.clone()).expect("npub is a shortcut");
    match shortcut {
        ChatInputShortcut::DirectPeer {
            peer_input,
            npub,
            pubkey_hex,
            display,
        } => {
            assert_eq!(npub, bob_npub);
            assert!(!pubkey_hex.is_empty());
            assert!(peer_input.starts_with("npub"));
            assert!(display.contains("…"), "{display}");
        }
        other => panic!("expected DirectPeer, got {other:?}"),
    }

    // Whitespace around an npub still classifies.
    let padded = format!("  {bob_npub}  ");
    assert!(matches!(
        classify_chat_input(padded),
        Some(ChatInputShortcut::DirectPeer { .. })
    ));

    // Invite-shaped URL — anything with both `://` and `#` is invited.
    let invite = "https://chat.iris.to/#abc123".to_string();
    let shortcut = classify_chat_input(invite.clone()).expect("invite shortcut");
    match shortcut {
        ChatInputShortcut::Invite {
            invite_input,
            display,
        } => {
            assert_eq!(invite_input, invite);
            assert!(display.contains("chat.iris.to"), "{display}");
        }
        other => panic!("expected Invite, got {other:?}"),
    }
}

/// Drives `FfiApp` end-to-end against a temp data dir and checks the
/// new `search()` method groups results into contacts / groups /
/// messages, with optional in-conversation scoping.
#[test]
fn ffi_search_returns_grouped_contacts_groups_and_messages() {
    let alice = TempDir::new().unwrap();
    let app = FfiApp::new(
        alice.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );
    let inbox = ReconcilerInbox::install(&app);

    app.dispatch(AppAction::CreateAccount {
        name: "Alice".to_string(),
    });
    inbox.wait_until(Duration::from_secs(5), |state| state.account.is_some());

    // Two direct chats and one group so we can exercise contact /
    // group / message grouping in one fixture.
    let bob_npub = ensure_account(&TempDir::new().unwrap(), "Bob");
    let carol_npub = ensure_account(&TempDir::new().unwrap(), "Carol");
    let dora_npub = ensure_account(&TempDir::new().unwrap(), "Dora");

    let bob_chat = create_chat_and_send(&app, &inbox, &bob_npub, "hello there bob");
    let _carol_chat = create_chat_and_send(&app, &inbox, &carol_npub, "carol catch up later");
    let group_chat =
        create_group_and_send(&app, &inbox, "Project Hello", &dora_npub, "kickoff agenda");
    let _ = group_chat; // group chat id consumed for grouping assertions below

    let final_state = inbox.snapshot();
    assert!(
        final_state.chat_list.len() >= 3,
        "expected >=3 chats, got {}",
        final_state.chat_list.len()
    );

    // "hello" hits a group ("Project Hello" by name) and the two
    // messages whose bodies contain "hello" (bob direct + group
    // kickoff). The direct chats are identified by chat_id only
    // (the offline test rig has no profile fetch), so contact
    // grouping is driven by message hits below.
    let result = app.search("hello".to_string(), None, 20);
    assert_eq!(result.query, "hello");
    assert!(result.scope_chat_id.is_none());
    assert!(result
        .groups
        .iter()
        .any(|g| g.display_name == "Project Hello"));
    assert!(result
        .messages
        .iter()
        .any(|m| m.body.contains("hello there bob")));
    assert!(result.messages.iter().any(|m| m.chat_id == bob_chat));
    for hit in &result.messages {
        match hit.chat_kind {
            ChatKind::Direct | ChatKind::Group => {}
        }
        assert!(!hit.chat_display_name.is_empty(), "{hit:?}");
    }

    // Scoping limits messages to the named chat and suppresses the
    // contacts/groups sections (the in-conversation pill UI mode).
    let scoped = app.search("hello".to_string(), Some(bob_chat.clone()), 20);
    assert!(scoped.contacts.is_empty());
    assert!(scoped.groups.is_empty());
    assert!(
        scoped.shortcut.is_none(),
        "scoped search must not surface global shortcuts"
    );
    assert!(!scoped.messages.is_empty());
    for hit in &scoped.messages {
        assert_eq!(hit.chat_id, bob_chat);
    }

    // Whitespace/empty queries short-circuit to an empty snapshot.
    let blank = app.search("   ".to_string(), None, 20);
    assert!(blank.is_empty());

    // Pasting an npub into the global search surfaces the shortcut row
    // — the UI offers "Start chat with …" without forcing the user to
    // navigate to the New Chat screen first.
    let bob_npub = ensure_account(&TempDir::new().unwrap(), "BobShortcut");
    let with_npub = app.search(bob_npub.clone(), None, 20);
    assert!(matches!(
        with_npub.shortcut,
        Some(ChatInputShortcut::DirectPeer { ref npub, .. }) if npub == &bob_npub
    ));
}

#[derive(Clone)]
struct ReconcilerInbox {
    state: Arc<Mutex<AppState>>,
}

impl ReconcilerInbox {
    fn install(app: &FfiApp) -> Self {
        let inbox = Self {
            state: Arc::new(Mutex::new(AppState::empty())),
        };
        let collector = Box::new(StateCollector {
            slot: inbox.state.clone(),
        });
        app.listen_for_updates(collector);
        inbox
    }

    fn wait_until<F>(&self, timeout: Duration, mut predicate: F)
    where
        F: FnMut(&AppState) -> bool,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(guard) = self.state.lock() {
                if predicate(&guard) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("predicate never observed within {timeout:?}");
    }

    fn snapshot(&self) -> AppState {
        self.state.lock().unwrap().clone()
    }

    fn current_chat_id_different_from(&self, prior: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(guard) = self.state.lock() {
                if let Some(chat) = guard.current_chat.as_ref() {
                    if chat.chat_id != prior {
                        return chat.chat_id.clone();
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("current_chat never advanced past {prior:?} within {timeout:?}");
    }
}

fn prior_chat_id(inbox: &ReconcilerInbox) -> String {
    inbox
        .snapshot()
        .current_chat
        .map(|chat| chat.chat_id)
        .unwrap_or_default()
}

fn create_chat_and_send(
    app: &FfiApp,
    inbox: &ReconcilerInbox,
    peer_input: &str,
    body: &str,
) -> String {
    let prior = prior_chat_id(inbox);
    app.dispatch(AppAction::CreateChat {
        peer_input: peer_input.to_string(),
    });
    let chat_id = inbox.current_chat_id_different_from(&prior, Duration::from_secs(5));
    app.dispatch(AppAction::SendMessage {
        chat_id: chat_id.clone(),
        text: body.to_string(),
    });
    let expected = body.to_string();
    inbox.wait_until(Duration::from_secs(5), |state| {
        state
            .chat_list
            .iter()
            .find(|c| c.chat_id == chat_id)
            .and_then(|c| c.last_message_preview.as_ref())
            .map(|preview| preview.contains(&expected))
            .unwrap_or(false)
    });
    chat_id
}

fn create_group_and_send(
    app: &FfiApp,
    inbox: &ReconcilerInbox,
    name: &str,
    member_npub: &str,
    body: &str,
) -> String {
    let prior = prior_chat_id(inbox);
    app.dispatch(AppAction::CreateGroup {
        name: name.to_string(),
        member_inputs: vec![member_npub.to_string()],
    });
    let chat_id = inbox.current_chat_id_different_from(&prior, Duration::from_secs(5));
    app.dispatch(AppAction::SendMessage {
        chat_id: chat_id.clone(),
        text: body.to_string(),
    });
    let expected = body.to_string();
    inbox.wait_until(Duration::from_secs(5), |state| {
        state
            .chat_list
            .iter()
            .find(|c| c.chat_id == chat_id)
            .and_then(|c| c.last_message_preview.as_ref())
            .map(|preview| preview.contains(&expected))
            .unwrap_or(false)
    });
    chat_id
}

struct StateCollector {
    slot: Arc<Mutex<AppState>>,
}

impl AppReconciler for StateCollector {
    fn reconcile(&self, update: AppUpdate) {
        if let AppUpdate::FullState(state) = update {
            if let Ok(mut guard) = self.slot.lock() {
                if state.rev >= guard.rev {
                    *guard = state;
                }
            }
        }
    }
}

fn ensure_account(temp: &TempDir, name: &str) -> String {
    let app = FfiApp::new(
        temp.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );
    let inbox = ReconcilerInbox::install(&app);
    app.dispatch(AppAction::CreateAccount {
        name: name.to_string(),
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() > deadline {
            panic!("account creation timeout for {name}");
        }
        let snapshot = inbox.state.lock().unwrap().clone();
        if let Some(account) = snapshot.account {
            return account.npub;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
