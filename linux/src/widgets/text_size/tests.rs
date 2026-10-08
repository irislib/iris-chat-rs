use super::*;
use crate::app_manager::AppManager;
use iris_chat_core::{ChatMessageKind, ChatMessageSnapshot, DeliveryState};
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
        assert!(Instant::now() < deadline, "text size UI timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn find(widget: &gtk::Widget, matches: &impl Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if matches(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = find(&widget, matches) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn font_size(widget: &impl IsA<gtk::Widget>) -> i32 {
    widget.pango_context().font_description().unwrap().size()
}

fn shortcut(keys: &gtk::EventControllerKey, key: gdk::Key) {
    let handled: bool = keys.emit_by_name(
        "key-pressed",
        &[&key, &0u32, &gdk::ModifierType::CONTROL_MASK],
    );
    assert!(handled, "text size shortcut must remain available");
}

pub fn verify_ui(manager: Rc<AppManager>) {
    let data = tempfile::tempdir().unwrap();
    {
        let mut state = manager.current_state();
        let chat = state
            .current_chat
            .as_mut()
            .expect("open chat for text size test");
        chat.direct_chat_capability = None;
        chat.messages = vec![ChatMessageSnapshot {
            id: "text-size-message".into(),
            chat_id: chat.chat_id.clone(),
            kind: ChatMessageKind::User,
            author: "Alex".into(),
            author_owner_pubkey_hex: None,
            author_picture_url: None,
            body: "Readable message text".into(),
            edit_history: vec![],
            deleted_for_everyone: false,
            system_notice_owner_pubkey_hex: None,
            attachments: vec![],
            direct_transfer: None,
            reactions: vec![],
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
        let mut view = crate::screens::chat::ChatView::new(&chat.chat_id);
        view.update(&state, &manager);
        let row = settings_row(data.path());
        assert_eq!(row.selected(), 3, "Default is the system font size");
        let preferences = adw::PreferencesGroup::new();
        preferences.add(&row);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.add_css_class("iris-root");
        content.append(&preferences);
        content.append(&view.root);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_css_class("iris-root");
        toolbar.set_content(Some(&content));
        let window = adw::Window::builder()
            .default_width(700)
            .default_height(600)
            .build();
        window.add_css_class("iris-root");
        window.set_content(Some(&toolbar));
        install(&window, data.path());
        window.present();
        let input = find(view.root.upcast_ref(), &|widget| {
            widget.widget_name() == "iris-chat-composer"
        })
        .unwrap();
        let message = find(view.root.upcast_ref(), &|widget| {
            widget
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == "Readable message text")
        })
        .unwrap();
        pump_until(|| input.is_mapped() && message.is_mapped());
        let normal_input = font_size(&input);
        let normal_message = font_size(&message);
        let keys = window
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
            .find(|controller| controller.name().as_deref() == Some("iris-text-size-shortcuts"))
            .unwrap();

        row.set_selected(4);
        pump_until(|| font_size(&input) > normal_input && font_size(&message) > normal_message);
        for (widget, normal) in [(&input, normal_input), (&message, normal_message)] {
            let scale = f64::from(font_size(widget)) / f64::from(normal);
            assert!(
                (scale - 1.2).abs() < 0.06,
                "nested panels must scale only once: actual ratio {scale}"
            );
        }
        shortcut(&keys, gdk::Key::minus);
        pump_until(|| row.selected() == 3 && font_size(&input) == normal_input);
        shortcut(&keys, gdk::Key::plus);
        pump_until(|| row.selected() == 4);
        shortcut(&keys, gdk::Key::_0);
        pump_until(|| row.selected() == 3 && font_size(&message) == normal_message);
        row.set_selected(5);
        pump_until(|| {
            std::fs::read_to_string(data.path().join("desktop-zoom.txt"))
                .ok()
                .as_deref()
                == Some("2")
        });
        window.close();
    }
    pump_until(|| {
        MODELS.with(|models| {
            models
                .borrow()
                .get(data.path())
                .and_then(Weak::upgrade)
                .is_none()
        })
    });
    let restored = settings_row(data.path());
    assert_eq!(
        restored.selected(),
        5,
        "Settings must restore the saved text size"
    );
}
