use super::*;
use iris_chat_core::{
    DesktopNearbyPeerSnapshot, DesktopNearbySnapshot, SocialBadge, SocialConnectionSnapshot,
};
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

fn find_badge(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if widget.has_css_class(class) {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

fn badge(widget: &gtk::Widget, class: &str) -> gtk::Widget {
    find_badge(widget, class).unwrap_or_else(|| panic!("missing {class}"))
}

pub fn run() {
    let data = tempfile::tempdir().unwrap();
    std::env::set_var("IRIS_UI_TEST_RUN_ID", "nearby-avatar");
    std::env::set_var("IRIS_UI_TEST_DATA_DIR", data.path());
    std::env::set_var("IRIS_DEMO_RELAYS", "ws://127.0.0.1:9");
    std::env::set_var("IRIS_FIPS_WEBSOCKET_SEED_URLS", "");
    std::env::set_var("XDG_CONFIG_HOME", data.path().join("config"));
    std::env::set_var("XDG_DATA_HOME", data.path().join("data"));
    adw::init().expect("GTK display required");
    let manager = Rc::new(AppManager::new());
    let rx = manager.update_rx();
    let drain = || {
        while let Ok(update) = rx.try_recv() {
            manager.apply_update(update);
        }
    };
    manager.dispatch(AppAction::CreateAccount {
        name: "Nearby test".into(),
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
        manager
            .current_state()
            .current_chat
            .is_some_and(|chat| chat.chat_id == PEER)
    });
    let mut state = manager.current_state();
    state.preferences.nearby_enabled = true;
    state.preferences.nearby_show_in_chat_list = false;
    state.rev += 1;
    manager.apply_update(AppUpdate::FullState(state.clone()));
    let connection = SocialConnectionSnapshot {
        is_favorite: true,
        badge: Some(SocialBadge::Following),
        follow_distance: Some(1),
        followed_by_friends: 0,
        description: "Following".into(),
    };
    state
        .chat_list
        .iter_mut()
        .find(|chat| chat.chat_id == PEER)
        .unwrap()
        .social_connection = Some(connection.clone());
    state.rev += 1;
    manager.apply_update(AppUpdate::FullState(state.clone()));
    assert!(
        manager
            .contact_social_connection(&PEER.to_uppercase())
            .unwrap()
            .is_favorite
    );
    assert!(manager.contact_social_connection("unknown").is_none());
    state
        .chat_list
        .iter_mut()
        .find(|chat| chat.chat_id == PEER)
        .unwrap()
        .social_connection = None;
    state.rev += 1;
    manager.apply_update(AppUpdate::FullState(state.clone()));
    assert!(manager.contact_social_connection(PEER).is_none());
    let avatar = crate::widgets::social_badge::user_avatar(
        &adw::Avatar::new(64, Some("Alex"), true),
        Some(&connection),
        PEER,
        &manager,
    );
    let mark = badge(avatar.upcast_ref(), "nearby-avatar-badge");
    let social = badge(avatar.upcast_ref(), "social-badge");
    let favorite = badge(avatar.upcast_ref(), "favorite-avatar-badge");
    assert!(favorite.is_visible());
    assert_eq!(
        (favorite.halign(), favorite.valign()),
        (gtk::Align::Start, gtk::Align::Start)
    );
    let mut ordinary = connection.clone();
    ordinary.is_favorite = false;
    let ordinary_avatar = crate::widgets::social_badge::avatar(
        &adw::Avatar::new(64, Some("Alex"), true),
        Some(&ordinary),
    );
    assert!(find_badge(ordinary_avatar.upcast_ref(), "favorite-avatar-badge").is_none());

    assert!(
        !mark.is_visible(),
        "not nearby until a matching peer appears"
    );
    let nearby = DesktopNearbySnapshot {
        visible: true,
        status: "Nearby".into(),
        peers: vec![
            DesktopNearbyPeerSnapshot {
                id: "device".into(),
                name: "Alex".into(),
                owner_pubkey_hex: Some(PEER.to_uppercase()),
                picture_url: None,
                profile_event_id: None,
                last_seen_secs: 1,
            },
            DesktopNearbyPeerSnapshot {
                id: "own-device".into(),
                name: "You".into(),
                owner_pubkey_hex: Some(state.account.as_ref().unwrap().public_key_hex.clone()),
                picture_url: None,
                profile_event_id: None,
                last_seen_secs: 1,
            },
        ],
    };
    manager.apply_nearby_snapshot(nearby.clone());
    assert!(
        mark.is_visible(),
        "existing avatar updates; nearby-list preference is independent"
    );
    assert_eq!(
        (mark.halign(), mark.valign()),
        (gtk::Align::End, gtk::Align::End)
    );
    assert_eq!(
        (social.halign(), social.valign()),
        (gtk::Align::End, gtk::Align::Start)
    );
    let mut chat = state.current_chat.clone().unwrap();
    chat.social_connection = Some(connection);
    chat.display_name = "Alex".into();
    let header = build_chat_header_avatar(&chat, &state, &manager);
    assert!(
        badge(&header, "nearby-avatar-badge").is_visible(),
        "chat header uses the same state"
    );
    let mut group = chat.clone();
    group.kind = iris_chat_core::ChatKind::Group;
    let group_header = build_chat_header_avatar(&group, &state, &manager);
    assert!(
        !badge(&group_header, "nearby-avatar-badge").is_visible(),
        "group never inherits a user badge even if IDs coincide"
    );
    let own = crate::widgets::social_badge::user_avatar(
        &adw::Avatar::new(48, Some("You"), true),
        None,
        &state.account.as_ref().unwrap().public_key_hex,
        &manager,
    );
    assert!(!badge(own.upcast_ref(), "nearby-avatar-badge").is_visible());
    let unresolved = crate::widgets::social_badge::user_avatar(
        &adw::Avatar::new(48, Some("Unknown"), true),
        None,
        "device",
        &manager,
    );
    assert!(
        !badge(unresolved.upcast_ref(), "nearby-avatar-badge").is_visible(),
        "device IDs do not match user identities"
    );
    manager.apply_nearby_snapshot(DesktopNearbySnapshot {
        peers: vec![],
        ..nearby.clone()
    });
    assert!(
        !mark.is_visible(),
        "disappearing peer clears existing avatar"
    );
    assert!(
        !badge(&header, "nearby-avatar-badge").is_visible(),
        "header updates too"
    );
    manager.apply_nearby_snapshot(nearby.clone());
    state.preferences.nearby_enabled = false;
    manager.sync_nearby_preference(&state);
    assert!(!mark.is_visible(), "master switch clears badge");
    state.preferences.nearby_enabled = true;
    manager.sync_nearby_preference(&state);
    manager.apply_nearby_snapshot(DesktopNearbySnapshot {
        visible: false,
        ..nearby.clone()
    });
    assert!(
        !mark.is_visible(),
        "stopped service clears badge even with old peers"
    );
    manager.apply_nearby_snapshot(nearby);

    let css = gtk::CssProvider::new();
    css.load_from_data(crate::widgets::social_badge::CSS);
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let column = gtk::Box::new(gtk::Orientation::Vertical, 20);
    column.set_margin_top(24);
    column.set_margin_bottom(24);
    column.set_margin_start(24);
    column.set_margin_end(24);
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    title.append(&header);
    title.append(&gtk::Label::new(Some("Alex")));
    column.append(&title);
    column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    column.append(&avatar);
    column.append(&gtk::Label::new(Some("Alex · Nearby")));
    let window = adw::Window::builder()
        .title("Nearby avatars")
        .default_width(320)
        .default_height(250)
        .build();
    window.set_content(Some(&column));
    window.present();
    pump_until(|| mark.width() > 0 && social.width() > 0);
    let mark_bounds = mark.compute_bounds(&avatar).unwrap();
    let social_bounds = social.compute_bounds(&avatar).unwrap();
    assert!(
        mark_bounds.y() > social_bounds.y() + social_bounds.height(),
        "badges do not overlap"
    );
    let header_mark = badge(&header, "nearby-avatar-badge")
        .compute_bounds(&header)
        .unwrap();
    let header_social = badge(&header, "social-badge")
        .compute_bounds(&header)
        .unwrap();
    assert!(
        header_mark.y() >= header_social.y() + header_social.height(),
        "small chat header badges do not overlap: nearby={header_mark:?}, social={header_social:?}"
    );
    if let Some(path) = std::env::var_os("IRIS_NEARBY_AVATAR_SCREENSHOT") {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
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
            .render_texture(&node.unwrap(), None)
            .save_to_png(path)
            .unwrap();
    }
    window.close();
    println!("PASS: GTK nearby avatar/header presence, removal, preferences, self exclusion and badge placement");
}
