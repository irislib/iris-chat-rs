use super::*;
use std::time::{Duration, Instant};

const PEER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

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
        assert!(Instant::now() < deadline, "GTK/core update timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn text_view(widget: &gtk::Widget) -> Option<gtk::TextView> {
    if let Ok(input) = widget.clone().downcast::<gtk::TextView>() {
        return Some(input);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(input) = text_view(&widget) {
            return Some(input);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_label(widget: &gtk::Widget, text: &str) -> Option<gtk::Label> {
    if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
        if label.text() == text {
            return Some(label);
        }
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(label) = find_label(&widget, text) {
            return Some(label);
        }
        child = widget.next_sibling();
    }
    None
}

fn header() -> HeaderWidgets {
    let title = gtk::Label::new(None);
    let title_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    title_column.append(&title);
    let title_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    title_slot.append(&title_column);
    HeaderWidgets {
        back: gtk::Button::new(),
        new_chat: gtk::Button::new(),
        settings: gtk::Button::new(),
        chat_info: gtk::Button::new(),
        chat_search: gtk::Button::new(),
        voice_call: gtk::Button::new(),
        video_call: gtk::Button::new(),
        title,
        title_column,
        title_status: gtk::Box::new(gtk::Orientation::Horizontal, 0),
        title_status_icon: gtk::Image::new(),
        title_status_label: gtk::Label::new(None),
        title_slot,
    }
}

pub fn run() {
    let data = tempfile::tempdir().unwrap();
    std::env::set_var("IRIS_UI_TEST_RUN_ID", "linux-composer-regression");
    std::env::set_var("IRIS_UI_TEST_DATA_DIR", data.path());
    std::env::set_var("XDG_CONFIG_HOME", data.path().join("config"));
    std::env::set_var("XDG_DATA_HOME", data.path().join("data"));
    adw::init().expect("GTK display required; run this test under xvfb-run on Linux");
    let manager = Rc::new(AppManager::new());
    crate::calls::verify_ui(manager.clone());
    crate::screens::chat::verify_image_clipboard_ui();
    let rx = manager.update_rx();
    let drain = || {
        while let Ok(update) = rx.try_recv() {
            manager.apply_update(update);
        }
    };
    manager.dispatch(AppAction::CreateAccount {
        name: "Typing test".into(),
    });
    pump_until(|| {
        drain();
        manager.current_state().account.is_some()
    });
    manager.dispatch(AppAction::CreateChat {
        peer_input: PEER.into(),
    });
    pump_until(|| {
        drain();
        manager.current_state().current_chat.is_some()
    });
    let slot = Content::new();
    let header = header();
    let window = adw::Window::builder()
        .default_width(390)
        .default_height(700)
        .build();
    window.set_content(Some(&slot.root));
    let mut state = manager.current_state();
    state.current_chat.as_mut().unwrap().direct_chat_capability = None;
    apply_state(&slot, &header, &manager, &state);
    window.present();
    let input = text_view(slot.root.upcast_ref()).expect("message input");
    pump_until(|| input.has_focus());
    // This is GTK's real committed-text action, used by keyboard/input methods.
    // Do not refocus between characters: that would conceal the reported failure.
    let apply_updates = || {
        while let Ok(update) = rx.try_recv() {
            if let Some(AppUpdate::FullState(mut state)) = manager.apply_update(update) {
                if let Some(chat) = state.current_chat.as_mut() {
                    chat.direct_chat_capability = None;
                }
                apply_state(&slot, &header, &manager, &state);
            }
        }
    };
    let mut expected = String::new();
    for character in "hello 世界 👋".chars() {
        input.emit_by_name::<()>("insert-at-cursor", &[&character.to_string()]);
        expected.push(character);
        pump_until(|| {
            apply_updates();
            manager
                .current_state()
                .current_chat
                .as_ref()
                .is_some_and(|chat| chat.draft == expected)
        });
        assert!(
            input.root().is_some(),
            "typing one character detached the message input"
        );
        assert!(
            input.has_focus(),
            "typing one character lost keyboard focus"
        );
        assert_eq!(text_view(slot.root.upcast_ref()).unwrap(), input);
    }
    let buffer = input.buffer();
    buffer.select_range(&buffer.iter_at_offset(2), &buffer.iter_at_offset(4));
    // An older queued state must not overwrite newer text or selection. These
    // updates also exercise the refresh used by nearby status and busy state.
    state.current_chat.as_mut().unwrap().display_name = "Typing test".into();
    state.current_chat.as_mut().unwrap().message_ttl_seconds = Some(60);
    state.busy.sending_message = true;
    state.busy.uploading_attachment = true;
    apply_state(&slot, &header, &manager, &state);
    assert_eq!(
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
        expected
    );
    let (start, end) = buffer
        .selection_bounds()
        .expect("selection survives updates");
    assert_eq!((start.offset(), end.offset()), (2, 4));
    assert!(input.has_focus());
    state.busy.sending_message = false;
    state.busy.uploading_attachment = false;
    apply_state(&slot, &header, &manager, &state);
    assert!(input.has_focus());

    // Capability checks never replace or resize the live composer.
    let capability_start = Instant::now();
    state.current_chat.as_mut().unwrap().direct_chat_capability =
        Some(iris_chat_core::DirectChatCapabilityState::Checking);
    apply_state(&slot, &header, &manager, &state);
    let checking = find_label(slot.root.upcast_ref(), "Checking messaging…")
        .unwrap()
        .parent()
        .unwrap();
    assert!(!checking.is_visible(), "short checks must stay silent");
    assert_eq!(text_view(slot.root.upcast_ref()).unwrap(), input);
    let before = input.compute_bounds(&slot.root).unwrap();
    pump_until(|| checking.is_visible());
    assert!(capability_start.elapsed() >= Duration::from_secs(2));
    assert_eq!(input.compute_bounds(&slot.root).unwrap(), before);
    assert!(input.has_focus());
    state.current_chat.as_mut().unwrap().direct_chat_capability =
        Some(iris_chat_core::DirectChatCapabilityState::Available);
    apply_state(&slot, &header, &manager, &state);
    assert!(checking.parent().is_none());
    assert_eq!(text_view(slot.root.upcast_ref()).unwrap(), input);
    assert_eq!(input.compute_bounds(&slot.root).unwrap(), before);
    assert!(input.has_focus());

    window.add_css_class("iris-root");
    crate::widgets::text_size::install(&window, data.path());
    let normal_font = input.pango_context().font_description().unwrap().size();
    let keys = window
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .unwrap();
    let handled: bool = keys.emit_by_name(
        "key-pressed",
        &[
            &gtk::gdk::Key::plus,
            &0u32,
            &gtk::gdk::ModifierType::CONTROL_MASK,
        ],
    );
    assert!(handled);
    pump_until(|| input.pango_context().font_description().unwrap().size() > normal_font);
    pump_until(|| {
        std::fs::read_to_string(data.path().join("desktop-zoom.txt"))
            .ok()
            .as_deref()
            == Some("1")
    });
    let _: bool = keys.emit_by_name(
        "key-pressed",
        &[
            &gtk::gdk::Key::_0,
            &0u32,
            &gtk::gdk::ModifierType::CONTROL_MASK,
        ],
    );
    pump_until(|| input.pango_context().font_description().unwrap().size() == normal_font);

    // Leaving and reopening the same chat should restore the saved draft and
    // focus the new editor once, without the old per-chat focus suppression.
    slot.replace(&gtk::Label::new(Some("Another screen")));
    assert!(input.root().is_none());
    state = manager.current_state();
    state.current_chat.as_mut().unwrap().direct_chat_capability = None;
    apply_state(&slot, &header, &manager, &state);
    let reopened = text_view(slot.root.upcast_ref()).unwrap();
    pump_until(|| reopened.has_focus());
    assert_ne!(reopened, input);
    let buffer = reopened.buffer();
    assert_eq!(
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
        expected
    );

    // Exercise the controller installed on the real chat, including drops over
    // the message editor. No network send is dispatched by this GTK signal.
    fn file_drop_target(widget: &gtk::Widget) -> Option<gtk::DropTarget> {
        let controllers = widget.observe_controllers();
        for index in 0..controllers.n_items() {
            if let Some(target) = controllers
                .item(index)
                .and_then(|item| item.downcast::<gtk::DropTarget>().ok())
            {
                return Some(target);
            }
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(target) = file_drop_target(&widget) {
                return Some(target);
            }
            child = widget.next_sibling();
        }
        None
    }
    // Exercise the same serialized target accepted by the GApplication action.
    let notification_chat = manager
        .current_state()
        .current_chat
        .as_ref()
        .unwrap()
        .chat_id
        .clone();
    let owner = manager
        .current_state()
        .account
        .as_ref()
        .unwrap()
        .public_key_hex
        .clone();
    let target = manager.notification_target(&notification_chat).unwrap();
    let payload = serde_json::to_string(&target).unwrap();
    let mut persisted =
        crate::platform::notifications::NotificationRouting::new(manager.app_data_dir());
    assert!(
        persisted.receive(&payload),
        "notification survives process restart"
    );
    let chats = manager.current_state().chat_list;
    assert_eq!(
        persisted.take(None, &chats),
        None,
        "wait for account restore"
    );
    assert_eq!(
        persisted.take(Some(&owner), &[]),
        None,
        "wait for chat restore"
    );
    assert_eq!(
        persisted.take(Some(&owner), &chats),
        Some(notification_chat.clone())
    );
    assert_eq!(
        persisted.take(Some(&owner), &chats),
        None,
        "one-shot navigation"
    );
    assert!(persisted.receive(&payload));
    assert_eq!(persisted.take(Some(&"0".repeat(64)), &chats), None);
    assert_eq!(
        persisted.take(Some(&owner), &chats),
        None,
        "account mismatch consumes target"
    );
    persisted.receive(&payload);
    persisted.invalidate();
    assert!(
        !persisted.receive(&payload),
        "logout invalidates old notification"
    );
    assert!(!persisted.receive("{}"));
    manager.dispatch(AppAction::PushScreen {
        screen: iris_chat_core::Screen::Settings,
    });
    assert!(matches!(
        manager.current_state().router.screen_stack.last(),
        Some(iris_chat_core::Screen::Settings)
    ));
    let actions = gio::SimpleActionGroup::new();
    let manager_for_notification = manager.clone();
    crate::platform::notifications::install_open_chat_action(&actions, move |payload| {
        manager_for_notification.receive_notification_chat(payload);
    });
    actions.activate_action("open-notification-chat", Some(&payload.to_variant()));
    assert_eq!(
        iris_chat_core::router_open_chat_id(manager.current_state().router),
        Some(notification_chat.clone()),
        "production manager dispatches exact chat route"
    );
    println!("PASS: Linux notification warm/cold target, pending auth, one-shot and logout/account guards");

    let target = file_drop_target(slot.root.upcast_ref()).expect("whole-chat file drop target");
    let apply_drop_state = |mut state: AppState| {
        state.rev = manager.current_state().rev + 1;
        manager.apply_update(AppUpdate::FullState(state));
    };
    let mut eligible = manager.current_state();
    let chat_id = eligible.current_chat.as_ref().unwrap().chat_id.clone();
    eligible
        .current_chat
        .as_mut()
        .unwrap()
        .direct_chat_capability = None;
    eligible.current_chat.as_mut().unwrap().is_request = false;
    apply_drop_state(eligible.clone());
    let first = data.path().join("First.txt");
    let second = data.path().join("Second.pdf");
    std::fs::write(&first, "one").unwrap();
    std::fs::write(&second, "two").unwrap();
    let files = gtk::gdk::FileList::from_array(&[
        gio::File::for_path(&first),
        gio::File::for_path(&second),
    ]);
    assert!(target.emit_by_name::<bool>(
        "drop",
        &[&glib::BoxedValue(files.to_value()), &0.0f64, &0.0f64]
    ));
    let staged = manager.staged_attachments(&chat_id);
    assert_eq!(
        staged
            .iter()
            .map(|file| file.filename.as_str())
            .collect::<Vec<_>>(),
        ["First.txt", "Second.pdf"]
    );
    assert_eq!(
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
        expected
    );
    assert!(find_label(slot.root.upcast_ref(), "First.txt").is_some());
    assert_eq!(
        manager
            .current_state()
            .current_chat
            .as_ref()
            .unwrap()
            .messages
            .len(),
        eligible.current_chat.as_ref().unwrap().messages.len(),
        "drop must not send"
    );
    let folders = gtk::gdk::FileList::from_array(&[gio::File::for_path(data.path())]);
    assert!(!target.emit_by_name::<bool>(
        "drop",
        &[&glib::BoxedValue(folders.to_value()), &0.0f64, &0.0f64]
    ));
    let remote =
        gtk::gdk::FileList::from_array(&[gio::File::for_uri("https://example.com/file.pdf")]);
    assert!(!target.emit_by_name::<bool>(
        "drop",
        &[&glib::BoxedValue(remote.to_value()), &0.0f64, &0.0f64]
    ));
    let mut changed = eligible.clone();
    changed.current_chat.as_mut().unwrap().chat_id = "different-chat".into();
    apply_drop_state(changed);
    assert!(!target.emit_by_name::<bool>(
        "drop",
        &[&glib::BoxedValue(files.to_value()), &0.0f64, &0.0f64]
    ));
    let mut request = eligible.clone();
    request.current_chat.as_mut().unwrap().is_request = true;
    apply_drop_state(request);
    assert!(!target.emit_by_name::<bool>(
        "drop",
        &[&glib::BoxedValue(files.to_value()), &0.0f64, &0.0f64]
    ));
    apply_drop_state(eligible);
    println!("PASS: whole-chat file drop stages files, preserves text, rejects folders/URLs/stale chat/request");

    if let Some(path) = std::env::var_os("IRIS_UI_TEST_SCREENSHOT") {
        pump_until(|| window.width() > 0 && window.height() > 0);
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let mut node = None;
        pump_until(|| {
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            node = snapshot.to_node();
            node.is_some()
        });
        let texture = window
            .renderer()
            .unwrap()
            .render_texture(node.as_ref().unwrap(), None);
        texture.save_to_png(path).unwrap();
    }
    // A core update already queued before logout must not restore the old call
    // or allow notifications to use the newly rotated local session.
    let before_logout = manager.current_state();
    manager.logout();
    let mut stale = before_logout.clone();
    stale.rev += 1;
    stale.toast = Some("Queued before logout".into());
    stale.call = Some(iris_chat_core::CallSnapshot {
        outgoing: false,
        target_bitrate_bps: 150_000,
        key_frame_generation: 0,
        media_connected: false,
        max_bitrate_bps: 1_200_000,
        call_id: "stale-logout-call".into(),
        chat_id: notification_chat,
        peer_name: "Alex".into(),
        phase: "incoming".into(),
        video: true,
        video_capable: true,
        muted: false,
        remote_video: true,
        remote_muted: false,
        started_at_secs: 0,
        connected_at_secs: None,
        end_reason: None,
    });
    assert!(
        manager
            .apply_update(AppUpdate::FullState(stale.clone()))
            .is_none(),
        "queued authorized snapshot must not reach call or notification rendering during logout"
    );
    assert_eq!(manager.current_state(), before_logout);
    // Earlier fixture states intentionally advanced beyond real core revisions.
    // Deliver its logged-out acknowledgement at the next accepted revision.
    let mut logged_out = stale;
    logged_out.account = None;
    logged_out.call = None;
    logged_out.toast = None;
    assert!(manager
        .apply_update(AppUpdate::FullState(logged_out))
        .is_some());
    assert!(manager.current_state().account.is_none());
    let mut next_login = before_logout;
    next_login.rev = manager.current_state().rev + 1;
    assert!(manager
        .apply_update(AppUpdate::FullState(next_login))
        .is_some());
    assert!(
        manager.current_state().account.is_some(),
        "logout acknowledgement permits a later login"
    );
    println!("PASS: logout ignores queued authorized state until logged-out acknowledgement");
    // A verified revocation uses that same logout path and explains it only
    // once the core acknowledges that the session has ended.
    let mut revoked = manager.current_state();
    revoked.rev += 1;
    revoked.account.as_mut().unwrap().authorization_state =
        iris_chat_core::DeviceAuthorizationState::Revoked;
    manager.apply_update(AppUpdate::FullState(revoked.clone()));
    let mut cleared = revoked;
    cleared.rev += 1;
    cleared.account = None;
    cleared.call = None;
    cleared.toast = None;
    manager.apply_update(AppUpdate::FullState(cleared.clone()));
    assert_eq!(
        manager.current_state().toast.as_deref(),
        Some("This device was removed. You’ve been logged out.")
    );
    cleared.rev += 1;
    manager.apply_update(AppUpdate::FullState(cleared));
    assert!(
        manager.current_state().toast.is_none(),
        "removal notice must not repeat on later snapshots"
    );
    println!("PASS: device removal uses normal logout and explains the completed session removal");
    window.close();
    println!("PASS: message input survives a real persisted-draft update with focus intact");
}
