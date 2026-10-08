use super::*;
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
        assert!(Instant::now() < deadline, "settings layout did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn named(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = named(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn select(view: &SettingsView, category: &str) {
    named(
        view.root.upcast_ref(),
        &format!("settings-category-{category}"),
    )
    .unwrap()
    .emit_by_name::<()>("activated", &[]);
}

fn screenshot(window: &adw::Window, filename: &str) {
    let Some(directory) = std::env::var_os("IRIS_LAYOUT_UI_SCREENSHOTS") else {
        return;
    };
    std::fs::create_dir_all(&directory).unwrap();
    // Mapping happens before the navigation spring has finished sliding.
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(500)));
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let mut node = None;
    pump_until(|| {
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        node = snapshot.to_node();
        node.is_some()
    });
    window
        .renderer()
        .unwrap()
        .render_texture(node.as_ref().unwrap(), None)
        .save_to_png(std::path::Path::new(&directory).join(filename))
        .unwrap();
}

pub fn verify_ui(manager: Rc<AppManager>) {
    let mut state = manager.current_state();
    let original_preferences = state.preferences.clone();
    let mut view = SettingsView::new(&state, &manager);
    let window = adw::Window::builder()
        .default_width(900)
        .default_height(740)
        .build();
    window.add_css_class("iris-root");
    window.set_content(Some(&view.root));
    window.present();
    pump_until(|| view.root.is_mapped() && window.width() >= 850);
    assert!(!view.split.is_collapsed());
    select(&view, "media");
    let url = named(view.root.upcast_ref(), "settings-image-proxy-url")
        .unwrap()
        .downcast::<adw::EntryRow>()
        .unwrap();
    let enabled = named(view.root.upcast_ref(), "settings-image-proxy-enabled")
        .unwrap()
        .downcast::<adw::SwitchRow>()
        .unwrap();
    let fallback = named(view.root.upcast_ref(), "settings-image-proxy-fallback")
        .unwrap()
        .downcast::<adw::SwitchRow>()
        .unwrap();
    assert_eq!(enabled.is_active(), state.preferences.image_proxy_enabled);
    assert_eq!(
        fallback.is_active(),
        state.preferences.image_proxy_fallback_enabled
    );
    assert_eq!(fallback.is_sensitive(), enabled.is_active());
    assert!(url.shows_apply_button());
    for name in ["settings-image-proxy-key", "settings-image-proxy-salt"] {
        let field = named(view.root.upcast_ref(), name)
            .unwrap()
            .downcast::<adw::PasswordEntryRow>()
            .expect("signing values stay masked");
        assert!(field.shows_apply_button());
        assert!(
            !field.text().is_empty(),
            "saved signing values should be loaded"
        );
    }
    pump_until(|| url.is_mapped());
    url.grab_focus();
    url.set_text("https://images.example/unfinished");
    url.select_region(8, 14);
    let devices = view.stack.child_by_name("devices").unwrap();
    state.busy.syncing_network = !state.busy.syncing_network;
    state.busy.updating_roster = !state.busy.updating_roster;
    view.update(&state, &manager);
    assert_eq!(view.stack.visible_child_name().as_deref(), Some("media"));
    assert_eq!(
        named(view.root.upcast_ref(), "settings-image-proxy-url").unwrap(),
        url.clone().upcast::<gtk::Widget>()
    );
    assert_eq!(url.text(), "https://images.example/unfinished");
    assert_eq!(url.selection_bounds(), Some((8, 14)));
    assert_ne!(
        devices,
        view.stack.child_by_name("devices").unwrap(),
        "device state must still refresh"
    );

    let key = named(view.root.upcast_ref(), "settings-image-proxy-key")
        .unwrap()
        .downcast::<adw::PasswordEntryRow>()
        .unwrap();
    let salt = named(view.root.upcast_ref(), "settings-image-proxy-salt")
        .unwrap()
        .downcast::<adw::PasswordEntryRow>()
        .unwrap();
    key.set_text("aabbccdd");
    salt.set_text("11223344");
    key.grab_focus();
    key.select_region(2, 6);
    let rx = manager.update_rx();
    manager.dispatch(AppAction::SetTypingIndicatorsEnabled {
        enabled: !original_preferences.send_typing_indicators,
    });
    pump_until(|| {
        while let Ok(update) = rx.try_recv() {
            if let Some(iris_chat_core::AppUpdate::FullState(state)) = manager.apply_update(update)
            {
                view.update(&state, &manager);
            }
        }
        manager.current_state().preferences.send_typing_indicators
            != original_preferences.send_typing_indicators
    });
    assert_eq!(url.text(), "https://images.example/unfinished");
    assert_eq!(key.text(), "aabbccdd");
    assert_eq!(salt.text(), "11223344");
    assert_eq!(key.selection_bounds(), Some((2, 6)));
    url.emit_by_name::<()>("apply", &[]);
    pump_until(|| {
        while let Ok(update) = rx.try_recv() {
            if let Some(iris_chat_core::AppUpdate::FullState(state)) = manager.apply_update(update)
            {
                view.update(&state, &manager);
            }
        }
        manager.current_state().preferences.image_proxy_url == "https://images.example/unfinished"
    });
    assert_eq!(
        named(view.root.upcast_ref(), "settings-image-proxy-key").unwrap(),
        key.clone().upcast::<gtk::Widget>()
    );
    assert_eq!(
        key.text(),
        "aabbccdd",
        "applying URL must preserve the key draft"
    );
    assert_eq!(
        salt.text(),
        "11223344",
        "applying URL must preserve the salt draft"
    );
    assert_eq!(key.selection_bounds(), Some((2, 6)));
    assert!(
        gtk::prelude::GtkWindowExt::focus(&window).is_some_and(|focused| focused.is_ancestor(&key))
    );

    manager.dispatch(AppAction::SetImageProxyUrl {
        url: original_preferences.image_proxy_url.clone(),
    });
    manager.dispatch(AppAction::SetTypingIndicatorsEnabled {
        enabled: original_preferences.send_typing_indicators,
    });
    pump_until(|| {
        while let Ok(update) = rx.try_recv() {
            if let Some(iris_chat_core::AppUpdate::FullState(state)) = manager.apply_update(update)
            {
                view.update(&state, &manager);
            }
        }
        let prefs = manager.current_state().preferences;
        prefs.image_proxy_url == original_preferences.image_proxy_url
            && prefs.send_typing_indicators == original_preferences.send_typing_indicators
    });
    screenshot(&window, "linux-settings-media-wide.png");
    window.set_default_size(390, 740);
    pump_until(|| window.width() < 450 && view.split.is_collapsed() && url.is_mapped());
    assert!(url.compute_bounds(&window).unwrap().width() <= window.width() as f32);
    let detail_scroll = named(view.root.upcast_ref(), "settings-detail-scroll").unwrap();
    assert!(
        detail_scroll.height() > window.height() * 3 / 4,
        "settings detail must use the available height"
    );
    assert_eq!(view.stack.visible_child_name().as_deref(), Some("media"));
    screenshot(&window, "linux-settings-media-narrow.png");
    view.split.set_show_content(false);
    pump_until(|| view.menu.is_mapped());
    select(&view, "general");
    let text_size =
        named(view.root.upcast_ref(), "textSizeSetting").expect("visible text-size setting");
    pump_until(|| text_size.is_mapped());
    screenshot(&window, "linux-settings-general-narrow.png");
    let target = "a".repeat(64);
    manager.dispatch(AppAction::SetUserBlocked {
        owner_pubkey_hex: target.clone(),
        blocked: true,
    });
    pump_until(|| {
        while let Ok(update) = rx.try_recv() {
            if let Some(iris_chat_core::AppUpdate::FullState(state)) = manager.apply_update(update)
            {
                view.update(&state, &manager);
            }
        }
        manager.current_state().blocked_people.len() == 1
    });
    view.split.set_show_content(false);
    pump_until(|| view.menu.is_mapped());
    select(&view, "messaging");
    let unblock = named(
        view.root.upcast_ref(),
        &format!("settings-unblock-{target}"),
    )
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    pump_until(|| unblock.is_mapped());
    screenshot(&window, "linux-settings-blocked-narrow.png");
    window.set_default_size(900, 740);
    pump_until(|| window.width() >= 850 && !view.split.is_collapsed());
    screenshot(&window, "linux-settings-blocked-wide.png");
    unblock.emit_clicked();
    pump_until(|| {
        while let Ok(update) = rx.try_recv() {
            if let Some(iris_chat_core::AppUpdate::FullState(state)) = manager.apply_update(update)
            {
                view.update(&state, &manager);
            }
        }
        manager.current_state().blocked_people.is_empty()
    });
    assert!(named(
        view.root.upcast_ref(),
        &format!("settings-unblock-{target}")
    )
    .is_none());
    drop(unblock);
    drop(detail_scroll);
    let weak_root = view.root.downgrade();
    // This test selects editor text. Release Linux's primary-selection owner
    // before testing widget disposal, so clipboard ownership is independent.
    gtk::prelude::WidgetExt::display(&window)
        .primary_clipboard()
        .set_content(None::<&gtk::gdk::ContentProvider>)
        .unwrap();
    window.set_content(None::<&gtk::Widget>);
    drop(view);
    window.destroy();
    // Native controls may retain their rooted navigation context while an
    // external caller still owns them. Release the test's handles too.
    drop((
        url, key, salt, enabled, fallback, devices, text_size, window,
    ));
    // GTK can retain native scrollable navigation pages beyond their container.
    // Verify that Iris releases its owning Settings widget after navigation.
    pump_until(|| weak_root.upgrade().is_none());
}
