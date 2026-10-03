use super::*;
use crate::{build_large_test_app_state, ffi_app_failure, ChatKind};

#[test]
fn search_messages_preserve_sender_identity_from_persisted_history() {
    let directory = tempfile::tempdir().unwrap();
    let mut state = build_large_test_app_state(1, 1, 1);
    let account = state.account.clone().unwrap();
    let mut template = state.current_chat.as_ref().unwrap().messages[0].clone();
    let peer = "aa".repeat(32);
    let member = "bb".repeat(32);
    state.chat_list[0].chat_id = peer.clone();
    state.chat_list[0].display_name = "Other participant".into();
    state.chat_list[0].picture_url = Some("https://example.com/peer.png".into());
    let group = state.chat_list[1].chat_id.clone();
    state.chat_list[1].display_name = "Test group".into();
    state.chat_list[1].picture_url = Some("https://example.com/group.png".into());
    state.account.as_mut().unwrap().display_name = "My name".into();
    state.account.as_mut().unwrap().picture_url = Some("https://example.com/me.png".into());
    // Search must resolve authors for history that is not loaded into the UI.
    state.current_chat = None;

    let mut store =
        crate::core::AppStore::new(crate::core::open_database(directory.path()).unwrap());
    {
        let shared = store.shared();
        let conn = shared.lock().unwrap();
        for (owner, name, nickname, picture) in [
            (
                &peer,
                "Other participant",
                None,
                "https://example.com/peer.png",
            ),
            (
                &member,
                "Public member name",
                Some("Group friend"),
                "https://example.com/member.png",
            ),
        ] {
            conn.execute(
                "INSERT INTO owner_profiles(owner_pubkey_hex, display_name, nickname, picture, updated_at_secs)
                 VALUES (?1, ?2, ?3, ?4, 1)",
                rusqlite::params![owner, name, nickname, picture],
            ).unwrap();
        }
    }
    for (chat, id, owner, outgoing, kind) in [
        (
            &peer,
            "direct-mine",
            Some(account.public_key_hex.as_str()),
            true,
            ChatMessageKind::User,
        ),
        (
            &peer,
            "direct-theirs",
            Some(peer.as_str()),
            false,
            ChatMessageKind::User,
        ),
        (&peer, "legacy-mine", None, true, ChatMessageKind::User),
        (&peer, "legacy-theirs", None, false, ChatMessageKind::User),
        (
            &group,
            "group-mine",
            Some(account.public_key_hex.as_str()),
            true,
            ChatMessageKind::User,
        ),
        (
            &group,
            "group-peer",
            Some(peer.as_str()),
            false,
            ChatMessageKind::User,
        ),
        (
            &group,
            "group-member",
            Some(member.as_str()),
            false,
            ChatMessageKind::User,
        ),
        (&group, "unknown-member", None, false, ChatMessageKind::User),
        (&peer, "system", None, false, ChatMessageKind::System),
    ] {
        template.chat_id = chat.clone();
        template.id = id.into();
        template.body = "searchneedle historical message".into();
        template.author = "Saved author label".into();
        template.author_owner_pubkey_hex = owner.map(str::to_owned);
        template.is_outgoing = outgoing;
        template.kind = kind;
        store
            .upsert_notification_preview_message(chat, 0, 100, &template)
            .unwrap();
    }
    drop(store);
    // Reopen the real SQLite file and use the same FFI search entry point as
    // Apple and Android. No core worker or network is needed for local search.
    let app = ffi_app_failure("isolated search fixture".into());
    *app.shared_state.write().unwrap() = state;
    crate::set_shared_db(
        &app.shared_db,
        Some(crate::core::open_database(directory.path()).unwrap()),
    );
    let direct = app.search("searchneedle".into(), Some(peer.clone()), 50);
    let group_hits = app.search("searchneedle".into(), Some(group), 50);
    let global = app.search("searchneedle".into(), None, 50);
    assert_eq!(direct.messages.len(), 5);
    assert_eq!(group_hits.messages.len(), 4);
    assert_eq!(global.messages.len(), 9);
    for results in [&direct, &group_hits, &global] {
        for hit in &results.messages {
            let (owner, name, picture) = match hit.message_id.as_str() {
                "direct-mine" | "legacy-mine" | "group-mine" => (
                    account.public_key_hex.as_str(),
                    "My name",
                    Some("https://example.com/me.png"),
                ),
                "direct-theirs" | "legacy-theirs" | "group-peer" => (
                    peer.as_str(),
                    "Other participant",
                    Some("https://example.com/peer.png"),
                ),
                "group-member" => (
                    member.as_str(),
                    "Group friend",
                    Some("https://example.com/member.png"),
                ),
                "unknown-member" | "system" => ("", "Saved author label", None),
                id => panic!("unexpected result {id}"),
            };
            assert_eq!(hit.author_pubkey, owner, "{} identity", hit.message_id);
            assert_eq!(hit.author_display_name, name, "{} name", hit.message_id);
            assert_eq!(
                hit.author_picture_url.as_deref(),
                picture,
                "{} avatar",
                hit.message_id
            );
            assert_eq!(
                hit.chat_display_name,
                if hit.chat_kind == ChatKind::Group {
                    "Test group"
                } else {
                    "Other participant"
                }
            );
        }
    }
    app.shutdown_inner(true);
}
