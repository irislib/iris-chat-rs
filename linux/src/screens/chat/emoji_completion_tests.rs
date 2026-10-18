use super::emoji_completion::EmojiCompletion;
use crate::widgets::clickable::PointerCursorExt;
use gtk::{gdk, glib, prelude::*};
use std::time::{Duration, Instant};

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
        assert!(Instant::now() < deadline, "Emoji completion did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn text(buffer: &gtk::TextBuffer) -> String {
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, true).to_string()
}

pub fn verify_ui() {
    let window = gtk::Window::new();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let input = gtk::TextView::new();
    let buffer = input.buffer();
    buffer.set_enable_undo(true);
    let completion = EmojiCompletion::new(&input, &root);
    root.append(&input);
    let button = gtk::Button::with_label("Action");
    button.show_pointer_cursor();
    root.append(&button);
    window.set_child(Some(&root));
    window.present();
    input.grab_focus();
    pump_until(|| input.has_focus());

    let before = input.compute_bounds(&root).unwrap();
    buffer.begin_user_action();
    buffer.insert_at_cursor("hi :sm");
    buffer.end_user_action();
    pump_until(|| completion.root.is_visible());
    assert!(
        input.has_focus(),
        "emoji suggestions must retain editor focus"
    );
    let after = input.compute_bounds(&root).unwrap();
    assert_eq!(
        before.y(),
        after.y(),
        "emoji suggestions must not move the editor"
    );
    assert_eq!(
        before.height(),
        after.height(),
        "emoji suggestions must not resize the editor"
    );
    super::completion_screenshots::save_widget(
        &completion
            .root
            .ancestor(gtk::Popover::static_type())
            .unwrap(),
        "linux-emoji-popup",
    );
    super::completion_screenshots::save(&window, "linux-emoji-suggestions");
    assert!(completion.handle_key(gdk::Key::Down, gdk::ModifierType::empty()));
    assert!(completion.handle_key(gdk::Key::Return, gdk::ModifierType::empty()));
    assert!(text(&buffer).starts_with("hi "));
    assert!(!text(&buffer).contains(":sm"));
    buffer.undo();
    pump_until(|| text(&buffer) == "hi :sm");
    assert!(!completion.root.is_visible());
    assert!(!completion.handle_key(gdk::Key::Down, gdk::ModifierType::empty()));

    buffer.set_text(":sm");
    pump_until(|| !completion.root.is_visible());
    buffer.set_text("");
    buffer.begin_user_action();
    buffer.insert_at_cursor(":fire:");
    buffer.end_user_action();
    pump_until(|| text(&buffer) == "🔥");
    buffer.undo();
    pump_until(|| text(&buffer) == ":fire:");
    assert!(!completion.root.is_visible());

    assert_eq!(
        button.cursor().and_then(|cursor| cursor.name()).as_deref(),
        Some("pointer")
    );
    root.set_sensitive(false);
    assert_eq!(
        button.cursor().and_then(|cursor| cursor.name()).as_deref(),
        Some("default")
    );
    root.set_sensitive(true);
    assert_eq!(
        button.cursor().and_then(|cursor| cursor.name()).as_deref(),
        Some("pointer")
    );
    window.close();
    println!("PASS: typed emoji selection, undo, restored drafts and disabled cursors");
}
