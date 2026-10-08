use super::*;
use std::time::{Duration, Instant};

pub fn verify_ui(manager: Rc<AppManager>) {
    let state = manager.current_state();
    let chat = state
        .current_chat
        .as_ref()
        .expect("current chat for hover test");
    let mut message = ChatMessageSnapshot {
        id: "hover-actions".into(),
        chat_id: chat.chat_id.clone(),
        kind: ChatMessageKind::User,
        author: "Alex".into(),
        author_owner_pubkey_hex: None,
        author_picture_url: None,
        body: "See you at the park!".into(),
        edit_history: vec![],
        deleted_for_everyone: false,
        system_notice_owner_pubkey_hex: None,
        attachments: vec![],
        direct_transfer: None,
        reactions: vec![],
        reactors: vec![],
        is_outgoing: false,
        created_at_secs: unix_now(),
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        recipient_deliveries: vec![],
        delivery_trace: Default::default(),
        source_event_id: None,
        call: None,
    };
    let incoming = render_message(
        &message,
        chat,
        true,
        true,
        true,
        unix_now(),
        &state.preferences,
        &manager,
    );
    message.id = "hover-outgoing".into();
    message.is_outgoing = true;
    message.body = "Sounds good, I'll be there in ten minutes.".into();
    let outgoing = render_message(
        &message,
        chat,
        true,
        true,
        true,
        unix_now(),
        &state.preferences,
        &manager,
    );
    message.id = "hover-deleted".into();
    message.deleted_for_everyone = true;
    let deleted = render_message(
        &message,
        chat,
        true,
        true,
        true,
        unix_now(),
        &state.preferences,
        &manager,
    );
    assert!(find(&deleted, "messageReactButton").is_none());
    assert!(find(&deleted, "messageMoreButton").is_some());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);
    content.append(&incoming);
    content.append(&outgoing);
    content.append(&deleted);
    let editor = gtk::Entry::new();
    editor.set_placeholder_text(Some("Message"));
    content.append(&editor);
    let window = adw::Window::builder()
        .title("Message actions")
        .default_width(860)
        .default_height(340)
        .build();
    window.set_content(Some(&content));
    window.present();
    editor.grab_focus();
    settle();

    for row in [&incoming, &outgoing] {
        let dock = find(row, "messageActionDock").unwrap();
        let menu = find(row, "messageActionsMenu")
            .unwrap()
            .downcast::<gtk::Popover>()
            .unwrap();
        let bubble = menu.parent().unwrap();
        let motion = controller::<gtk::EventControllerMotion>(row);
        motion.emit_by_name::<()>("leave", &[]);
        settle();
        assert!(!dock.is_visible(), "actions stay hidden while typing");
        let bounds = bubble.compute_bounds(&content).unwrap();
        let row_height = row.height();
        motion.emit_by_name::<()>("enter", &[&4.0f64, &4.0f64]);
        settle();
        assert!(dock.is_visible(), "hover reveals the message actions");
        assert_eq!(bubble.compute_bounds(&content).unwrap(), bounds);
        assert_eq!(row.height(), row_height, "hover must not move messages");

        find(row, "messageMoreButton")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        settle();
        assert!(menu.is_visible(), "More opens the existing action menu");
        motion.emit_by_name::<()>("leave", &[]);
        settle();
        assert!(dock.is_visible(), "open menu retains its action dock");
        menu.popdown();
        editor.grab_focus();
        settle();
        assert!(!dock.is_visible());

        let react = find(row, "messageReactButton")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        motion.emit_by_name::<()>("enter", &[&4.0f64, &4.0f64]);
        react.emit_clicked();
        settle();
        let picker = find(row, "messageReactionPicker")
            .unwrap()
            .downcast::<gtk::Popover>()
            .unwrap();
        assert!(picker.is_visible(), "React opens the production picker");
        motion.emit_by_name::<()>("leave", &[]);
        settle();
        assert!(dock.is_visible(), "pointer can travel into the picker");
        picker.popdown();
        editor.grab_focus();
        settle();

        let right_click = controller::<gtk::GestureClick>(&bubble);
        assert_eq!(right_click.button(), 3);
        right_click.emit_by_name::<()>("pressed", &[&1i32, &4.0f64, &4.0f64]);
        settle();
        assert!(menu.is_visible(), "right-click remains available");
        menu.popdown();
        editor.grab_focus();
        settle();
        let long_press = controller::<gtk::GestureLongPress>(&bubble);
        assert!(
            long_press.is_touch_only(),
            "mouse text selection is preserved"
        );
        long_press.emit_by_name::<()>("pressed", &[&4.0f64, &4.0f64]);
        settle();
        assert!(menu.is_visible(), "touch long press remains available");
        menu.popdown();
        editor.grab_focus();
        settle();

        bubble.grab_focus();
        settle();
        assert!(dock.is_visible(), "keyboard focus reveals the same actions");
        let keys = controller::<gtk::EventControllerKey>(&bubble);
        let handled: bool = keys.emit_by_name(
            "key-pressed",
            &[
                &gtk::gdk::Key::F10,
                &0u32,
                &gtk::gdk::ModifierType::SHIFT_MASK,
            ],
        );
        settle();
        assert!(
            handled && menu.is_visible(),
            "Shift+F10 opens message actions"
        );
        menu.popdown();
        editor.grab_focus();
        settle();
    }

    // Narrow layouts keep the bubble geometry when hover is toggled too.
    window.set_default_width(390);
    settle();
    let menu = find(&incoming, "messageActionsMenu").unwrap();
    let bubble = menu.parent().unwrap();
    let before = bubble.compute_bounds(&content).unwrap();
    let motion = controller::<gtk::EventControllerMotion>(&incoming);
    motion.emit_by_name::<()>("enter", &[&4.0f64, &4.0f64]);
    settle();
    assert_eq!(bubble.compute_bounds(&content).unwrap(), before);
    motion.emit_by_name::<()>("leave", &[]);
    window.set_default_width(860);
    settle();
    let outgoing_motion = controller::<gtk::EventControllerMotion>(&outgoing);
    outgoing_motion.emit_by_name::<()>("enter", &[&4.0f64, &4.0f64]);
    settle();
    if let Some(path) = std::env::var_os("IRIS_HOVER_UI_SCREENSHOT") {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        let node = snapshot.to_node().expect("rendered hover actions");
        let renderer = window.renderer().expect("window renderer");
        renderer
            .render_texture(&node, None)
            .save_to_png(path)
            .expect("save hover actions screenshot");
    }
    window.close();
    settle();
    println!("PASS: message hover actions, stable desktop/mobile geometry, reaction/menu continuity, keyboard, right-click and touch access");
}

fn find(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn controller<T: IsA<gtk::EventController> + IsA<glib::Object> + glib::object::IsClass>(
    widget: &gtk::Widget,
) -> T {
    widget
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<T>().ok())
        .expect("production message controller")
}

fn settle() {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_millis(180);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
