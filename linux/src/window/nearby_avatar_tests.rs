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
        if let Some(found) = find_badge(&widget, class) {
            return Some(found);
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
    if std::env::var_os("IRIS_NEARBY_LAYOUT_ONLY").is_some() {
        install_nearby_css();
        verify_nearby_row_geometry(&manager);
        println!("PASS: Nearby row geometry for 0, 1, 8, 0 peers");
        return;
    }
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
    let core_connection = connection.clone();
    manager.set_profile_reader(move |owner| iris_chat_core::UserProfileSnapshot {
        owner_pubkey_hex: owner.into(),
        name: iris_chat_core::present_person_name(
            if owner == PEER {
                "Ada".into()
            } else {
                String::new()
            },
            owner.into(),
            (owner == PEER).then(|| "Ada".into()),
        ),
        picture_url: None,
        social_connection: (owner == PEER).then(|| core_connection.clone()),
    });
    let profile_name = crate::widgets::user_profile::name(&manager, PEER);
    assert_eq!(
        profile_name.text(),
        "Ada",
        "name comes from the profile reader, not view metadata"
    );
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
    // Every state update syncs the Nearby service. Its visibility reports
    // carry no peers and must not erase the FIPS peer list.
    let service_rx = manager.nearby_update_rx();
    let mut lan_state = state.clone();
    lan_state.preferences.nearby_lan_enabled = true;
    for _ in 0..3 {
        manager.sync_nearby_preference(&lan_state);
        while let Ok(snapshot) = service_rx.try_recv() {
            manager.apply_nearby_service_snapshot(snapshot);
        }
    }
    assert_eq!(manager.nearby_snapshot().peers, nearby.peers);
    assert!(mark.is_visible(), "state sync keeps nearby peers");
    let mut second_device = nearby.clone();
    second_device.peers.push(DesktopNearbyPeerSnapshot {
        id: "second-device".into(),
        owner_pubkey_hex: Some(PEER.to_lowercase()),
        ..nearby.peers[0].clone()
    });
    manager.apply_nearby_snapshot(second_device);
    assert_eq!(
        manager.nearby_snapshot().peers,
        nearby.peers,
        "a person with two nearby devices shows once"
    );
    manager.apply_nearby_service_snapshot(DesktopNearbySnapshot {
        visible: false,
        status: "Off".into(),
        peers: vec![],
    });
    assert!(manager.nearby_snapshot().peers.is_empty());
    manager.apply_nearby_snapshot(nearby.clone());
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
        find_badge(&group_header, "nearby-avatar-badge").is_none_or(|badge| !badge.is_visible()),
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
    check_nearby_screen_list(&manager);
    check_nearby_card_balanced(&manager);
    check_nearby_dialog_scrolls(&manager);

    install_nearby_css();
    verify_nearby_row_geometry(&manager);
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

fn install_nearby_css() {
    let css = gtk::CssProvider::new();
    css.load_from_data(&format!(
        "{}\n{}",
        crate::widgets::social_badge::CSS,
        crate::widgets::controls::CSS
    ));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn verify_nearby_row_geometry(manager: &std::rc::Rc<AppManager>) {
    let original = manager.nearby_snapshot();
    let window = adw::Window::builder()
        .title("Nearby layout")
        .default_width(320)
        .default_height(180)
        .build();
    let mut baseline = None;
    for count in [0, 1, 8, 0] {
        manager.apply_nearby_snapshot(DesktopNearbySnapshot {
            visible: true,
            status: "Nearby".into(),
            peers: (0..count)
                .map(|index| DesktopNearbyPeerSnapshot {
                    id: format!("peer-{index}"),
                    name: format!("A long nearby display name {index}"),
                    // Real peers have owners (profile avatars); odd ones stay unresolved.
                    owner_pubkey_hex: (index % 2 == 0).then(|| format!("{:064x}", index + 1)),
                    picture_url: None,
                    profile_event_id: None,
                    last_seen_secs: 1,
                })
                .collect(),
        });
        let row = crate::screens::chat_list::nearby_row_for_test(manager);
        let following = gtk::Label::new(Some("Following chat"));
        following.set_size_request(-1, 60);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.set_valign(gtk::Align::Start);
        column.append(&row);
        column.append(&following);
        window.set_content(Some(&column));
        window.present();
        pump_until(|| row.width() > 0 && following.height() > 0);
        let bounds = row.compute_bounds(&column).unwrap();
        let following_bounds = following.compute_bounds(&column).unwrap();
        let geometry = (bounds.y(), bounds.height(), following_bounds.y());
        check_nearby_columns_centered(&row, count);
        if count == 0 {
            check_empty_label_centered(&row);
        }
        if let Some(baseline) = baseline {
            assert_eq!(
                geometry, baseline,
                "Nearby layout changed with {count} peers"
            );
        } else {
            baseline = Some(geometry);
        }
    }
    window.close();
    manager.apply_nearby_snapshot(original);
}

fn check_nearby_columns_centered(row: &gtk::Widget, count: usize) {
    fn first_avatar(widget: &gtk::Widget) -> Option<gtk::Widget> {
        if widget.is::<adw::Avatar>() {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(found) = first_avatar(&widget) {
                return Some(found);
            }
            child = widget.next_sibling();
        }
        None
    }
    fn columns(widget: &gtk::Widget, found: &mut Vec<gtk::Widget>) {
        if widget.has_css_class("nearby-preview-button") && widget.is::<gtk::Button>() {
            if let Some(column) = widget.first_child().filter(|c| c.is::<gtk::Box>()) {
                found.push(column);
                return;
            }
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            columns(&widget, found);
            child = widget.next_sibling();
        }
    }
    let mut found = Vec::new();
    columns(row, &mut found);
    assert_eq!(found.len(), count, "one column per nearby person");
    for column in found {
        let avatar = first_avatar(&column).expect("nearby avatar");
        let name = column.last_child().expect("nearby name");
        let center = |widget: &gtk::Widget| {
            let bounds = widget.compute_bounds(&column).unwrap();
            bounds.x() + bounds.width() / 2.0
        };
        assert!(
            (center(&avatar) - center(&name)).abs() < 1.0,
            "nearby avatar and name share a center line"
        );
    }
}

fn check_nearby_screen_list(manager: &std::rc::Rc<AppManager>) {
    fn has(widget: &gtk::Widget, found: &dyn Fn(&gtk::Widget) -> bool) -> bool {
        if found(widget) {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if has(&widget, found) {
                return true;
            }
            child = widget.next_sibling();
        }
        false
    }
    let mut rendered = None;
    let list = crate::screens::nearby_list_for_test(manager, &mut rendered);
    let rows: Vec<gtk::Widget> = {
        let mut rows = Vec::new();
        let mut child = list.first_child();
        while let Some(row) = child {
            child = row.next_sibling();
            rows.push(row);
        }
        rows
    };
    assert!(!rows.is_empty());
    for row in &rows {
        assert!(
            has(row, &|widget| widget.is::<adw::Avatar>()),
            "Nearby list shows each person's avatar"
        );
        assert!(
            !has(row, &|widget| widget
                .downcast_ref::<gtk::Image>()
                .and_then(|image| image.icon_name())
                .is_some_and(|name| name == "avatar-default-symbolic")),
            "no generic person icon"
        );
    }
    crate::screens::update_nearby_list_for_test(&list, manager, &mut rendered);
    assert_eq!(
        list.first_child(),
        Some(rows[0].clone()),
        "unchanged Nearby keeps its rows"
    );
}

/// Space above the avatars matches space below the names' text.
fn check_nearby_card_balanced(manager: &std::rc::Rc<AppManager>) {
    fn find(widget: &gtk::Widget, found: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
        if found(widget) {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(hit) = find(&widget, found) {
                return Some(hit);
            }
            child = widget.next_sibling();
        }
        None
    }
    let card = crate::screens::chat_list::nearby_card_for_test(manager);
    let window = adw::Window::builder()
        .default_width(360)
        .default_height(200)
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.set_valign(gtk::Align::Start);
    column.append(&card);
    window.set_content(Some(&column));
    window.present();
    let row = card.first_child().expect("card row");
    pump_until(|| row.height() > 0 && row.is_mapped());
    let avatar = find(&row, &|widget| widget.is::<adw::Avatar>()).expect("nearby avatar");
    let name = find(&row, &|widget| {
        widget.is::<gtk::Label>() && widget.has_css_class("caption")
    })
    .and_then(|widget| widget.downcast::<gtk::Label>().ok())
    .expect("nearby name");
    let top = avatar.compute_bounds(&row).unwrap().y() as f64;
    let name_bounds = name.compute_bounds(&row).unwrap();
    let (ink, _) = name.layout().pixel_extents();
    let (_, offset_y) = name.layout_offsets();
    let ink_bottom = name_bounds.y() as f64 + offset_y as f64 + (ink.y() + ink.height()) as f64;
    let bottom = row.height() as f64 - ink_bottom;
    window.close();
    assert!(
        (top - bottom).abs() <= 1.5,
        "Nearby card is off-center: {top}px above avatars, {bottom}px below names"
    );
}

/// "No users nearby" lines up with the center of the Nearby button.
fn check_empty_label_centered(row: &gtk::Widget) {
    fn find(widget: &gtk::Widget, found: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
        if found(widget) {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(hit) = find(&widget, found) {
                return Some(hit);
            }
            child = widget.next_sibling();
        }
        None
    }
    let icon = find(row, &|widget| widget.has_css_class("nearby-avatar-button")).unwrap();
    let label = find(row, &|widget| widget.is::<gtk::Label>()).unwrap();
    let center = |widget: &gtk::Widget| {
        let bounds = widget.compute_bounds(row).unwrap();
        bounds.y() + bounds.height() / 2.0
    };
    assert!(
        (center(&icon) - center(&label)).abs() <= 1.0,
        "empty Nearby text is centered on the Nearby button"
    );
}

/// With many people nearby in a short window, the Nearby dialog scrolls.
fn check_nearby_dialog_scrolls(manager: &std::rc::Rc<AppManager>) {
    fn all(widget: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        out.push(widget.clone());
        let mut child = widget.first_child();
        while let Some(widget) = child {
            all(&widget, out);
            child = widget.next_sibling();
        }
    }
    let original = manager.nearby_snapshot();
    manager.apply_nearby_snapshot(DesktopNearbySnapshot {
        visible: true,
        status: "Nearby".into(),
        peers: (0..12)
            .map(|index| DesktopNearbyPeerSnapshot {
                id: format!("dialog-peer-{index}"),
                name: format!("Person {index}"),
                owner_pubkey_hex: None,
                picture_url: None,
                profile_event_id: None,
                last_seen_secs: 1,
            })
            .collect(),
    });
    let window = adw::ApplicationWindow::builder()
        .default_width(400)
        .default_height(360)
        .build();
    window.present();
    pump_until(|| window.is_mapped());
    crate::screens::present_nearby(Some(window.upcast_ref()), manager.clone());
    let scroller = || {
        let mut widgets = Vec::new();
        all(window.upcast_ref(), &mut widgets);
        widgets.into_iter().find_map(|widget| {
            let scroller = widget.downcast::<gtk::ScrolledWindow>().ok()?;
            let mut inner = Vec::new();
            all(scroller.upcast_ref(), &mut inner);
            inner
                .iter()
                .any(|w| w.is::<gtk::ListBox>() && w.has_css_class("boxed-list"))
                .then_some(scroller)
        })
    };
    pump_until(|| scroller().is_some_and(|s| s.height() > 0));
    let scroller = scroller().unwrap();
    pump_until(|| scroller.vadjustment().upper() > 0.0);
    let adjustment = scroller.vadjustment();
    assert!(
        adjustment.upper() > adjustment.page_size(),
        "12 nearby people overflow a short window, so the dialog scrolls"
    );
    window.close();
    manager.apply_nearby_snapshot(original);
}
