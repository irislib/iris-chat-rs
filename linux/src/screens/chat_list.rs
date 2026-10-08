use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use adw::prelude::*;
use iris_chat_core::{
    AppAction, AppState, ChatInputShortcut, ChatKind, ChatThreadSnapshot,
    DesktopNearbyPeerSnapshot, FollowedUserSearchResult, MessageSearchHit, PreferencesSnapshot,
    SearchResultSnapshot,
};

use crate::app_manager::{AppManager, SearchUiState};
use crate::screens::chat::{present_chat_info, ChatInfoSnapshot};
use crate::screens::confirm_delete_chat;
use crate::widgets::clickable::PointerCursorExt;
use crate::widgets::image_cache;

mod timestamps;
#[cfg(feature = "ui-tests")]
pub use timestamps::verify_ui as verify_timestamps_ui;
use timestamps::Timestamps;

#[derive(PartialEq)]
struct ListKey {
    chats: Vec<ChatThreadSnapshot>,
    preferences: PreferencesSnapshot,
    nearby: iris_chat_core::DesktopNearbySnapshot,
    search: SearchUiState,
    results: Option<SearchResultSnapshot>,
    discovering: bool,
    selected: Option<String>,
}

pub(crate) struct ChatListView {
    pub root: gtk::Box,
    entry: gtk::SearchEntry,
    scope: gtk::Box,
    scrolled: gtk::ScrolledWindow,
    sync: super::sync_status::SyncStatus,
    timestamps: Timestamps,
    rendered: Option<ListKey>,
}

impl ChatListView {
    pub fn new(manager: &Rc<AppManager>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        let timestamps = Timestamps::install(&root);
        let (search_box, entry, scope) = build_search_box(manager);
        root.append(&search_box);
        let sync = super::sync_status::SyncStatus::new();
        root.append(&sync.root);
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
        scrolled.set_vexpand(true);
        root.append(&scrolled);
        Self {
            root,
            entry,
            scope,
            scrolled,
            sync,
            timestamps,
            rendered: None,
        }
    }

    pub fn update(&mut self, state: &AppState, manager: &Rc<AppManager>) {
        self.sync.update(state);
        let ui = manager.search_ui();
        // Keep the live editor (including selection and IME preedit) mounted.
        if self.entry.text().as_str() != ui.query {
            self.entry.set_text(&ui.query);
        }
        let scope_changed = self.rendered.as_ref().is_none_or(|old| {
            old.search.scope_chat_id != ui.scope_chat_id
                || old.search.scope_display_name != ui.scope_display_name
        });
        if scope_changed {
            while let Some(child) = self.scope.first_child() {
                self.scope.remove(&child);
            }
            if let (Some(id), Some(name)) = (&ui.scope_chat_id, &ui.scope_display_name) {
                self.scope.append(&build_scope_chip(manager, id, name));
            }
            self.scope.set_visible(self.scope.first_child().is_some());
            self.entry.set_placeholder_text(Some("Search"));
        }
        let focus_requested = self
            .rendered
            .as_ref()
            .is_none_or(|old| old.search.focus_request != ui.focus_request)
            && ui.focus_request > 0;
        let key = ListKey {
            chats: state.chat_list.clone(),
            preferences: state.preferences.clone(),
            nearby: manager.nearby_snapshot(),
            results: if ui.is_active() {
                manager.search_results(50)
            } else {
                None
            },
            discovering: state.user_discovery_syncing,
            selected: state.current_chat.as_ref().map(|c| c.chat_id.clone()),
            search: ui,
        };
        if self.rendered.as_ref() != Some(&key) {
            let focus =
                crate::widgets::keyboard_list::FocusBookmark::capture(self.scrolled.upcast_ref());
            let offset = self.scrolled.vadjustment().value();
            self.timestamps.clear();
            let body = build_body(
                state,
                manager,
                &key.search,
                key.results.as_ref(),
                &self.timestamps,
            );
            if let Some(id) = key.selected.as_ref() {
                mark_selected(body.upcast_ref(), &format!("iris-keyboard-chat-{id}"));
            }
            self.scrolled.set_child(Some(&body));
            self.scrolled.vadjustment().set_value(offset);
            if let Some(focus) = focus {
                focus.restore(self.scrolled.upcast_ref());
            }
            self.rendered = Some(key);
        }
        self.timestamps.refresh(unix_now());
        if focus_requested {
            self.entry.grab_focus();
        }
    }
}

fn mark_selected(widget: &gtk::Widget, name: &str) {
    if widget.widget_name() == name {
        widget.add_css_class("selected-chat");
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        mark_selected(&widget, name);
        child = widget.next_sibling();
    }
}

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let mut view = ChatListView::new(manager);
    view.update(state, manager);
    view.root.upcast()
}

fn build_body(
    state: &AppState,
    manager: &Rc<AppManager>,
    ui_state: &SearchUiState,
    results: Option<&SearchResultSnapshot>,
    timestamps: &Timestamps,
) -> gtk::Box {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.set_widget_name("iris-keyboard-chat-list");
    if ui_state.is_active() {
        if let Some(results) = results {
            append_search_results(&body, state, manager, results, timestamps);
        } else {
            let searching = gtk::Label::new(Some("Searching…"));
            searching.add_css_class("dim-label");
            searching.set_margin_top(20);
            body.append(&searching);
        }
    } else {
        body.set_margin_top(12);
        body.set_margin_bottom(12);
        body.set_margin_start(12);
        body.set_margin_end(12);

        let now = unix_now();
        let show_nearby = state.preferences.nearby_show_in_chat_list;
        let pinned: Vec<&ChatThreadSnapshot> = state
            .chat_list
            .iter()
            .filter(|chat| chat.is_pinned)
            .collect();
        let unpinned: Vec<&ChatThreadSnapshot> = state
            .chat_list
            .iter()
            .filter(|chat| !chat.is_pinned)
            .collect();
        let section_count = usize::from(show_nearby)
            + usize::from(!pinned.is_empty())
            + usize::from(!unpinned.is_empty() || state.chat_list.is_empty());

        if show_nearby {
            append_grouped_section(
                &body,
                (section_count > 1).then_some("Nearby"),
                vec![nearby_row(manager)],
            );
        }
        if !pinned.is_empty() {
            append_grouped_section(
                &body,
                (section_count > 1).then_some("Pinned"),
                pinned
                    .into_iter()
                    .map(|chat| {
                        row_for(chat, &state.preferences, now, manager, timestamps).upcast()
                    })
                    .collect(),
            );
        }
        if unpinned.is_empty() && state.chat_list.is_empty() {
            append_grouped_section(
                &body,
                (section_count > 1).then_some("Chats"),
                vec![empty_chats_row()],
            );
        } else if !unpinned.is_empty() {
            append_grouped_section(
                &body,
                (section_count > 1).then_some("Chats"),
                unpinned
                    .into_iter()
                    .map(|chat| {
                        row_for(chat, &state.preferences, now, manager, timestamps).upcast()
                    })
                    .collect(),
            );
        }
    }

    crate::widgets::keyboard_list::install(&body);
    body
}

impl SearchUiState {
    fn is_active(&self) -> bool {
        !self.query.trim().is_empty() || self.scope_chat_id.is_some()
    }
}

fn build_search_box(manager: &Rc<AppManager>) -> (gtk::Box, gtk::SearchEntry, gtk::Box) {
    let ui_state = manager.search_ui();
    let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 6);
    wrapper.set_margin_top(8);
    wrapper.set_margin_start(12);
    wrapper.set_margin_end(12);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);

    let scope = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    scope.set_visible(false);
    wrapper.append(&scope);

    let entry = gtk::SearchEntry::new();
    entry.set_widget_name("iris-keyboard-search");
    entry.set_hexpand(true);
    entry.set_text(&ui_state.query);
    entry.set_placeholder_text(Some("Search"));

    let manager_for_change = manager.clone();
    entry.connect_changed(move |entry| {
        let text = entry.text().to_string();
        if manager_for_change.search_ui().query == text {
            return;
        }
        // Auto-proceed when the user pasted a full npub / invite URL —
        // saves them tapping the shortcut row. Partial input never
        // classifies (the core parser only accepts complete keys), so
        // this is safe on every keystroke.
        if let Some(shortcut) = iris_chat_core::classify_chat_input(text.clone()) {
            let action = match shortcut {
                iris_chat_core::ChatInputShortcut::DirectPeer { peer_input, .. } => {
                    iris_chat_core::AppAction::CreateChat { peer_input }
                }
                iris_chat_core::ChatInputShortcut::Invite { invite_input, .. } => {
                    iris_chat_core::AppAction::AcceptInvite { invite_input }
                }
            };
            manager_for_change.clear_search();
            manager_for_change.dispatch(action);
            manager_for_change.redraw_ui();
            return;
        }
        manager_for_change.set_search_query(text);
        manager_for_change.redraw_ui();
    });

    row.append(&entry);
    wrapper.append(&row);

    (wrapper, entry, scope)
}

fn build_scope_chip(manager: &Rc<AppManager>, _chat_id: &str, name: &str) -> gtk::Widget {
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    chip.add_css_class("pill");
    chip.add_css_class("card");
    let label = gtk::Label::new(Some(name));
    label.add_css_class("caption-heading");
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(28);
    chip.append(&label);
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.add_css_class("flat");
    close.add_css_class("circular");
    close.set_tooltip_text(Some("Clear filter"));
    let manager = manager.clone();
    close.connect_clicked(move |_| {
        manager.clear_chat_scope();
        manager.redraw_ui();
    });
    chip.append(&close);
    chip.upcast()
}

fn append_search_results(
    container: &gtk::Box,
    state: &AppState,
    manager: &Rc<AppManager>,
    results: &SearchResultSnapshot,
    timestamps: &Timestamps,
) {
    container.set_margin_top(8);
    container.set_margin_bottom(12);
    container.set_margin_start(12);
    container.set_margin_end(12);

    let now = unix_now();
    let mut wrote_any = false;

    if let Some(shortcut) = results.shortcut.as_ref() {
        container.append(&shortcut_row(shortcut, manager));
        wrote_any = true;
    }

    if state.user_discovery_syncing && results.people.is_empty() {
        container.append(&section_label("People"));
        let finding = gtk::Label::new(Some("Finding people…"));
        finding.add_css_class("dim-label");
        finding.set_halign(gtk::Align::Start);
        finding.set_margin_top(8);
        finding.set_margin_bottom(8);
        container.append(&finding);
        wrote_any = true;
    }

    if !results.people.is_empty() {
        container.append(&section_label("People"));
        let list = grouped_list();
        let expanded = manager.search_ui().people_expanded;
        let visible = if expanded {
            results.people.as_slice()
        } else {
            &results.people[..results.people.len().min(7)]
        };
        for person in visible {
            list.append(&person_row(person, &state.preferences, manager));
        }
        container.append(&list);
        if results.people.len() > visible.len() {
            let more = gtk::Button::with_label("View more");
            more.add_css_class("flat");
            more.set_halign(gtk::Align::Start);
            let manager = manager.clone();
            more.connect_clicked(move |_| {
                manager.expand_people_search();
                manager.redraw_ui();
            });
            container.append(&more);
        }
        wrote_any = true;
    }

    if !results.contacts.is_empty() {
        container.append(&section_label("Contacts"));
        let list = grouped_list();
        for chat in &results.contacts {
            list.append(&row_for(chat, &state.preferences, now, manager, timestamps));
        }
        container.append(&list);
        wrote_any = true;
    }

    if !results.groups.is_empty() {
        container.append(&section_label("Groups"));
        let list = grouped_list();
        for chat in &results.groups {
            list.append(&row_for(chat, &state.preferences, now, manager, timestamps));
        }
        container.append(&list);
        wrote_any = true;
    }

    if !results.messages.is_empty() {
        container.append(&section_label("Messages"));
        let list = grouped_list();
        for hit in &results.messages {
            list.append(&message_hit_row(
                hit,
                &state.preferences,
                now,
                manager,
                timestamps,
            ));
        }
        container.append(&list);
        wrote_any = true;
    }

    if !wrote_any {
        let empty = gtk::Label::new(Some("No matches"));
        empty.add_css_class("dim-label");
        empty.set_margin_top(48);
        empty.set_margin_bottom(48);
        container.append(&empty);
    }
}

fn shortcut_row(shortcut: &ChatInputShortcut, manager: &Rc<AppManager>) -> gtk::Widget {
    let (icon_name, title, subtitle, action) = match shortcut {
        ChatInputShortcut::DirectPeer {
            peer_input,
            display,
            ..
        } => (
            "list-add-symbolic",
            format!("Start chat with {display}"),
            "New direct chat".to_string(),
            AppAction::CreateChat {
                peer_input: peer_input.clone(),
            },
        ),
        ChatInputShortcut::Invite {
            invite_input,
            display,
        } => (
            "mail-attachment-symbolic",
            "Accept invite".to_string(),
            display.clone(),
            AppAction::AcceptInvite {
                invite_input: invite_input.clone(),
            },
        ),
    };
    let row = adw::ActionRow::builder()
        .title(escape(&title))
        .subtitle(escape(&subtitle))
        .activatable(true)
        .build();
    row.show_pointer_cursor();
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(28);
    row.add_prefix(&icon);
    let manager = manager.clone();
    row.connect_activated(move |_| {
        manager.clear_search();
        manager.dispatch(action.clone());
    });

    let list = grouped_list();
    list.append(&row);
    list.upcast()
}

fn section_label(text: &str) -> gtk::Widget {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("heading");
    label.add_css_class("dim-label");
    label.set_halign(gtk::Align::Start);
    label.set_margin_top(12);
    label.set_margin_bottom(4);
    label.upcast()
}

fn grouped_list() -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    list
}

fn append_grouped_section(container: &gtk::Box, title: Option<&str>, rows: Vec<gtk::Widget>) {
    if let Some(title) = title {
        container.append(&section_label(title));
    }
    let list = grouped_list();
    for row in rows {
        list.append(&row);
    }
    container.append(&list);
}

fn empty_chats_row() -> gtk::Widget {
    let label = gtk::Label::new(Some("No chats yet"));
    label.add_css_class("dim-label");
    label.set_margin_top(18);
    label.set_margin_bottom(18);
    label.set_halign(gtk::Align::Center);
    label.upcast()
}

fn message_hit_row(
    hit: &MessageSearchHit,
    prefs: &PreferencesSnapshot,
    now: u64,
    manager: &Rc<AppManager>,
    timestamps: &Timestamps,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(escape(&hit.chat_display_name))
        .subtitle(escape(
            &hit.body.split_whitespace().collect::<Vec<_>>().join(" "),
        ))
        .title_lines(1)
        .subtitle_lines(1)
        .activatable(true)
        .build();
    row.show_pointer_cursor();
    let avatar = adw::Avatar::new(40, Some(&hit.chat_display_name), true);
    if let Some(url) = hit.chat_picture_url.as_ref() {
        image_cache::fetch_proxied_into_avatar(&avatar, url, prefs, 80);
    }
    row.add_prefix(&crate::widgets::social_badge::user_avatar(
        &avatar,
        if hit.chat_kind == ChatKind::Direct {
            manager.contact_social_connection(&hit.chat_id)
        } else {
            None
        }
        .as_ref(),
        if hit.chat_kind == ChatKind::Direct {
            &hit.chat_id
        } else {
            ""
        },
        manager,
    ));

    if hit.created_at_secs > 0 {
        let label = timestamps.label(hit.created_at_secs, now);
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        label.set_valign(gtk::Align::Center);
        row.add_suffix(&label);
    }

    let manager = manager.clone();
    let chat_id = hit.chat_id.clone();
    row.connect_activated(move |_| {
        manager.clear_search();
        manager.dispatch(AppAction::OpenChat {
            chat_id: chat_id.clone(),
        });
    });

    row
}

fn person_row(
    person: &FollowedUserSearchResult,
    prefs: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
) -> adw::ActionRow {
    let subtitle = person
        .profile_label
        .as_deref()
        .filter(|label| !label.eq_ignore_ascii_case(&person.display_label))
        .or(person.about.as_deref())
        .unwrap_or("");
    let row = adw::ActionRow::builder()
        .title(escape(&person.display_label))
        .subtitle(escape(subtitle))
        .activatable(true)
        .build();
    row.show_pointer_cursor();
    let avatar = adw::Avatar::new(40, Some(&person.display_label), true);
    if let Some(url) = person.picture_url.as_ref() {
        image_cache::fetch_proxied_into_avatar(&avatar, url, prefs, 80);
    }
    row.add_prefix(&crate::widgets::social_badge::user_avatar(
        &avatar,
        person.social_connection.as_ref(),
        &person.owner_pubkey_hex,
        manager,
    ));
    let manager = manager.clone();
    let owner = person.owner_pubkey_hex.clone();
    row.connect_activated(move |_| {
        manager.clear_search();
        manager.dispatch(AppAction::CreateChat {
            peer_input: owner.clone(),
        });
    });
    row
}

fn nearby_row(manager: &Rc<AppManager>) -> gtk::Widget {
    let snapshot = manager.nearby_snapshot();
    let nearby_enabled = manager.current_state().preferences.nearby_enabled;
    let active = nearby_enabled && snapshot.visible;
    let peers = if nearby_enabled {
        snapshot.peers.as_slice()
    } else {
        &[]
    };
    const NEARBY_AVATAR_SIZE: i32 = 40;
    const NEARBY_ROW_CONTENT_HEIGHT: i32 = NEARBY_AVATAR_SIZE + 22;

    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    outer.set_margin_top(6);
    outer.set_margin_bottom(6);
    outer.set_margin_start(12);
    outer.set_margin_end(12);
    outer.set_hexpand(true);
    outer.set_size_request(-1, NEARBY_ROW_CONTENT_HEIGHT);
    outer.set_valign(gtk::Align::Start);

    if !peers.is_empty() {
        outer.append(&nearby_icon_button(manager, active, NEARBY_AVATAR_SIZE));
        outer.append(&nearby_avatar_strip(peers, manager));
        return outer.upcast();
    }

    outer.append(&nearby_icon(active, NEARBY_AVATAR_SIZE));

    let label = gtk::Label::new(Some(if !nearby_enabled {
        "Off"
    } else if active {
        "No users nearby"
    } else {
        "Tap to enable"
    }));
    label.set_valign(gtk::Align::Center);
    label.set_halign(gtk::Align::Start);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class("dim-label");
    label.set_hexpand(true);
    outer.append(&label);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.show_pointer_cursor();
    button.set_child(Some(&outer));
    let manager_for_click = manager.clone();
    button.connect_clicked(move |btn| {
        present_nearby_from_button(btn, manager_for_click.clone());
    });
    button.upcast()
}

fn nearby_icon_button(manager: &Rc<AppManager>, active: bool, size: i32) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("nearby-avatar-button");
    button.show_pointer_cursor();
    button.set_size_request(size, size);
    button.set_tooltip_text(Some("Nearby"));
    button.set_child(Some(&nearby_icon(active, size)));
    button.set_valign(gtk::Align::Start);
    let manager_for_click = manager.clone();
    button.connect_clicked(move |btn| {
        present_nearby_from_button(btn, manager_for_click.clone());
    });
    button
}

fn nearby_icon(active: bool, size: i32) -> gtk::Box {
    let background = gtk::Box::new(gtk::Orientation::Vertical, 0);
    background.set_size_request(size, size);
    background.set_valign(gtk::Align::Start);
    background.set_halign(gtk::Align::Center);
    background.add_css_class("nearby-avatar");
    background.add_css_class(if active {
        "nearby-active"
    } else {
        "nearby-off"
    });
    let icon = gtk::Image::from_icon_name("network-wireless-symbolic");
    icon.set_pixel_size(24);
    icon.set_vexpand(true);
    icon.set_valign(gtk::Align::Center);
    icon.set_halign(gtk::Align::Center);
    icon.add_css_class(if active {
        "nearby-active-icon"
    } else {
        "dim-label"
    });
    background.append(&icon);
    background
}

fn present_nearby_from_button(button: &gtk::Button, manager: Rc<AppManager>) {
    let parent = button.root().and_then(|r| r.downcast::<gtk::Window>().ok());
    crate::screens::present_nearby(parent.as_ref(), manager);
}

fn nearby_avatar_strip(
    peers: &[DesktopNearbyPeerSnapshot],
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let avatar_size: i32 = 40;
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    strip.set_valign(gtk::Align::Start);
    let prefs = manager.current_state().preferences.clone();
    for peer in peers {
        let name = nearby_peer_resolved_name(peer, manager, "Nearby user");
        let avatar = adw::Avatar::new(avatar_size, Some(&name), true);
        if let Some(url) = peer.picture_url.as_ref() {
            image_cache::fetch_proxied_into_avatar(&avatar, url, &prefs, (avatar_size * 2) as u32);
        }
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.set_size_request(64, -1);
        column.set_halign(gtk::Align::Center);
        column.set_valign(gtk::Align::Start);
        column.append(&crate::widgets::social_badge::user_avatar(
            &avatar,
            manager
                .contact_social_connection(peer.owner_pubkey_hex.as_deref().unwrap_or_default())
                .as_ref(),
            peer.owner_pubkey_hex.as_deref().unwrap_or_default(),
            manager,
        ));
        let label = gtk::Label::new(Some(&nearby_peer_display_name(&name)));
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        label.set_max_width_chars(9);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_halign(gtk::Align::Center);
        label.set_xalign(0.5);
        column.append(&label);

        let button = gtk::Button::new();
        button.add_css_class("flat");
        button.show_pointer_cursor();
        button.set_child(Some(&column));
        button.set_tooltip_text(Some(&name));
        if let Some(owner) = peer.owner_pubkey_hex.clone() {
            let manager_for_click = manager.clone();
            let peer_for_click = peer.clone();
            button.connect_clicked(move |button| {
                open_nearby_peer_from_widget(
                    button,
                    &peer_for_click,
                    owner.as_str(),
                    manager_for_click.clone(),
                );
            });
        } else {
            button.set_sensitive(false);
        }
        strip.append(&button);
    }

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    scrolled.set_hexpand(true);
    scrolled.set_min_content_height(avatar_size + 22);
    scrolled.set_child(Some(&strip));
    scrolled.upcast()
}

fn nearby_peer_display_name(name: &str) -> String {
    let trimmed = name.trim();
    let value = if trimmed.is_empty() {
        "Nearby"
    } else {
        trimmed
    };
    if value.chars().count() <= 14 {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(13).collect::<String>())
    }
}

fn nearby_peer_resolved_name(
    peer: &DesktopNearbyPeerSnapshot,
    manager: &Rc<AppManager>,
    fallback: &str,
) -> String {
    if let Some(owner) = peer.owner_pubkey_hex.as_deref() {
        let state = manager.current_state();
        if let Some(chat) = state.chat_list.iter().find(|chat| {
            matches!(chat.kind, ChatKind::Direct) && chat.chat_id.eq_ignore_ascii_case(owner)
        }) {
            let name = chat.display_name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    let name = peer.name.trim();
    if name.is_empty() {
        fallback.to_string()
    } else {
        name.to_string()
    }
}

fn open_nearby_peer_from_widget(
    widget: &gtk::Button,
    peer: &DesktopNearbyPeerSnapshot,
    owner: &str,
    manager: Rc<AppManager>,
) {
    if is_known_direct_chat(&manager, owner) {
        manager.dispatch(AppAction::OpenChat {
            chat_id: owner.to_string(),
        });
        return;
    }

    let parent = widget.root().and_then(|r| r.downcast::<gtk::Window>().ok());
    present_chat_info(
        parent.as_ref(),
        nearby_peer_chat_info(peer, owner, &manager),
        manager,
    );
}

fn is_known_direct_chat(manager: &Rc<AppManager>, owner: &str) -> bool {
    manager.current_state().chat_list.iter().any(|chat| {
        matches!(chat.kind, ChatKind::Direct) && chat.chat_id.eq_ignore_ascii_case(owner)
    })
}

fn nearby_peer_chat_info(
    peer: &DesktopNearbyPeerSnapshot,
    owner: &str,
    manager: &Rc<AppManager>,
) -> ChatInfoSnapshot {
    let name = nearby_peer_resolved_name(peer, manager, "Nearby user");
    ChatInfoSnapshot {
        social_connection: None,
        chat_id: owner.to_string(),
        display_name: name,
        nickname: None,
        contact_note: None,
        profile_name: None,
        subtitle: None,
        picture_url: peer.picture_url.clone(),
        about: None,
        is_muted: false,
        show_message_action: true,
        preferences: manager.current_state().preferences,
    }
}

fn row_for(
    chat: &ChatThreadSnapshot,
    prefs: &PreferencesSnapshot,
    now: u64,
    manager: &Rc<AppManager>,
    timestamps: &Timestamps,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(crate::widgets::person_name::markup(
            &chat.display_name,
            if chat.kind == ChatKind::Direct {
                &chat.chat_id
            } else {
                ""
            },
            crate::widgets::person_name::explicit(
                chat.nickname.as_deref(),
                chat.profile_name.as_deref(),
            ),
        ))
        .activatable(true)
        .build();
    row.show_pointer_cursor();
    row.set_widget_name(&format!("iris-keyboard-chat-{}", chat.chat_id));

    let avatar = adw::Avatar::new(40, Some(&chat.display_name), true);
    if let Some(url) = chat.picture_url.as_ref() {
        image_cache::fetch_proxied_into_avatar(&avatar, url, prefs, 80);
    }
    row.add_prefix(&crate::widgets::social_badge::user_avatar(
        &avatar,
        chat.social_connection.as_ref(),
        if chat.kind == ChatKind::Direct {
            &chat.chat_id
        } else {
            ""
        },
        manager,
    ));

    let draft = chat.draft.trim();
    let subtitle = if chat.is_typing {
        "Typing…".to_string()
    } else if !draft.is_empty() {
        format!("Draft: {draft}")
    } else {
        chat.last_message_preview
            .clone()
            .or_else(|| chat.subtitle.clone())
            .unwrap_or_else(|| "No messages yet".to_string())
    };
    row.set_title_lines(1);
    row.set_subtitle_lines(1);
    row.set_subtitle(&escape(
        &subtitle.split_whitespace().collect::<Vec<_>>().join(" "),
    ));

    let suffix = gtk::Box::new(gtk::Orientation::Vertical, 4);
    suffix.set_valign(gtk::Align::Center);
    suffix.set_halign(gtk::Align::End);

    if let Some(secs) = chat.last_message_at_secs {
        let label = timestamps.label(secs, now);
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        label.set_halign(gtk::Align::End);
        suffix.append(&label);
    }

    if chat.unread_count > 0 {
        let badge = gtk::Label::new(Some(&format!("{}", chat.unread_count)));
        badge.add_css_class("caption");
        badge.add_css_class("accent");
        badge.set_halign(gtk::Align::End);
        suffix.append(&badge);
    }

    if chat.is_muted {
        let muted = gtk::Image::from_icon_name("notifications-disabled-symbolic");
        muted.add_css_class("dim-label");
        muted.set_tooltip_text(Some("muted"));
        row.add_suffix(&muted);
    }
    if chat.is_pinned {
        let pinned = gtk::Image::from_icon_name("view-pin-symbolic");
        pinned.add_css_class("dim-label");
        pinned.set_tooltip_text(Some("pinned"));
        row.add_suffix(&pinned);
    }
    row.add_suffix(&suffix);

    let activate_manager = manager.clone();
    let chat_id = chat.chat_id.clone();
    row.connect_activated(move |_| {
        activate_manager.dispatch(AppAction::OpenChat {
            chat_id: chat_id.clone(),
        });
    });

    attach_context_menu(&row, chat, manager.clone());

    row
}

fn attach_context_menu(row: &adw::ActionRow, chat: &ChatThreadSnapshot, manager: Rc<AppManager>) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    let chat = chat.clone();
    gesture.connect_pressed(move |gesture, _, x, y| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        let popover = chat_context_popover(&chat, &manager);
        popover.set_parent(&widget);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    row.add_controller(gesture);
}

fn chat_context_popover(chat: &ChatThreadSnapshot, manager: &Rc<AppManager>) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_autohide(true);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_margin_top(6);
    column.set_margin_bottom(6);
    column.set_margin_start(6);
    column.set_margin_end(6);

    let read_label = if chat.unread_count > 0 {
        "Mark read"
    } else {
        "Mark as unread"
    };
    column.append(&context_button(read_label, {
        let manager = manager.clone();
        let chat_id = chat.chat_id.clone();
        let unread = chat.unread_count == 0;
        move || {
            manager.dispatch(AppAction::SetChatUnread {
                chat_id: chat_id.clone(),
                unread,
            });
        }
    }));

    column.append(&context_button(
        if chat.is_pinned {
            "Unpin chat"
        } else {
            "Pin chat"
        },
        {
            let manager = manager.clone();
            let chat_id = chat.chat_id.clone();
            let pinned = !chat.is_pinned;
            move || {
                manager.dispatch(AppAction::SetChatPinned {
                    chat_id: chat_id.clone(),
                    pinned,
                });
            }
        },
    ));

    column.append(&context_button_with_widget("Mute notifications…", {
        let manager = manager.clone();
        let chat_id = chat.chat_id.clone();
        let muted = chat.is_muted;
        move |button| {
            let parent = button
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok());
            super::show_chat_mute_options(parent.as_ref(), &manager, &chat_id, muted);
        }
    }));

    column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let delete = context_button_with_widget("Delete", {
        let manager = manager.clone();
        let chat_id = chat.chat_id.clone();
        move |button| {
            let parent = button
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok());
            confirm_delete_chat(parent.as_ref(), &manager, chat_id.clone());
        }
    });
    delete.add_css_class("destructive-action");
    column.append(&delete);

    popover.set_child(Some(&column));
    popover
}

fn context_button(label: &str, action: impl Fn() + 'static) -> gtk::Button {
    context_button_with_widget(label, move |_| action())
}

fn context_button_with_widget(label: &str, action: impl Fn(&gtk::Button) + 'static) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class("flat");
    button.set_halign(gtk::Align::Fill);
    button.connect_clicked(move |button| {
        action(button);
        if let Some(popover) = button
            .ancestor(gtk::Popover::static_type())
            .and_then(|widget| widget.downcast::<gtk::Popover>().ok())
        {
            popover.popdown();
        }
    });
    button
}

fn escape(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn relative_time(secs: u64, now: u64) -> String {
    if secs == 0 || secs > now {
        return String::new();
    }
    let diff = now - secs;
    if diff < 60 {
        return "now".to_string();
    }
    if diff < 3600 {
        return format!("{}m", diff / 60);
    }
    if diff < 86_400 {
        return format!("{}h", diff / 3600);
    }
    if diff < 86_400 * 7 {
        return format!("{}d", diff / 86_400);
    }
    if diff < 86_400 * 30 {
        return format!("{}w", diff / (86_400 * 7));
    }
    if diff < 86_400 * 365 {
        return format!("{}mo", diff / (86_400 * 30));
    }
    format!("{}y", diff / (86_400 * 365))
}
