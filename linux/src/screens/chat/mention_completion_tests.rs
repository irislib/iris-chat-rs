use super::{mention_completion::MentionCompletion, mention_draft::MentionDraft};
use gtk::{gdk, glib, prelude::*};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

#[track_caller]
fn pump_until(mut ready: impl FnMut() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if ready() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Mention completion did not settle"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn text(buffer: &gtk::TextBuffer) -> String {
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, true).to_string()
}
pub fn verify_ui(manager: Rc<crate::app_manager::AppManager>) {
    let window = gtk::Window::new();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let input = gtk::TextView::new();
    let buffer = input.buffer();
    buffer.set_enable_undo(true);
    let draft = Rc::new(RefCell::new(MentionDraft::default()));
    super::mention_editor::install(&buffer, draft.clone(), |_| {});
    let changed = draft.clone();
    buffer.connect_changed(move |buffer| changed.borrow_mut().changed(&text(buffer)));
    let selected = draft.clone();
    let selected_buffer = buffer.clone();
    let completion = MentionCompletion::new(&input, &root, move |offset, label, wire| {
        let value = text(&selected_buffer);
        let byte = value
            .char_indices()
            .nth(offset as usize)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        selected.borrow_mut().insert(byte, label, wire);
    });
    let first = "1".repeat(64);
    let second = "2".repeat(64);
    completion.set_people(
        [first.clone(), second.clone()]
            .into_iter()
            .map(|owner_pubkey_hex| iris_chat_core::ChatParticipantSnapshot {
                social_connection: None,
                owner_pubkey_hex,
                display_name: "Alex".into(),
                picture_url: None,
                is_local_owner: false,
            })
            .collect(),
    );
    root.append(&input);
    window.set_child(Some(&root));
    window.present();
    input.grab_focus();
    pump_until(|| input.has_focus());
    let before = input.compute_bounds(&root).unwrap();
    buffer.begin_user_action();
    buffer.insert_at_cursor("👋 @");
    buffer.end_user_action();
    // Accept immediately, before the scheduled completion update runs.
    assert!(completion.handle_key(gdk::Key::Down, gdk::ModifierType::empty()));
    assert!(completion.handle_key(gdk::Key::Return, gdk::ModifierType::empty()));
    assert_eq!(text(&buffer), "👋 @Alex ");
    assert_eq!(
        iris_chat_core::message_mentions(draft.borrow().wire_text())[0].owner_pubkey_hex,
        second
    );
    buffer.undo();
    pump_until(|| text(&buffer) == "👋 @");
    assert!(!completion.root.is_visible());
    assert!(!completion.handle_key(gdk::Key::Down, gdk::ModifierType::empty()));
    buffer.redo();
    pump_until(|| text(&buffer) == "👋 @Alex ");
    assert_eq!(
        iris_chat_core::message_mentions(draft.borrow().wire_text())[0].owner_pubkey_hex,
        second
    );
    buffer.begin_user_action();
    buffer.insert_at_cursor("@");
    buffer.end_user_action();
    pump_until(|| completion.root.is_visible());
    assert!(
        input.has_focus(),
        "opening suggestions must keep editor focus"
    );
    let after = input.compute_bounds(&root).unwrap();
    assert_eq!(
        before.y(),
        after.y(),
        "mention popup must not move the composer"
    );
    super::completion_screenshots::save(&window, "linux-mention-suggestions");
    super::completion_screenshots::save_widget(
        &completion
            .root
            .ancestor(gtk::Popover::static_type())
            .unwrap(),
        "linux-mention-popup",
    );
    assert!(completion.handle_key(gdk::Key::Return, gdk::ModifierType::empty()));
    assert_eq!(text(&buffer), "👋 @Alex @Alex ");
    let both = draft.borrow().wire_text();
    buffer.begin_user_action();
    buffer.delete(&mut buffer.iter_at_offset(2), &mut buffer.iter_at_offset(8));
    buffer.end_user_action();
    assert_eq!(
        iris_chat_core::message_mentions(draft.borrow().wire_text())[0].owner_pubkey_hex,
        first
    );
    buffer.undo();
    pump_until(|| text(&buffer) == "👋 @Alex @Alex ");
    assert_eq!(draft.borrow().wire_text(), both);
    buffer.redo();
    pump_until(|| text(&buffer) == "👋 @Alex ");
    assert_eq!(
        iris_chat_core::message_mentions(draft.borrow().wire_text())[0].owner_pubkey_hex,
        first
    );
    buffer.begin_user_action();
    buffer.delete(&mut buffer.iter_at_offset(3), &mut buffer.iter_at_offset(4));
    buffer.end_user_action();
    assert!(iris_chat_core::message_mentions(draft.borrow().wire_text()).is_empty());
    buffer.undo();
    pump_until(|| text(&buffer) == "👋 @Alex ");
    assert_eq!(
        iris_chat_core::message_mentions(draft.borrow().wire_text())[0].owner_pubkey_hex,
        first
    );
    buffer.set_text("@");
    pump_until(|| !completion.root.is_visible());
    buffer.begin_user_action();
    buffer.insert_at_cursor("A");
    buffer.end_user_action();
    pump_until(|| completion.root.is_visible());
    assert!(completion.handle_key(gdk::Key::Escape, gdk::ModifierType::empty()));
    assert_eq!(text(&buffer), "@A");
    completion.set_people(
        (1..=12)
            .map(|index| iris_chat_core::ChatParticipantSnapshot {
                social_connection: None,
                owner_pubkey_hex: format!("{index:064x}"),
                display_name: format!("Person {index}"),
                picture_url: None,
                is_local_owner: false,
            })
            .collect(),
    );
    buffer.set_text("");
    buffer.begin_user_action();
    buffer.insert_at_cursor("@");
    buffer.end_user_action();
    for _ in 0..11 {
        assert!(completion.handle_key(gdk::Key::Down, gdk::ModifierType::empty()));
    }
    assert!(completion.handle_key(gdk::Key::Return, gdk::ModifierType::empty()));
    assert_eq!(text(&buffer), "@Person 12 ");
    assert_eq!(
        iris_chat_core::message_mentions(draft.borrow().wire_text())[0].owner_pubkey_hex,
        format!("{:064x}", 12)
    );
    let token = iris_chat_core::format_message_mention(first, "Old name".into());
    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bubble.add_css_class("bubble-in");
    bubble.set_valign(gtk::Align::Start);
    super::append_truncatable_body(
        &bubble,
        &format!("Hello {token} https://iris.to!"),
        &manager,
    );
    let label = bubble
        .first_child()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    window.set_child(Some(&bubble));
    label.select_region(0, -1);
    assert_eq!(label.text(), "Hello @Old name https://iris.to!");
    assert!(label.selection_bounds().is_some());
    label.select_region(0, 0);
    super::completion_screenshots::save(&window, "linux-mentions-links");
    bubble.remove_css_class("bubble-in");
    bubble.add_css_class("bubble-out");
    super::completion_screenshots::save(&window, "linux-outgoing-mentions-links");
    window.close();
    println!("PASS: bare @, exact same-name identity, Unicode edits, keyboard acceptance, undo and dismissal");
}
