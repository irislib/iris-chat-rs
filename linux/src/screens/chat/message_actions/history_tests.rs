use super::*;
use iris_chat_core::{AppUpdate, MessageEditSnapshot};
use std::time::{Duration, Instant};

#[track_caller]
fn pump_until(mut ready: impl FnMut() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if ready() {
            return;
        }
        assert!(Instant::now() < deadline, "edit history UI did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
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

fn button(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Some(button) = root.downcast_ref::<gtk::Button>() {
        if button.label().as_deref() == Some(label) {
            return Some(button.clone());
        }
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = button(&widget, label) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn apply(manager: &AppManager, state: &AppState) {
    let mut state = state.clone();
    state.rev = manager.current_state().rev + 1;
    manager.apply_update(AppUpdate::FullState(state)).unwrap();
}

fn dialog(window: &adw::Window, title: &str) -> adw::Dialog {
    pump_until(|| {
        window.visible_dialog().is_some_and(|dialog| {
            // Controls become usable after the sheet opens, which follows
            // the dialog's own map on libadwaita 1.5.
            dialog.title() == title
                && dialog.is_mapped()
                && dialog.child().is_some_and(|content| content.is_mapped())
        })
    });
    window.visible_dialog().unwrap()
}

fn close(window: &adw::Window) {
    if let Some(dialog) = window.visible_dialog() {
        dialog.close();
    }
    pump_until(|| window.dialogs().n_items() == 0);
}

fn rows(dialog: &adw::Dialog) -> Vec<(String, String)> {
    let versions = find(dialog.upcast_ref(), "editHistoryVersions").unwrap();
    let mut rows = Vec::new();
    let mut child = versions.first_child();
    while let Some(row) = child {
        let heading = row.first_child().unwrap().downcast::<gtk::Label>().unwrap();
        let time = heading
            .next_sibling()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        let body = time
            .next_sibling()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert!(time.is_selectable() && !time.text().is_empty());
        assert!(body.is_selectable());
        rows.push((heading.text().to_string(), body.text().to_string()));
        child = row.next_sibling();
    }
    rows
}

pub fn verify_edit_history_ui(manager: Rc<AppManager>) {
    let original = manager.current_state();
    let mut state = original.clone();
    let chat = state.current_chat.as_mut().expect("history test chat");
    let version = |id: &str, body: &str, seconds| MessageEditSnapshot {
        id: id.into(),
        body: body.into(),
        created_at_secs: seconds,
    };
    let now = unix_now();
    let message = ChatMessageSnapshot {
        id: "edit-history-ui".into(),
        chat_id: chat.chat_id.clone(),
        kind: ChatMessageKind::User,
        author: "Alex".into(),
        author_owner_pubkey_hex: None,
        author_picture_url: None,
        body: "Latest version".into(),
        edit_history: vec![
            version(
                "original",
                "↩ Alex: A quoted message\n\nOriginal version",
                now - 90,
            ),
            version("edit-1", "First edit", now - 60),
            version("edit-2", "Latest version", now - 30),
        ],
        deleted_for_everyone: false,
        system_notice_owner_pubkey_hex: None,
        attachments: vec![],
        direct_transfer: None,
        reactions: vec![],
        reactors: vec![],
        is_outgoing: true,
        created_at_secs: now - 90,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        recipient_deliveries: vec![],
        delivery_trace: Default::default(),
        source_event_id: None,
        call: None,
    };
    chat.messages = vec![message.clone()];
    let chat = chat.clone();
    apply(&manager, &state);
    let window = adw::Window::builder()
        .title("Edit history")
        .default_width(680)
        .default_height(720)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    let menu_button = gtk::MenuButton::builder().label("Message actions").build();
    let menu = build_message_popover(&message, &chat, &manager);
    menu_button.set_popover(Some(&menu));
    content.append(&menu_button);
    let edited = history_button(&message, &manager);
    content.append(&edited);
    window.set_content(Some(&content));
    window.present();
    pump_until(|| window.is_mapped());

    // Exercise the production Info menu and its near-top history action.
    menu.popup();
    pump_until(|| menu.is_mapped());
    button(menu.upcast_ref(), "Info").unwrap().emit_clicked();
    let details = dialog(&window, "Message Details");
    let history_row = find(details.upcast_ref(), "messageInfoEditHistory")
        .unwrap()
        .downcast::<adw::ActionRow>()
        .unwrap();
    let details_scroll = details.child().unwrap();
    pump_until(|| {
        history_row.is_mapped() && history_row.height() > 0 && details_scroll.height() > 0
    });
    let history_bounds = history_row.compute_bounds(&details_scroll).unwrap();
    assert!(
        history_bounds.y() >= 0.0
            && history_bounds.y() < 200.0
            && history_bounds.y() + history_bounds.height() <= details_scroll.height() as f32,
        "history must appear near the top without scrolling: {history_bounds:?}"
    );
    history_row.emit_by_name::<()>("activated", &[]);
    let history = dialog(&window, "Edit history");
    assert_eq!(
        rows(&history),
        vec![
            ("Current".into(), "Latest version".into()),
            ("Edit 1".into(), "First edit".into()),
            ("Original".into(), "Original version".into()),
        ]
    );
    let selected_version = find(history.upcast_ref(), "editHistoryVersion-edit-2").unwrap();
    let selected_text = selected_version
        .last_child()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    pump_until(|| {
        selected_text.is_mapped() && selected_text.width() > 0 && selected_text.height() > 0
    });
    // Mapping precedes initial focus and the Details-to-History transition.
    // Select after they settle, as a user does in the presented history view.
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(500)));
    assert!(selected_text.grab_focus());
    selected_text.select_region(2, 6);
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(100)));
    assert_eq!(
        selected_text.selection_bounds(),
        Some((2, 6)),
        "user selection must be stable before a live update"
    );

    let current = &mut state.current_chat.as_mut().unwrap().messages[0];
    current.body = "Newest version".into();
    current
        .edit_history
        .push(version("edit-3", "Newest version", now));
    apply(&manager, &state);
    pump_until(|| rows(&history).len() == 4);
    assert_eq!(
        rows(&history)[0],
        ("Current".into(), "Newest version".into())
    );
    assert_eq!(rows(&history)[1].0, "Edit 2");
    assert_eq!(
        find(history.upcast_ref(), "editHistoryVersion-edit-2").unwrap(),
        selected_version,
        "a new edit must preserve existing version widgets"
    );
    assert_eq!(selected_text.selection_bounds(), Some((2, 6)));
    assert_eq!(
        gtk::prelude::GtkWindowExt::focus(&window),
        Some(selected_text.clone().upcast()),
        "a new edit must preserve focused version text"
    );
    let saved_versions = state.current_chat.as_ref().unwrap().messages[0]
        .edit_history
        .clone();
    state.current_chat.as_mut().unwrap().messages[0]
        .edit_history
        .retain(|version| version.id != "edit-2");
    apply(&manager, &state);
    pump_until(|| rows(&history).len() == 3);
    assert!(
        gtk::prelude::GtkWindowExt::focus(&window)
            .is_some_and(|focused| focused.is_ancestor(&history)),
        "removing a focused version moves focus within the history dialog"
    );
    state.current_chat.as_mut().unwrap().messages[0].edit_history = saved_versions;
    apply(&manager, &state);
    pump_until(|| rows(&history).len() == 4);
    close(&window);

    // The existing menu and Edited link resolve current state, not their snapshot.
    menu.popup();
    pump_until(|| menu.is_mapped());
    find(menu.upcast_ref(), "messageEditHistoryMenu")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    assert_eq!(
        rows(&dialog(&window, "Edit history"))[0].1,
        "Newest version"
    );
    close(&window);
    edited.emit_clicked();
    let history = dialog(&window, "Edit history");
    assert_eq!(rows(&history).len(), 4);
    screenshot(&window);
    close(&window);

    // Deleted and unedited messages do not advertise history in details.
    for deleted in [false, true] {
        let mut changed = state.clone();
        let current = &mut changed.current_chat.as_mut().unwrap().messages[0];
        current.deleted_for_everyone = deleted;
        if !deleted {
            current.edit_history.clear();
        }
        apply(&manager, &changed);
        present_message_info(Some(window.upcast_ref()), &message, &chat, &manager);
        assert!(find(
            dialog(&window, "Message Details").upcast_ref(),
            "messageInfoEditHistory"
        )
        .is_none());
        close(&window);
        present_history(Some(window.upcast_ref()), &message, &manager);
        assert!(
            window.visible_dialog().is_none(),
            "stale history opener must refuse removed history"
        );
    }

    for change in [
        "deleted",
        "removed",
        "account",
        "chat",
        "screen",
        "logged-out",
    ] {
        apply(&manager, &state);
        present_history(Some(window.upcast_ref()), &message, &manager);
        dialog(&window, "Edit history");
        let mut changed = state.clone();
        match change {
            "deleted" => {
                changed.current_chat.as_mut().unwrap().messages[0].deleted_for_everyone = true
            }
            "removed" => changed.current_chat.as_mut().unwrap().messages.clear(),
            "account" => {
                changed.account.as_mut().unwrap().public_key_hex = "different-account".into()
            }
            "chat" => changed.current_chat.as_mut().unwrap().chat_id = "different-chat".into(),
            "screen" => changed
                .router
                .screen_stack
                .push(iris_chat_core::Screen::Settings),
            "logged-out" => changed.account = None,
            _ => unreachable!(),
        }
        apply(&manager, &changed);
        pump_until(|| window.dialogs().n_items() == 0);
        if change == "account" {
            edited.emit_clicked();
            assert!(window.visible_dialog().is_none());
            menu.popup();
            pump_until(|| menu.is_mapped());
            button(menu.upcast_ref(), "Edit history")
                .unwrap()
                .emit_clicked();
            assert!(window.visible_dialog().is_none());
            menu.popup();
            pump_until(|| menu.is_mapped());
            button(menu.upcast_ref(), "Info").unwrap().emit_clicked();
            assert!(window.visible_dialog().is_none());
        } else {
            present_history(Some(window.upcast_ref()), &message, &manager);
            assert!(
                window.visible_dialog().is_none(),
                "stale {change} opener must refuse history"
            );
        }
        close(&window);
    }

    // An invalidation can precede the sheet's first frame on a slow machine.
    // Start the production watcher before presentation to hold that gap open,
    // then prove its close is retained even if the account/message reappears.
    for change in ["deleted", "account"] {
        apply(&manager, &state);
        let early = adw::Dialog::builder().title("Early history").build();
        early.set_child(Some(&gtk::Label::new(Some("Message history"))));
        watch_message(&early, &message, &manager, |message| {
            !message.deleted_for_everyone
        });
        let mut changed = state.clone();
        if change == "deleted" {
            changed.current_chat.as_mut().unwrap().messages[0].deleted_for_everyone = true;
        } else {
            changed.account.as_mut().unwrap().public_key_hex = "different-account".into();
        }
        apply(&manager, &changed);
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(250)));
        assert!(!early.child().unwrap().is_mapped());
        apply(&manager, &state);
        crate::widgets::dialogs::present(&early, Some(window.upcast_ref()));
        // The dialog leaves the model before its closing animation unmaps it.
        pump_until(|| window.dialogs().n_items() == 0 && !early.is_mapped());
        // Advance beyond the two-frame pending open in libadwaita 1.5 so an
        // early close followed by a reopen cannot satisfy the regression.
        let frames = Rc::new(std::cell::Cell::new(0));
        let tick_frames = frames.clone();
        window.add_tick_callback(move |_, _| {
            tick_frames.set(tick_frames.get() + 1);
            if tick_frames.get() >= 3 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        pump_until(|| frames.get() >= 3);
        assert!(
            window.dialogs().n_items() == 0 && !early.is_mapped(),
            "early {change} must remain dismissed"
        );
    }

    // The local clock expires history without a new core snapshot.
    let mut expiring = state.clone();
    expiring.current_chat.as_mut().unwrap().messages[0].expires_at_secs = Some(unix_now() + 2);
    apply(&manager, &expiring);
    present_history(Some(window.upcast_ref()), &message, &manager);
    dialog(&window, "Edit history");
    let revision = manager.current_state().rev;
    pump_until(|| window.dialogs().n_items() == 0);
    assert_eq!(manager.current_state().rev, revision);
    present_history(Some(window.upcast_ref()), &message, &manager);
    assert!(window.visible_dialog().is_none());
    present_message_info(Some(window.upcast_ref()), &message, &chat, &manager);
    assert!(window.visible_dialog().is_none());

    apply(&manager, &original);
    window.destroy();
    println!("PASS: edit history details/menu/Edited access, newest-first selectable versions, live updates, account/chat/deletion/expiry closure");
}

fn screenshot(window: &adw::Window) {
    let Some(directory) = std::env::var_os("IRIS_LAYOUT_UI_SCREENSHOTS") else {
        return;
    };
    std::fs::create_dir_all(&directory).unwrap();
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(500)));
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
    let node = snapshot.to_node().expect("history screenshot");
    window
        .renderer()
        .unwrap()
        .render_texture(&node, None)
        .save_to_png(std::path::Path::new(&directory).join("linux-edit-history.png"))
        .unwrap();
}
