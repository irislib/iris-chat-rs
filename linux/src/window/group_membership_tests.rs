use super::*;
use iris_chat_core::{
    ChatKind, ChatMessageKind, ChatMessageSnapshot, ChatParticipantSnapshot, DeliveryState,
    MessageReactionSnapshot,
};

fn button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
        if button.label().as_deref() == Some(label)
            || button.tooltip_text().as_deref() == Some(label)
        {
            return Some(button);
        }
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = button(&widget, label) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

pub(super) fn run(
    manager: &Rc<AppManager>,
    slot: &Content,
    header: &HeaderWidgets,
    drop_target: &gtk::DropTarget,
    files: &gtk::gdk::FileList,
) {
    let original = manager.current_state();
    let apply = |mut state: AppState| {
        state.rev = manager.current_state().rev + 1;
        assert!(manager.apply_update(AppUpdate::FullState(state)).is_some());
        apply_state(slot, header, manager, &manager.current_state());
    };
    let mut member = original.clone();
    let local_owner = member.account.as_ref().unwrap().public_key_hex.clone();
    let chat = member.current_chat.as_mut().unwrap();
    let chat_id = chat.chat_id.clone();
    chat.kind = ChatKind::Group;
    chat.group_id = Some("membership-test".into());
    chat.display_name = "Weekend plans".into();
    chat.participants = vec![
        ChatParticipantSnapshot {
            owner_pubkey_hex: local_owner,
            display_name: "You".into(),
            picture_url: None,
            is_local_owner: true,
        },
        ChatParticipantSnapshot {
            owner_pubkey_hex: PEER.into(),
            display_name: "Alex".into(),
            picture_url: None,
            is_local_owner: false,
        },
    ];
    chat.messages = vec![ChatMessageSnapshot {
        id: "membership-history".into(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: "Alex".into(),
        author_owner_pubkey_hex: Some(PEER.into()),
        author_picture_url: None,
        body: "Meet at the park at noon".into(),
        attachments: vec![],
        reactions: vec![MessageReactionSnapshot {
            emoji: "👍".into(),
            count: 1,
            reacted_by_me: false,
        }],
        reactors: vec![],
        is_outgoing: false,
        created_at_secs: 1,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        recipient_deliveries: vec![],
        delivery_trace: Default::default(),
        source_event_id: None,
        call: None,
    }];
    apply(member.clone());
    let input = text_view(slot.root.upcast_ref()).expect("member composer");
    let buffer = input.buffer();
    let draft = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    let send = button(slot.root.upcast_ref(), "Send").expect("member send");
    let reaction = button(slot.root.upcast_ref(), "👍 1").expect("member reaction");
    assert!(reaction.is_sensitive());
    let staged = manager.staged_attachments(&chat_id);
    assert!(!staged.is_empty());

    let mut removed = member.clone();
    removed
        .current_chat
        .as_mut()
        .unwrap()
        .participants
        .retain(|participant| !participant.is_local_owner);
    apply(removed);
    assert!(find_label(slot.root.upcast_ref(), "You’re no longer in this group").is_some());
    assert!(find_label(slot.root.upcast_ref(), "Meet at the park at noon").is_some());
    assert!(text_view(slot.root.upcast_ref()).is_none());
    assert!(!button(slot.root.upcast_ref(), "👍 1")
        .unwrap()
        .is_sensitive());

    // Retained GTK controls can still emit callbacks after their view is gone.
    send.emit_clicked();
    assert_eq!(
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
        draft
    );
    assert_eq!(manager.staged_attachments(&chat_id), staged);
    assert!(!drop_target.emit_by_name::<bool>(
        "drop",
        &[&glib::BoxedValue(files.to_value()), &0.0f64, &0.0f64]
    ));
    let recent_path = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap())
        .join("iris-chat/recent-reactions.txt");
    let recent = std::fs::read(&recent_path).ok();
    reaction.emit_clicked();
    assert_eq!(std::fs::read(&recent_path).ok(), recent);

    apply(member);
    let restored = text_view(slot.root.upcast_ref()).expect("re-added member composer");
    let restored = restored.buffer();
    assert_eq!(
        restored.text(&restored.start_iter(), &restored.end_iter(), true),
        draft
    );
    assert!(button(slot.root.upcast_ref(), "👍 1")
        .unwrap()
        .is_sensitive());
    assert!(find_label(slot.root.upcast_ref(), "You’re no longer in this group").is_none());
    apply(original);
    println!("PASS: removed group preserves history/draft, blocks stale send/drop/reaction and restores composer on re-add");
}
