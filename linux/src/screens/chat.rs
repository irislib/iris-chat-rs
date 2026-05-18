use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use adw::prelude::*;
use iris_chat_core::{
    peer_input_to_npub, proxied_image_url, AppAction, AppState, ChatKind, ChatMessageKind,
    ChatMessageSnapshot, ChatThreadSnapshot, CurrentChatSnapshot, DeliveryState,
    MessageAttachmentSnapshot, MessageReactionSnapshot, MessageReactor,
    MessageRecipientDeliverySnapshot, OutgoingAttachment, PreferencesSnapshot,
};

use crate::app_manager::AppManager;
use crate::screens::chat_list::{relative_time, unix_now};
use crate::widgets::image_cache;

#[derive(Clone)]
pub struct ChatInfoSnapshot {
    pub chat_id: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub profile_name: Option<String>,
    pub subtitle: Option<String>,
    pub picture_url: Option<String>,
    pub about: Option<String>,
    pub is_muted: bool,
    pub show_message_action: bool,
    pub preferences: PreferencesSnapshot,
}

struct ParticipantInfo {
    owner_pubkey_hex: Option<String>,
    name: String,
    picture_url: Option<String>,
    is_me: bool,
}

pub fn render(chat_id: &str, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.set_vexpand(true);

    let Some(chat) = state.current_chat.as_ref().filter(|c| c.chat_id == chat_id) else {
        let placeholder = gtk::Label::new(Some("Loading chat…"));
        placeholder.add_css_class("dim-label");
        placeholder.set_vexpand(true);
        container.append(&placeholder);
        return container.upcast();
    };

    mark_visible_seen(chat, manager);

    container.append(&ttl_strip(chat, manager));
    container.append(&messages_view(chat, &state.preferences, manager));
    container.append(&composer(chat, state, manager));

    container.upcast()
}

pub fn present_chat_info(
    parent: Option<&gtk::Window>,
    info: ChatInfoSnapshot,
    manager: Rc<AppManager>,
) {
    let dialog = adw::Dialog::builder()
        .title(&info.display_name)
        .content_width(360)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(24);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let header_row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    header_row.set_halign(gtk::Align::Start);

    let avatar = adw::Avatar::new(72, Some(&info.display_name), true);
    if let Some(url) = info.picture_url.as_deref() {
        if url.starts_with("http://") || url.starts_with("https://") {
            let proxied = proxied_image_url(
                url.to_string(),
                info.preferences.clone(),
                Some(144),
                Some(144),
                true,
            );
            image_cache::fetch_into_avatar(&avatar, &proxied);
        }
    }
    header_row.append(&avatar);

    let text_column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text_column.set_valign(gtk::Align::Center);

    let header = gtk::Label::new(Some(&info.display_name));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    header.set_xalign(0.0);
    header.set_wrap(true);
    text_column.append(&header);

    if let Some(subtitle) = info.subtitle.as_deref().filter(|s| !s.is_empty()) {
        let sub = gtk::Label::new(Some(subtitle));
        sub.add_css_class("dim-label");
        sub.set_wrap(true);
        sub.set_max_width_chars(36);
        sub.set_xalign(0.0);
        sub.set_halign(gtk::Align::Start);
        text_column.append(&sub);
    }
    header_row.append(&text_column);
    content.append(&header_row);

    if let Some(about) = info
        .about
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        content.append(&profile_about_card(about));
    }

    let common_groups = manager.mutual_groups(&info.chat_id);
    if !common_groups.is_empty() {
        content.append(&common_groups_card(common_groups, &dialog, manager.clone()));
    }

    content.append(&nickname_card(&info, manager.clone()));

    if info.show_message_action {
        let message = gtk::Button::with_label("Message");
        message.set_halign(gtk::Align::Start);
        let manager_for_message = manager.clone();
        let chat_id_for_message = info.chat_id.clone();
        let dialog_for_message = dialog.clone();
        message.connect_clicked(move |_| {
            manager_for_message.dispatch(AppAction::OpenChat {
                chat_id: chat_id_for_message.clone(),
            });
            dialog_for_message.close();
        });
        content.append(&message);
    }

    let mute = gtk::Button::with_label(if info.is_muted {
        "Unmute chat"
    } else {
        "Mute chat"
    });
    mute.set_halign(gtk::Align::Start);
    let manager_for_mute = manager.clone();
    let chat_id_for_mute = info.chat_id.clone();
    let muted_for_mute = info.is_muted;
    mute.connect_clicked(move |_| {
        manager_for_mute.dispatch(AppAction::SetChatMuted {
            chat_id: chat_id_for_mute.clone(),
            muted: !muted_for_mute,
        });
    });
    content.append(&mute);

    let delete = gtk::Button::with_label("Delete chat");
    delete.add_css_class("destructive-action");
    delete.set_halign(gtk::Align::Start);
    let manager_for_delete = manager.clone();
    let chat_id_for_delete = info.chat_id.clone();
    let dialog_for_delete = dialog.clone();
    delete.connect_clicked(move |_| {
        manager_for_delete.dispatch(AppAction::DeleteChat {
            chat_id: chat_id_for_delete.clone(),
        });
        dialog_for_delete.close();
    });
    content.append(&delete);

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn profile_about_card(about: &str) -> gtk::Widget {
    let group = adw::PreferencesGroup::new();
    let row = adw::PreferencesRow::new();
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    let icon = gtk::Image::from_icon_name("document-edit-symbolic");
    icon.set_valign(gtk::Align::Start);
    body.append(&icon);

    let label = gtk::Label::new(None);
    label.set_markup(&linkified_about_markup(about));
    label.set_wrap(true);
    label.set_lines(3);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_selectable(true);
    body.append(&label);
    row.set_child(Some(&body));
    group.add(&row);
    group.upcast()
}

fn linkified_about_markup(text: &str) -> String {
    let mut markup = String::new();
    for part in text.split_inclusive(char::is_whitespace) {
        let token = part.trim_end();
        let whitespace = &part[token.len()..];
        if token.starts_with("https://")
            || token.starts_with("http://")
            || token.starts_with("www.")
        {
            let href = if token.starts_with("www.") {
                format!("https://{token}")
            } else {
                token.to_string()
            };
            markup.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                glib::markup_escape_text(&href),
                glib::markup_escape_text(token)
            ));
        } else {
            markup.push_str(&glib::markup_escape_text(token));
        }
        markup.push_str(&glib::markup_escape_text(whitespace));
    }
    markup
}

fn nickname_card(info: &ChatInfoSnapshot, manager: Rc<AppManager>) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Nickname").build();

    let stored_nickname = info
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let nickname_row = adw::ActionRow::builder()
        .title("Nickname")
        .activatable(true)
        .build();
    if let Some(nickname) = stored_nickname.as_deref() {
        nickname_row.set_subtitle(nickname);
    }
    let info_for_edit = info.clone();
    nickname_row.connect_activated(move |row| {
        let parent = row
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        present_nickname_editor(parent.as_ref(), &info_for_edit, manager.clone());
    });

    group.add(&nickname_row);

    let primary_name = info
        .nickname
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&info.display_name);
    if let Some(profile_name) = info
        .profile_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case(primary_name.trim()))
    {
        let profile_row = adw::ActionRow::builder()
            .title("Profile name")
            .subtitle(profile_name)
            .build();
        group.add(&profile_row);
    }

    group.upcast()
}

fn present_nickname_editor(
    parent: Option<&gtk::Window>,
    info: &ChatInfoSnapshot,
    manager: Rc<AppManager>,
) {
    let dialog = adw::Dialog::builder()
        .title("Nickname")
        .content_width(360)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let nickname_row = adw::EntryRow::builder().title("Nickname").build();
    nickname_row.set_text(info.nickname.as_deref().unwrap_or(""));
    content.append(&nickname_row);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let manager_for_save = manager.clone();
    let row_for_save = nickname_row.clone();
    let dialog_for_save = dialog.clone();
    let chat_id_for_save = info.chat_id.clone();
    save.connect_clicked(move |_| {
        manager_for_save.dispatch(AppAction::SetContactNickname {
            owner_pubkey_hex: chat_id_for_save.clone(),
            nickname: row_for_save.text().trim().to_string(),
        });
        dialog_for_save.close();
    });
    actions.append(&save);

    if info
        .nickname
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        let remove = gtk::Button::with_label("Remove");
        let manager_for_remove = manager.clone();
        let dialog_for_remove = dialog.clone();
        let chat_id_for_remove = info.chat_id.clone();
        remove.connect_clicked(move |_| {
            manager_for_remove.dispatch(AppAction::SetContactNickname {
                owner_pubkey_hex: chat_id_for_remove.clone(),
                nickname: String::new(),
            });
            dialog_for_remove.close();
        });
        actions.append(&remove);
    }
    content.append(&actions);

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn common_groups_card(
    groups: Vec<ChatThreadSnapshot>,
    dialog: &adw::Dialog,
    manager: Rc<AppManager>,
) -> gtk::Widget {
    let section = adw::PreferencesGroup::builder()
        .title("Groups in common")
        .build();

    for group in groups {
        let title = if group.display_name.trim().is_empty() {
            "Group".to_string()
        } else {
            group.display_name.clone()
        };
        let row = adw::ActionRow::builder()
            .title(title)
            .activatable(true)
            .build();
        row.set_subtitle(&format!("{} people", group.member_count));
        let avatar = adw::Avatar::new(32, Some(&group.display_name), true);
        row.add_prefix(&avatar);
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class("dim-label");
        row.add_suffix(&chevron);

        let manager_for_row = manager.clone();
        let dialog_for_row = dialog.clone();
        let chat_id = group.chat_id.clone();
        row.connect_activated(move |_| {
            let Some(group_id) = group_id_from_chat_id(&chat_id) else {
                return;
            };
            dialog_for_row.close();
            manager_for_row.dispatch(AppAction::PushScreen {
                screen: iris_chat_core::Screen::GroupDetails { group_id },
            });
        });
        section.add(&row);
    }

    section.upcast()
}

fn group_id_from_chat_id(chat_id: &str) -> Option<String> {
    let trimmed = chat_id.trim();
    let prefix = "group:";
    let starts_with_group = trimmed
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix));
    if !starts_with_group {
        return None;
    }
    let group_id = trimmed[prefix.len()..].trim();
    if group_id.is_empty() {
        None
    } else {
        Some(group_id.to_string())
    }
}

fn present_message_info(
    parent: Option<&gtk::Window>,
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
    manager: &Rc<AppManager>,
) {
    let dialog = adw::Dialog::builder()
        .title("Message Details")
        .content_width(420)
        .content_height(560)
        .build();

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_vscrollbar_policy(gtk::PolicyType::Automatic);
    scroll.set_propagate_natural_height(true);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let header_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header_row.set_halign(gtk::Align::Start);
    let glyph = gtk::Label::new(Some(delivery_glyph(&message.delivery)));
    glyph.add_css_class("title-2");
    header_row.append(&glyph);
    let status_label = gtk::Label::new(Some(delivery_label(&message.delivery)));
    status_label.add_css_class("title-3");
    status_label.set_xalign(0.0);
    header_row.append(&status_label);
    let header_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_spacer.set_hexpand(true);
    header_row.append(&header_spacer);
    let copy_all = gtk::Button::with_label("Copy info");
    copy_all.add_css_class("flat");
    let info_text = message_info_text(message, Some(chat));
    copy_all.connect_clicked(move |_| {
        crate::platform::clipboard::copy(&info_text);
    });
    header_row.append(&copy_all);
    content.append(&header_row);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    // Status section
    let status_section = info_section("Status");
    info_value_row(
        &status_section,
        "Time",
        &message_info_date_time(message.created_at_secs),
    );
    if let Some(expires) = message.expires_at_secs {
        info_value_row(&status_section, "Deletes", &message_info_date_time(expires));
    }
    info_value_row(&status_section, "Type", message_info_kind(message));
    content.append(&status_section);

    let state = manager.current_state();

    // People
    let people_section = info_section("People");
    if message.is_outgoing {
        if message.recipient_deliveries.is_empty() {
            if matches!(chat.kind, ChatKind::Direct) {
                let info = direct_recipient_info(chat);
                info_recipient_row(
                    &people_section,
                    &info,
                    &format!("{} · No receipt", delivery_label(&message.delivery)),
                    Some(&message.delivery),
                    &state.preferences,
                    manager,
                    &dialog,
                );
            } else {
                info_value_row(&people_section, "Recipients", "No receipts");
            }
        } else {
            for recipient in &message.recipient_deliveries {
                let info = recipient_info(recipient, chat);
                info_recipient_row(
                    &people_section,
                    &info,
                    &format!(
                        "{} · {}",
                        delivery_label(&recipient.delivery),
                        message_info_date_time(recipient.updated_at_secs)
                    ),
                    Some(&recipient.delivery),
                    &state.preferences,
                    manager,
                    &dialog,
                );
            }
        }
    } else {
        let info = message_author_info(message, chat);
        info_recipient_row(
            &people_section,
            &info,
            &format!(
                "{} · {}",
                delivery_label(&message.delivery),
                message_info_date_time(message.created_at_secs)
            ),
            Some(&message.delivery),
            &state.preferences,
            manager,
            &dialog,
        );
    }
    content.append(&people_section);

    // Transport
    let trace = &message.delivery_trace;
    let channels: Vec<String> = trace
        .transport_channels
        .iter()
        .map(|c| pretty_transport_channel(c))
        .collect();
    let queued_device_npubs: Vec<String> = trace
        .queued_protocol_targets
        .iter()
        .map(|id| short_npub(id))
        .collect();
    let has_transport = !channels.is_empty()
        || !queued_device_npubs.is_empty()
        || trace
            .last_transport_error
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false);
    if has_transport {
        let transport_section = info_section("Transport");
        if !channels.is_empty() {
            info_multivalue_row(
                &transport_section,
                if message.is_outgoing {
                    "Sent over"
                } else {
                    "Received over"
                },
                &channels,
                false,
            );
        }
        if !trace.pending_relay_event_ids.is_empty() {
            let shortened: Vec<String> = trace
                .pending_relay_event_ids
                .iter()
                .map(|id| short_message_identifier(id))
                .collect();
            info_multivalue_row(
                &transport_section,
                "Pending message servers",
                &shortened,
                true,
            );
        }
        if !queued_device_npubs.is_empty() {
            info_multivalue_row(
                &transport_section,
                "Queued devices",
                &queued_device_npubs,
                true,
            );
        }
        if let Some(error) = trace
            .last_transport_error
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            info_value_row(&transport_section, "Last error", error);
        }
        content.append(&transport_section);
    }

    // IDs
    let ids_section = info_section("IDs");
    info_copy_row(&ids_section, "Message", &message.id, true);
    if let Some(source_event_id) = message
        .source_event_id
        .as_deref()
        .filter(|id| !id.is_empty())
    {
        info_copy_row(&ids_section, "Received event", source_event_id, true);
    }
    for (idx, value) in trace.outer_event_ids.iter().enumerate() {
        info_copy_row(
            &ids_section,
            if idx == 0 { "Network events" } else { "" },
            value,
            true,
        );
    }
    for (idx, value) in trace.target_device_ids.iter().enumerate() {
        let npub = peer_input_to_npub(value.clone());
        let display = if npub.is_empty() { value.clone() } else { npub };
        info_copy_row(
            &ids_section,
            if idx == 0 { "Target devices" } else { "" },
            &display,
            true,
        );
    }
    content.append(&ids_section);

    // Attachments
    if !message.attachments.is_empty() {
        let attach_section = info_section("Attachments");
        for attachment in &message.attachments {
            let label = if attachment.filename.is_empty() {
                "File".to_string()
            } else {
                attachment.filename.clone()
            };
            info_copy_row(&attach_section, &label, &attachment.htree_url, true);
        }
        content.append(&attach_section);
    }

    // Reactions
    if !message.reactions.is_empty() || !message.reactors.is_empty() {
        let react_section = info_section("Reactions");
        for reaction in &message.reactions {
            info_value_row(&react_section, &reaction.emoji, &reaction.count.to_string());
        }
        for reactor in &message.reactors {
            let value = if reactor.emoji.is_empty() {
                "Removed".to_string()
            } else {
                reactor.emoji.clone()
            };
            let info = reactor_info(reactor, chat);
            info_recipient_row(
                &react_section,
                &info,
                &value,
                None,
                &state.preferences,
                manager,
                &dialog,
            );
        }
        content.append(&react_section);
    }

    // Inner rumor — synthesized from the snapshot so the same shape
    // shows up across platforms. The `id` field matches the rumor hash
    // for messages received as runtime rumors; pubkey is best-effort.
    let rumor_section = info_section("Inner rumor");
    let rumor_json = synthesize_message_rumor_json(message, chat, &state);
    let rumor_label = gtk::Label::new(Some(&rumor_json));
    rumor_label.set_wrap(true);
    rumor_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    rumor_label.set_xalign(0.0);
    rumor_label.add_css_class("monospace");
    rumor_label.set_selectable(true);
    rumor_label.set_margin_start(12);
    rumor_label.set_margin_end(12);
    rumor_label.set_margin_bottom(8);
    rumor_section.append(&rumor_label);
    let rumor_copy = gtk::Button::with_label("Copy rumor JSON");
    rumor_copy.add_css_class("flat");
    rumor_copy.set_halign(gtk::Align::Start);
    rumor_copy.set_margin_start(12);
    rumor_copy.set_margin_bottom(12);
    let rumor_for_copy = rumor_json.clone();
    rumor_copy.connect_clicked(move |_| {
        crate::platform::clipboard::copy(&rumor_for_copy);
    });
    rumor_section.append(&rumor_copy);
    content.append(&rumor_section);

    scroll.set_child(Some(&content));
    dialog.set_child(Some(&scroll));
    dialog.present(parent);
}

fn info_section(title: &str) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 4);
    section.add_css_class("card");
    section.set_margin_top(2);
    section.set_margin_bottom(2);
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("heading");
    title_label.set_xalign(0.0);
    title_label.set_margin_top(10);
    title_label.set_margin_bottom(2);
    title_label.set_margin_start(12);
    title_label.set_margin_end(12);
    section.append(&title_label);
    section
}

fn info_value_row(parent: &gtk::Box, label: &str, value: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_margin_top(2);
    row.set_margin_start(12);
    row.set_margin_end(12);
    let label_widget = gtk::Label::new(Some(label));
    label_widget.add_css_class("dim-label");
    label_widget.set_xalign(0.0);
    label_widget.set_width_chars(14);
    row.append(&label_widget);
    let value_widget = gtk::Label::new(Some(value));
    value_widget.set_xalign(0.0);
    value_widget.set_wrap(true);
    value_widget.set_selectable(true);
    value_widget.set_hexpand(true);
    row.append(&value_widget);
    parent.append(&row);
}

fn info_multivalue_row(parent: &gtk::Box, label: &str, values: &[String], monospace: bool) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_margin_top(2);
    column.set_margin_start(12);
    column.set_margin_end(12);
    let label_widget = gtk::Label::new(Some(label));
    label_widget.add_css_class("dim-label");
    label_widget.set_xalign(0.0);
    column.append(&label_widget);
    for value in values {
        let value_widget = gtk::Label::new(Some(value));
        value_widget.set_xalign(0.0);
        value_widget.set_wrap(true);
        value_widget.set_selectable(true);
        if monospace {
            value_widget.add_css_class("monospace");
        }
        column.append(&value_widget);
    }
    parent.append(&column);
}

fn info_copy_row(parent: &gtk::Box, label: &str, value: &str, monospace: bool) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_margin_top(2);
    row.set_margin_start(12);
    row.set_margin_end(12);
    let label_widget = gtk::Label::new(Some(label));
    label_widget.add_css_class("dim-label");
    label_widget.set_xalign(0.0);
    label_widget.set_width_chars(14);
    row.append(&label_widget);
    let display = short_message_identifier(value);
    let value_widget = gtk::Label::new(Some(&display));
    value_widget.set_xalign(0.0);
    value_widget.set_wrap(true);
    value_widget.set_selectable(true);
    value_widget.set_hexpand(true);
    if monospace {
        value_widget.add_css_class("monospace");
    }
    row.append(&value_widget);
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.add_css_class("flat");
    let value_owned = value.to_string();
    copy.connect_clicked(move |_| {
        crate::platform::clipboard::copy(&value_owned);
    });
    row.append(&copy);
    parent.append(&row);
}

fn info_recipient_row(
    parent: &gtk::Box,
    info: &ParticipantInfo,
    subtitle: &str,
    delivery: Option<&DeliveryState>,
    preferences: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
    dialog: &adw::Dialog,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    row.set_margin_start(12);
    row.set_margin_end(12);
    if let Some(owner_pubkey_hex) = info
        .owner_pubkey_hex
        .as_ref()
        .filter(|owner| !owner.is_empty() && !info.is_me)
    {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        let manager_for_click = manager.clone();
        let dialog_for_click = dialog.clone();
        let peer_input = owner_pubkey_hex.clone();
        click.connect_released(move |_, _, _, _| {
            dialog_for_click.close();
            manager_for_click.dispatch(AppAction::CreateChat {
                peer_input: peer_input.clone(),
            });
        });
        row.add_controller(click);
    }

    let avatar = adw::Avatar::new(32, Some(&info.name), true);
    if let Some(url) = info.picture_url.as_deref() {
        if url.starts_with("http://") || url.starts_with("https://") {
            let proxied = proxied_image_url(
                url.to_string(),
                preferences.clone(),
                Some(64),
                Some(64),
                true,
            );
            image_cache::fetch_into_avatar(&avatar, &proxied);
        }
    }
    row.append(&avatar);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 1);
    column.set_hexpand(true);
    let title_widget = gtk::Label::new(Some(&info.name));
    title_widget.set_xalign(0.0);
    title_widget.add_css_class("body");
    title_widget.set_ellipsize(gtk::pango::EllipsizeMode::End);
    column.append(&title_widget);
    let subtitle_widget = gtk::Label::new(Some(subtitle));
    subtitle_widget.add_css_class("caption");
    subtitle_widget.add_css_class("dim-label");
    subtitle_widget.set_xalign(0.0);
    column.append(&subtitle_widget);
    row.append(&column);
    if let Some(delivery) = delivery {
        let glyph = gtk::Label::new(Some(delivery_glyph(delivery)));
        row.append(&glyph);
    }
    parent.append(&row);
}

fn ttl_strip(chat: &CurrentChatSnapshot, manager: &Rc<AppManager>) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_start(12);
    row.set_margin_end(12);
    row.set_margin_top(6);
    row.set_halign(gtk::Align::End);

    let label = match chat.message_ttl_seconds {
        None | Some(0) => "No expiry".to_string(),
        Some(s) if s < 3600 => format!("Expires {}m", s / 60),
        Some(s) if s < 86_400 => format!("Expires {}h", s / 3600),
        Some(s) if s < 86_400 * 7 => format!("Expires {}d", s / 86_400),
        Some(s) => format!("Expires {}w", s / (86_400 * 7)),
    };

    let menu_button = gtk::MenuButton::new();
    menu_button.set_label(&label);
    menu_button.add_css_class("flat");
    menu_button.add_css_class("caption");

    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.set_margin_start(6);
    list.set_margin_end(6);

    let options: &[(&str, Option<u64>)] = &[
        ("No expiry", None),
        ("1 hour", Some(3600)),
        ("6 hours", Some(6 * 3600)),
        ("1 day", Some(86_400)),
        ("1 week", Some(7 * 86_400)),
    ];
    for (option_label, ttl) in options {
        let item = gtk::Button::with_label(option_label);
        item.add_css_class("flat");
        item.set_halign(gtk::Align::Fill);
        let manager = manager.clone();
        let chat_id = chat.chat_id.clone();
        let ttl_value = *ttl;
        let popover_for_close = popover.clone();
        item.connect_clicked(move |_| {
            manager.dispatch(AppAction::SetChatMessageTtl {
                chat_id: chat_id.clone(),
                ttl_seconds: ttl_value,
            });
            popover_for_close.popdown();
        });
        list.append(&item);
    }
    popover.set_child(Some(&list));
    menu_button.set_popover(Some(&popover));

    row.append(&menu_button);
    row.upcast()
}

fn messages_view(
    chat: &CurrentChatSnapshot,
    prefs: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_vexpand(true);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_vexpand(true);
    list.set_valign(gtk::Align::End);
    list.set_margin_top(8);
    list.set_margin_bottom(8);
    list.set_margin_start(10);
    list.set_margin_end(10);

    if chat.messages.is_empty() {
        let empty = gtk::Label::new(Some("No messages yet"));
        empty.add_css_class("dim-label");
        empty.set_vexpand(true);
        empty.set_valign(gtk::Align::Center);
        list.append(&empty);
    } else {
        let mut last_day: Option<String> = None;
        let mut last_author: Option<String> = None;
        let mut last_outgoing = false;
        let mut last_secs: u64 = 0;

        let now = unix_now();

        for (idx, message) in chat.messages.iter().enumerate() {
            let day = day_label_secs(message.created_at_secs);
            let same_day = matches!(last_day.as_deref(), Some(d) if d == day);
            if !same_day {
                list.append(&day_chip(&day));
                last_author = None;
            }
            last_day = Some(day);

            let cluster_break = !matches!(&last_author, Some(a) if a == &message.author)
                || last_outgoing != message.is_outgoing
                || message.created_at_secs.saturating_sub(last_secs) > 300;

            let is_last = idx + 1 == chat.messages.len();
            let next_message = chat.messages.get(idx + 1);
            let cluster_ends = match next_message {
                Some(next) => {
                    next.author != message.author
                        || next.is_outgoing != message.is_outgoing
                        || next.created_at_secs.saturating_sub(message.created_at_secs) > 300
                        || day_label_secs(next.created_at_secs)
                            != day_label_secs(message.created_at_secs)
                }
                None => true,
            };

            list.append(&render_message(
                message,
                chat,
                cluster_break,
                cluster_ends,
                is_last,
                now,
                prefs,
                manager,
            ));

            last_author = Some(message.author.clone());
            last_outgoing = message.is_outgoing;
            last_secs = message.created_at_secs;
        }
    }

    scrolled.set_child(Some(&list));

    let adj = scrolled.vadjustment();
    adj.connect_changed(|adj| {
        let bottom = (adj.upper() - adj.page_size()).max(adj.lower());
        adj.set_value(bottom);
    });
    glib::idle_add_local_once(move || {
        let bottom = (adj.upper() - adj.page_size()).max(adj.lower());
        adj.set_value(bottom);
    });

    scrolled.upcast()
}

pub(crate) fn mark_visible_seen(chat: &CurrentChatSnapshot, manager: &Rc<AppManager>) {
    if !manager.can_mark_active_chat_seen() {
        return;
    }
    let unseen: Vec<String> = chat
        .messages
        .iter()
        .filter(|m| {
            !m.is_outgoing
                && matches!(m.kind, ChatMessageKind::User)
                && !matches!(m.delivery, DeliveryState::Seen)
        })
        .map(|m| m.id.clone())
        .collect();
    if unseen.is_empty() {
        return;
    }
    manager.dispatch(AppAction::MarkMessagesSeen {
        chat_id: chat.chat_id.clone(),
        message_ids: unseen,
    });
}

fn render_message(
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
    cluster_start: bool,
    cluster_end: bool,
    _is_last: bool,
    now: u64,
    prefs: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    if matches!(message.kind, ChatMessageKind::System) {
        let label = gtk::Label::new(Some(&message.body));
        label.add_css_class("dim-label");
        label.add_css_class("caption");
        label.set_halign(gtk::Align::Center);
        label.set_wrap(true);
        label.set_selectable(true);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        return label.upcast();
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_hexpand(true);
    row.set_margin_top(if cluster_start { 8 } else { 2 });
    row.set_margin_bottom(if cluster_end { 4 } else { 0 });

    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_hexpand(false);

    if matches!(chat.kind, ChatKind::Group) && !message.is_outgoing && cluster_start {
        let author = gtk::Label::new(Some(&message.author));
        author.add_css_class("chat-author");
        author.set_halign(gtk::Align::Start);
        column.append(&author);
    }

    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 2);
    bubble.add_css_class(if message.is_outgoing {
        "bubble-out"
    } else {
        "bubble-in"
    });
    bubble.set_halign(if message.is_outgoing {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });

    let image_attachments: Vec<MessageAttachmentSnapshot> = message
        .attachments
        .iter()
        .filter(|a| a.is_image)
        .cloned()
        .collect();
    let other_attachments: Vec<&MessageAttachmentSnapshot> =
        message.attachments.iter().filter(|a| !a.is_image).collect();

    if !image_attachments.is_empty() {
        bubble.append(&image_album(&image_attachments, prefs, manager));
    }

    if !message.body.is_empty() {
        append_truncatable_body(&bubble, &message.body);
    }

    if !other_attachments.is_empty() {
        bubble.append(&attachment_summary_widget(&other_attachments, manager));
    }

    if cluster_end {
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        footer.add_css_class("bubble-meta");
        let time = gtk::Label::new(Some(&relative_time(message.created_at_secs, now)));
        footer.append(&time);
        if message.is_outgoing {
            let glyph = gtk::Label::new(Some(delivery_glyph(&message.delivery)));
            footer.append(&glyph);
        }
        footer.set_halign(gtk::Align::End);
        footer.set_margin_top(2);
        bubble.append(&footer);
    }

    let popover = build_message_popover(message, chat, manager);
    popover.set_parent(&bubble);
    let popover_for_gesture = popover.clone();
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    gesture.connect_pressed(move |_, _, x, y| {
        popover_for_gesture
            .set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover_for_gesture.popup();
    });
    bubble.add_controller(gesture);

    let popover_for_long = popover.clone();
    let long_press = gtk::GestureLongPress::new();
    long_press.connect_pressed(move |_, x, y| {
        popover_for_long.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover_for_long.popup();
    });
    bubble.add_controller(long_press);

    column.append(&bubble);

    if !message.reactions.is_empty() {
        column.append(&reactions_row(message, &message.reactions, manager));
    }

    if message.is_outgoing {
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        row.append(&column);
    } else {
        row.append(&column);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
    }

    row.upcast()
}

// Cap tall message bodies behind a Show more/less toggle. The
// char-and-newline heuristic plus a hard `lines` cap means a single
// pathological glyph stream still gets ellipsized — Pango won't render
// past the lines limit even if the chars-per-line is unusual.
fn append_truncatable_body(bubble: &gtk::Box, body_text: &str) {
    const COLLAPSED_LINES: i32 = 14;
    const LONG_CHAR_THRESHOLD: usize = 600;
    const LONG_NEWLINE_THRESHOLD: usize = 14;

    let body = gtk::Label::new(Some(body_text));
    body.set_wrap(true);
    body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    body.set_xalign(0.0);
    body.set_max_width_chars(40);
    body.set_selectable(true);
    if let Some(count) = jumbomoji_count(body_text) {
        body.add_css_class(&format!("bubble-jumbomoji-{count}"));
    }

    let long = body_text.chars().count() > LONG_CHAR_THRESHOLD
        || body_text.matches('\n').count() >= LONG_NEWLINE_THRESHOLD;
    if !long {
        bubble.append(&body);
        return;
    }

    body.set_ellipsize(gtk::pango::EllipsizeMode::End);
    body.set_lines(COLLAPSED_LINES);
    bubble.append(&body);

    let toggle = gtk::Button::with_label("Show more");
    toggle.add_css_class("flat");
    // Intentionally NOT `.link`: Adwaita's `.link` style colours the
    // button with `@accent_color`, which in our palette is the brand
    // purple. Text/icons in chat content are never purple — the
    // toggle stays as a plain flat button and inherits the bubble's
    // muted foreground via the `.bubble-toggle` rule in main.rs.
    toggle.add_css_class("bubble-toggle");
    toggle.set_halign(gtk::Align::Start);

    let expanded = Rc::new(std::cell::Cell::new(false));
    let label_for_click = body.clone();
    let expanded_for_click = Rc::clone(&expanded);
    toggle.connect_clicked(move |btn| {
        let next = !expanded_for_click.get();
        expanded_for_click.set(next);
        if next {
            label_for_click.set_lines(-1);
            label_for_click.set_ellipsize(gtk::pango::EllipsizeMode::None);
            btn.set_label("Show less");
        } else {
            label_for_click.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label_for_click.set_lines(COLLAPSED_LINES);
            btn.set_label("Show more");
        }
    });
    bubble.append(&toggle);
}

fn jumbomoji_count(text: &str) -> Option<usize> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut count = 0usize;
    let mut cluster_open = false;
    let mut last_was_joiner = false;
    for ch in trimmed.chars() {
        let code = ch as u32;
        if ch.is_whitespace() {
            cluster_open = false;
            last_was_joiner = false;
        } else if is_emoji_continuation(code) {
            if !cluster_open {
                return None;
            }
            last_was_joiner = code == 0x200D;
        } else if is_emoji_base(code) {
            if !cluster_open || !last_was_joiner {
                count += 1;
                if count > 5 {
                    return None;
                }
            }
            cluster_open = true;
            last_was_joiner = false;
        } else {
            return None;
        }
    }

    (count > 0).then_some(count)
}

fn is_emoji_continuation(code: u32) -> bool {
    code == 0x200D || code == 0xFE0F || (0x1F3FB..=0x1F3FF).contains(&code)
}

fn is_emoji_base(code: u32) -> bool {
    (0x1F000..=0x1FAFF).contains(&code) || (0x2600..=0x27BF).contains(&code)
}

fn build_message_popover(
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Top);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_margin_top(6);
    column.set_margin_bottom(6);
    column.set_margin_start(6);
    column.set_margin_end(6);

    let reactions_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    for emoji in reaction_picker_emojis() {
        let btn = gtk::Button::with_label(&emoji);
        btn.add_css_class("flat");
        btn.add_css_class("circular");
        let manager = manager.clone();
        let chat_id = message.chat_id.clone();
        let message_id = message.id.clone();
        let emoji_owned = emoji.to_string();
        let popover_for_close = popover.clone();
        btn.connect_clicked(move |_| {
            remember_reaction_emoji(&emoji_owned);
            manager.dispatch(AppAction::ToggleReaction {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                emoji: emoji_owned.clone(),
            });
            popover_for_close.popdown();
        });
        reactions_row.append(&btn);
    }
    let more = gtk::Button::from_icon_name("list-add-symbolic");
    more.add_css_class("flat");
    more.add_css_class("circular");
    more.set_tooltip_text(Some("More emoji"));
    let chooser = build_reaction_emoji_popover(message, manager, &popover);
    chooser.set_parent(&more);
    {
        let chooser_for_click = chooser.clone();
        more.connect_clicked(move |_| chooser_for_click.popup());
    }
    reactions_row.append(&more);
    column.append(&reactions_row);

    let forward = gtk::Button::with_label("Forward");
    forward.add_css_class("flat");
    forward.set_halign(gtk::Align::Fill);
    let forward_message = message.clone();
    let manager_for_forward = manager.clone();
    let popover_for_forward = popover.clone();
    forward.connect_clicked(move |btn| {
        let parent = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        present_forward_dialog(
            parent.as_ref(),
            &forwardable_message_text(&forward_message),
            &manager_for_forward,
        );
        popover_for_forward.popdown();
    });
    column.append(&forward);

    let copy = gtk::Button::with_label("Copy text");
    copy.add_css_class("flat");
    copy.set_halign(gtk::Align::Fill);
    let body = message.body.clone();
    let popover_for_copy = popover.clone();
    copy.connect_clicked(move |_| {
        crate::platform::clipboard::copy(&body);
        popover_for_copy.popdown();
    });
    column.append(&copy);

    let info_btn = gtk::Button::with_label("Info");
    info_btn.add_css_class("flat");
    info_btn.set_halign(gtk::Align::Fill);
    let popover_for_info = popover.clone();
    let info_message = message.clone();
    let info_chat = chat.clone();
    let manager_for_info = manager.clone();
    info_btn.connect_clicked(move |btn| {
        let parent = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        present_message_info(
            parent.as_ref(),
            &info_message,
            &info_chat,
            &manager_for_info,
        );
        popover_for_info.popdown();
    });
    column.append(&info_btn);

    let delete = gtk::Button::with_label("Delete locally");
    delete.add_css_class("flat");
    delete.add_css_class("error");
    delete.set_halign(gtk::Align::Fill);
    let manager_for_delete = manager.clone();
    let chat_id = message.chat_id.clone();
    let message_id = message.id.clone();
    let popover_for_delete = popover.clone();
    delete.connect_clicked(move |_| {
        manager_for_delete.dispatch(AppAction::DeleteLocalMessage {
            chat_id: chat_id.clone(),
            message_id: message_id.clone(),
        });
        popover_for_delete.popdown();
    });
    column.append(&delete);

    popover.set_child(Some(&column));
    popover
}

fn build_reaction_emoji_popover(
    message: &ChatMessageSnapshot,
    manager: &Rc<AppManager>,
    owner_popover: &gtk::Popover,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Top);

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_min_content_width(280);
    scrolled.set_min_content_height(320);
    scrolled.set_max_content_height(420);
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.set_margin_top(8);
    column.set_margin_bottom(8);
    column.set_margin_start(8);
    column.set_margin_end(8);

    let mut shown = Vec::new();
    let message_emojis = message_reaction_emojis(message);
    if !message_emojis.is_empty() {
        append_reaction_emoji_section(
            &column,
            "This message",
            &message_emojis,
            message,
            manager,
            &popover,
            owner_popover,
        );
        shown.extend(message_emojis);
    }

    let recent: Vec<String> = recent_reaction_emojis()
        .into_iter()
        .filter(|emoji| !shown.contains(emoji))
        .collect();
    if !recent.is_empty() {
        append_reaction_emoji_section(
            &column,
            "Recent",
            &recent,
            message,
            manager,
            &popover,
            owner_popover,
        );
        shown.extend(recent);
    }

    for (name, emojis) in REACTION_EMOJI_CATEGORIES {
        let choices = emojis
            .iter()
            .map(|emoji| (*emoji).to_string())
            .collect::<Vec<_>>();
        append_reaction_emoji_section(
            &column,
            name,
            &choices,
            message,
            manager,
            &popover,
            owner_popover,
        );
    }

    scrolled.set_child(Some(&column));
    popover.set_child(Some(&scrolled));
    popover
}

fn append_reaction_emoji_section(
    column: &gtk::Box,
    title: &str,
    emojis: &[String],
    message: &ChatMessageSnapshot,
    manager: &Rc<AppManager>,
    picker_popover: &gtk::Popover,
    owner_popover: &gtk::Popover,
) {
    let header = gtk::Label::new(Some(title));
    header.add_css_class("caption");
    header.add_css_class("dim-label");
    header.set_xalign(0.0);
    column.append(&header);

    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(7);
    flow.set_min_children_per_line(4);
    flow.set_row_spacing(2);
    flow.set_column_spacing(2);

    for emoji in emojis {
        let btn = gtk::Button::with_label(emoji);
        btn.add_css_class("flat");
        btn.add_css_class("circular");
        let manager = manager.clone();
        let chat_id = message.chat_id.clone();
        let message_id = message.id.clone();
        let emoji = emoji.clone();
        let picker_for_close = picker_popover.clone();
        let owner_for_close = owner_popover.clone();
        btn.connect_clicked(move |_| {
            remember_reaction_emoji(&emoji);
            manager.dispatch(AppAction::ToggleReaction {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                emoji: emoji.clone(),
            });
            picker_for_close.popdown();
            owner_for_close.popdown();
        });
        flow.insert(&btn, -1);
    }

    column.append(&flow);
}

fn reactions_row(
    message: &ChatMessageSnapshot,
    reactions: &[MessageReactionSnapshot],
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.set_margin_start(8);
    row.set_margin_end(8);
    row.set_halign(if message.is_outgoing {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });
    for reaction in reactions {
        let chip = gtk::Button::with_label(&format!("{} {}", reaction.emoji, reaction.count));
        chip.add_css_class("pill");
        chip.add_css_class("flat");
        if reaction.reacted_by_me {
            chip.add_css_class("suggested-action");
        }
        let manager = manager.clone();
        let chat_id = message.chat_id.clone();
        let message_id = message.id.clone();
        let emoji = reaction.emoji.clone();
        chip.connect_clicked(move |_| {
            remember_reaction_emoji(&emoji);
            manager.dispatch(AppAction::ToggleReaction {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                emoji: emoji.clone(),
            });
        });
        row.append(&chip);
    }
    row.upcast()
}

const DEFAULT_REACTION_EMOJIS: [&str; 7] = ["❤️", "👍", "😂", "😮", "😢", "🙏", "🔥"];
const REACTION_EMOJI_CATEGORIES: &[(&str, &[&str])] = &[
    (
        "Smileys",
        &[
            "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "😊", "🙂", "🙃", "😉", "😍", "🥰",
            "😘", "😎", "🤩", "🥳", "😏", "😌", "😴", "🤔", "😐", "🙄", "😬", "🥺", "😢", "😭",
            "😠", "🤬", "😱", "🤗",
        ],
    ),
    (
        "Hearts",
        &[
            "❤️",
            "🧡",
            "💛",
            "💚",
            "💙",
            "💜",
            "🖤",
            "🤍",
            "🤎",
            "💖",
            "💗",
            "💓",
            "💕",
            "💔",
            "❤️‍🔥",
            "❤️‍🩹",
        ],
    ),
    (
        "Hands",
        &[
            "👍", "👎", "👌", "✌️", "🤞", "🤟", "🤘", "🤙", "👈", "👉", "👆", "👇", "☝️", "✋",
            "🤚", "👋", "🤝", "🙏", "👏", "🙌", "💪", "🫶",
        ],
    ),
    (
        "Symbols",
        &[
            "✅", "❌", "⭕", "🚫", "⚠️", "💯", "🔥", "✨", "⭐", "🌈", "☀️", "🌙", "⚡", "💥",
            "🎉", "🎊", "🎁", "🏆", "💤", "💭",
        ],
    ),
];
const RECENT_REACTION_EMOJI_LIMIT: usize = 16;
static RECENT_REACTION_EMOJIS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn reaction_picker_emojis() -> Vec<String> {
    DEFAULT_REACTION_EMOJIS
        .iter()
        .map(|emoji| (*emoji).to_string())
        .collect()
}

fn message_reaction_emojis(message: &ChatMessageSnapshot) -> Vec<String> {
    unique_reaction_emojis(
        message
            .reactions
            .iter()
            .map(|reaction| reaction.emoji.clone()),
    )
}

fn unique_reaction_emojis(emojis: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for emoji in emojis {
        let trimmed = emoji.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            result.push(trimmed.to_string());
        }
    }
    result
}

fn recent_reaction_emojis() -> Vec<String> {
    RECENT_REACTION_EMOJIS
        .get_or_init(|| Mutex::new(load_recent_reaction_emojis()))
        .lock()
        .map(|items| items.clone())
        .unwrap_or_default()
}

fn remember_reaction_emoji(emoji: &str) {
    let trimmed = emoji.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Ok(mut items) = RECENT_REACTION_EMOJIS
        .get_or_init(|| Mutex::new(load_recent_reaction_emojis()))
        .lock()
    {
        items.retain(|item| item != trimmed);
        items.insert(0, trimmed.to_string());
        items.truncate(RECENT_REACTION_EMOJI_LIMIT);
        save_recent_reaction_emojis(&items);
    }
}

fn load_recent_reaction_emojis() -> Vec<String> {
    let Some(path) = recent_reaction_emojis_path() else {
        return Vec::new();
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    unique_reaction_emojis(contents.lines().map(str::to_string))
        .into_iter()
        .take(RECENT_REACTION_EMOJI_LIMIT)
        .collect()
}

fn save_recent_reaction_emojis(emojis: &[String]) {
    let Some(path) = recent_reaction_emojis_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, emojis.join("\n"));
}

fn recent_reaction_emojis_path() -> Option<std::path::PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })?;
    Some(config_home.join("iris-chat").join("recent-reactions.txt"))
}

fn day_chip(label: &str) -> gtk::Widget {
    let chip = gtk::Label::new(Some(label));
    chip.add_css_class("chat-day");
    chip.set_halign(gtk::Align::Center);
    chip.set_margin_top(12);
    chip.set_margin_bottom(6);
    chip.upcast()
}

fn day_label_secs(secs: u64) -> String {
    let now = unix_now();
    if secs == 0 || secs > now {
        return "—".to_string();
    }
    let now_day = now / 86_400;
    let secs_day = secs / 86_400;
    let diff = now_day.saturating_sub(secs_day);
    match diff {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        2..=6 => format!("{} days ago", diff),
        _ => format!("{}d ago", diff),
    }
}

fn delivery_glyph(state: &DeliveryState) -> &'static str {
    use DeliveryState::*;
    match state {
        Queued => "⋯",
        Pending => "⋯",
        Sent => "✓",
        Received => "✓✓",
        Seen => "✓✓",
        Failed => "!",
    }
}

fn delivery_label(state: &DeliveryState) -> &'static str {
    use DeliveryState::*;
    match state {
        Queued => "Queued",
        Pending => "Pending",
        Sent => "Sent",
        Received => "Received",
        Seen => "Seen",
        Failed => "Failed",
    }
}

fn message_info_text(message: &ChatMessageSnapshot, chat: Option<&CurrentChatSnapshot>) -> String {
    let trace = &message.delivery_trace;
    let mut lines = vec![
        format!("Message {}", message.id),
        format!("Time {}", message_info_date_time(message.created_at_secs)),
        format!("Type {}", message_info_kind(message)),
        format!("Status {}", delivery_label(&message.delivery)),
    ];
    if let Some(expires) = message.expires_at_secs {
        lines.push(format!("Deletes {}", message_info_date_time(expires)));
    }
    let channels: Vec<String> = trace
        .transport_channels
        .iter()
        .map(|c| pretty_transport_channel(c))
        .collect();
    if !channels.is_empty() {
        lines.push(format!(
            "{} {}",
            if message.is_outgoing {
                "Sent over"
            } else {
                "Received over"
            },
            channels.join(", "),
        ));
    }
    if !message.recipient_deliveries.is_empty() {
        lines.push("Recipients".to_string());
        lines.extend(message.recipient_deliveries.iter().map(|recipient| {
            let name = chat
                .map(|c| recipient_info(recipient, c).name)
                .unwrap_or_else(|| fallback_person_name(&recipient.display_name));
            format!(
                "- {} {} {}",
                name,
                delivery_label(&recipient.delivery),
                message_info_date_time(recipient.updated_at_secs),
            )
        }));
    } else if !message.is_outgoing {
        let name = chat
            .map(|c| message_author_info(message, c).name)
            .unwrap_or_else(|| fallback_person_name(&message.author));
        lines.push(format!("From {name}"));
        lines.push(format!("You {}", delivery_label(&message.delivery)));
    }
    if !trace.outer_event_ids.is_empty() {
        lines.push(format!(
            "Network IDs {}",
            short_message_identifier_list(&trace.outer_event_ids)
        ));
    }
    if !trace.queued_protocol_targets.is_empty() {
        let npubs: Vec<String> = trace
            .queued_protocol_targets
            .iter()
            .map(|id| short_npub(id))
            .collect();
        lines.push(format!("Queued devices {}", npubs.join(", ")));
    }
    if !trace.target_device_ids.is_empty() {
        let npubs: Vec<String> = trace
            .target_device_ids
            .iter()
            .map(|id| short_npub(id))
            .collect();
        lines.push(format!("Devices {}", npubs.join(", ")));
    }
    if let Some(error) = trace
        .last_transport_error
        .as_deref()
        .filter(|error| !error.is_empty())
    {
        lines.push(format!("Last send error {}", error));
    }
    if let Some(source_event_id) = message
        .source_event_id
        .as_deref()
        .filter(|id| !id.is_empty())
    {
        lines.push(format!(
            "Received as {}",
            short_message_identifier(source_event_id)
        ));
    }
    if !message.attachments.is_empty() {
        lines.push("Attachments".to_string());
        lines.extend(message.attachments.iter().map(|attachment| {
            format!(
                "- {} {}",
                if attachment.filename.is_empty() {
                    "File"
                } else {
                    attachment.filename.as_str()
                },
                attachment.htree_url,
            )
        }));
    }
    if !message.reactions.is_empty() {
        lines.push("Reactions".to_string());
        lines.extend(
            message
                .reactions
                .iter()
                .map(|reaction| format!("- {} {}", reaction.emoji, reaction.count)),
        );
    }
    lines.join("\n")
}

fn synthesize_message_rumor_json(
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
    state: &iris_chat_core::AppState,
) -> String {
    let pubkey = if message.is_outgoing {
        state
            .account
            .as_ref()
            .map(|account| account.public_key_hex.clone())
            .unwrap_or_default()
    } else if matches!(chat.kind, ChatKind::Direct) {
        chat.chat_id.clone()
    } else {
        String::new()
    };

    let mut tags: Vec<serde_json::Value> = Vec::new();
    if let Some(expires) = message.expires_at_secs {
        tags.push(serde_json::json!(["expiration", expires.to_string()]));
    }
    for attachment in &message.attachments {
        tags.push(serde_json::json!([
            "imeta",
            format!("url {}", attachment.htree_url)
        ]));
    }

    let mut content = message.body.clone();
    if !message.attachments.is_empty() {
        let urls: Vec<String> = message
            .attachments
            .iter()
            .map(|attachment| attachment.htree_url.clone())
            .collect();
        let joined = urls.join("\n");
        content = if content.is_empty() {
            joined
        } else {
            format!("{}\n{}", content, joined)
        };
    }

    let rumor = serde_json::json!({
        "id": message.id,
        "pubkey": pubkey,
        "created_at": message.created_at_secs,
        "kind": 14,
        "tags": tags,
        "content": content,
    });
    serde_json::to_string_pretty(&rumor).unwrap_or_else(|_| String::from("{}"))
}

fn message_info_kind(message: &ChatMessageSnapshot) -> &'static str {
    match message.kind {
        ChatMessageKind::System => "System",
        _ => {
            if message.is_outgoing {
                "Sent"
            } else {
                "Received"
            }
        }
    }
}

fn participant_info(
    owner_pubkey_hex: Option<&str>,
    display_name: &str,
    picture_url: Option<&str>,
    chat: &CurrentChatSnapshot,
) -> ParticipantInfo {
    let owner = owner_pubkey_hex
        .map(str::trim)
        .filter(|owner| !owner.is_empty());
    if let Some(participant) = owner.and_then(|owner| {
        chat.participants
            .iter()
            .find(|p| p.owner_pubkey_hex == owner)
    }) {
        return ParticipantInfo {
            owner_pubkey_hex: Some(participant.owner_pubkey_hex.clone()),
            name: fallback_person_name(&participant.display_name),
            picture_url: participant.picture_url.clone(),
            is_me: participant.is_local_owner,
        };
    }
    ParticipantInfo {
        owner_pubkey_hex: owner.map(ToString::to_string),
        name: fallback_person_name(display_name),
        picture_url: picture_url.map(ToString::to_string),
        is_me: false,
    }
}

fn recipient_info(
    recipient: &MessageRecipientDeliverySnapshot,
    chat: &CurrentChatSnapshot,
) -> ParticipantInfo {
    participant_info(
        Some(&recipient.owner_pubkey_hex),
        &recipient.display_name,
        recipient.picture_url.as_deref(),
        chat,
    )
}

fn reactor_info(reactor: &MessageReactor, chat: &CurrentChatSnapshot) -> ParticipantInfo {
    participant_info(
        Some(&reactor.author),
        &reactor.display_name,
        reactor.picture_url.as_deref(),
        chat,
    )
}

fn direct_recipient_info(chat: &CurrentChatSnapshot) -> ParticipantInfo {
    participant_info(
        Some(&chat.chat_id),
        &chat.display_name,
        chat.picture_url.as_deref(),
        chat,
    )
}

fn message_author_info(
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
) -> ParticipantInfo {
    let owner = message
        .author_owner_pubkey_hex
        .as_deref()
        .filter(|owner| !owner.is_empty())
        .or_else(|| {
            (!message.is_outgoing && matches!(chat.kind, ChatKind::Direct))
                .then_some(chat.chat_id.as_str())
        });
    participant_info(
        owner,
        &message.author,
        message.author_picture_url.as_deref(),
        chat,
    )
}

fn fallback_person_name(display_name: &str) -> String {
    let trimmed = display_name.trim();
    if trimmed.is_empty() {
        "Iris user".to_string()
    } else {
        trimmed.to_string()
    }
}

fn short_npub(pubkey_input: &str) -> String {
    let npub = peer_input_to_npub(pubkey_input.to_string());
    let value = if npub.is_empty() {
        pubkey_input
    } else {
        npub.as_str()
    };
    short_message_identifier(value)
}

fn pretty_transport_channel(channel: &str) -> String {
    if let Some(rest) = channel.strip_prefix("message server: ") {
        return rest.to_string();
    }
    if channel == "message servers" {
        return "Message server".to_string();
    }
    channel.to_string()
}

fn message_info_date_time(secs: u64) -> String {
    let glib_dt = match gtk::glib::DateTime::from_unix_local(secs as i64) {
        Ok(v) => v,
        Err(_) => return relative_time(secs, unix_now()),
    };
    glib_dt
        .format("%b %-d, %Y · %H:%M")
        .map(|s| s.to_string())
        .unwrap_or_else(|_| relative_time(secs, unix_now()))
}

fn short_message_identifier_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| short_message_identifier(value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn short_message_identifier(value: &str) -> String {
    let char_count = value.chars().count();
    if char_count <= 16 {
        return value.to_string();
    }
    let start: String = value.chars().take(8).collect();
    let end: String = value.chars().skip(char_count - 8).collect();
    format!("{}...{}", start, end)
}

fn attachment_summary(attachments: &[&MessageAttachmentSnapshot]) -> String {
    if attachments.len() == 1 {
        let a = attachments[0];
        if a.is_video {
            return format!("🎞 {}", a.filename);
        }
        if a.is_audio {
            return format!("🔊 {}", a.filename);
        }
        return format!("📎 {}", a.filename);
    }
    format!("📎 {} attachments", attachments.len())
}

fn attachment_summary_widget(
    attachments: &[&MessageAttachmentSnapshot],
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let label = gtk::Label::new(Some(&attachment_summary(attachments)));
    label.add_css_class("bubble-meta");
    label.set_xalign(0.0);

    let text = attachments
        .iter()
        .map(|attachment| forwardable_attachment_text(attachment))
        .filter(|url| !url.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return label.upcast();
    }

    let popover = build_attachment_text_popover(text, manager);
    popover.set_parent(&label);

    let popover_for_click = popover.clone();
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    gesture.connect_pressed(move |_, _, x, y| {
        popover_for_click
            .set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover_for_click.popup();
    });
    label.add_controller(gesture);

    let popover_for_long = popover.clone();
    let long_press = gtk::GestureLongPress::new();
    long_press.connect_pressed(move |_, x, y| {
        popover_for_long.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover_for_long.popup();
    });
    label.add_controller(long_press);

    label.upcast()
}

fn forwardable_message_text(message: &ChatMessageSnapshot) -> String {
    let mut pieces = Vec::new();
    let body = reply_stripped_body(&message.body).trim().to_string();
    if !body.is_empty() {
        pieces.push(body);
    }
    pieces.extend(
        message
            .attachments
            .iter()
            .map(forwardable_attachment_text)
            .filter(|url| !url.is_empty()),
    );
    pieces.join("\n")
}

fn forwardable_attachment_text(attachment: &MessageAttachmentSnapshot) -> String {
    attachment.htree_url.trim().to_string()
}

fn reply_stripped_body(body: &str) -> &str {
    let Some(rest) = body.strip_prefix("↩ ") else {
        return body;
    };
    let Some(separator) = rest.find("\n\n") else {
        return body;
    };
    let header = &rest[..separator];
    if header.contains(':') {
        &rest[(separator + 2)..]
    } else {
        body
    }
}

fn present_forward_dialog(parent: Option<&gtk::Window>, text: &str, manager: &Rc<AppManager>) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }

    let chats = manager.current_state().chat_list;
    let dialog = adw::Dialog::builder()
        .title("Forward")
        .content_width(360)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    if chats.is_empty() {
        let empty = gtk::Label::new(Some("Start a chat first"));
        empty.add_css_class("dim-label");
        empty.set_vexpand(true);
        content.append(&empty);
        dialog.set_child(Some(&content));
        dialog.present(parent);
        return;
    }

    let selected = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let send = gtk::Button::with_label("Send");
    send.add_css_class("suggested-action");
    send.set_sensitive(false);

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_min_content_height(280);
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);

    for chat in chats {
        let row = gtk::CheckButton::with_label(&chat.display_name);
        row.set_halign(gtk::Align::Fill);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        row.set_margin_start(4);
        row.set_margin_end(4);
        let chat_id = chat.chat_id.clone();
        let selected_for_toggle = selected.clone();
        let send_for_toggle = send.clone();
        row.connect_toggled(move |check| {
            let mut selected = selected_for_toggle.borrow_mut();
            if check.is_active() {
                if !selected.iter().any(|id| id == &chat_id) {
                    selected.push(chat_id.clone());
                }
            } else {
                selected.retain(|id| id != &chat_id);
            }
            send_for_toggle.set_sensitive(!selected.is_empty());
        });
        list.append(&row);
    }
    scrolled.set_child(Some(&list));
    content.append(&scrolled);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    actions.append(&cancel);
    actions.append(&send);
    content.append(&actions);

    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog_for_cancel.close();
    });

    let dialog_for_send = dialog.clone();
    let selected_for_send = selected.clone();
    let manager_for_send = manager.clone();
    send.connect_clicked(move |_| {
        let targets = selected_for_send.borrow().clone();
        if targets.is_empty() {
            return;
        }
        for chat_id in &targets {
            manager_for_send.dispatch(AppAction::SendMessage {
                chat_id: chat_id.clone(),
                text: text.clone(),
            });
        }
        if let Some(first) = targets.first() {
            manager_for_send.dispatch(AppAction::OpenChat {
                chat_id: first.clone(),
            });
        }
        dialog_for_send.close();
    });

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn image_album(
    attachments: &[MessageAttachmentSnapshot],
    prefs: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    const ALBUM_WIDTH: i32 = 232;
    const GAP: i32 = 2;
    let container = gtk::Box::new(gtk::Orientation::Vertical, GAP);

    fn cell(
        attachment: &MessageAttachmentSnapshot,
        prefs: &PreferencesSnapshot,
        manager: &Rc<AppManager>,
        album: &[MessageAttachmentSnapshot],
        index: usize,
        width: i32,
        height: i32,
        overflow: Option<usize>,
    ) -> gtk::Widget {
        let widget = image_bubble_sized(attachment, prefs, manager, album, index, width, height);
        if let Some(extra) = overflow.filter(|n| *n > 0) {
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&widget));
            let shade = gtk::Box::new(gtk::Orientation::Vertical, 0);
            shade.set_hexpand(false);
            shade.set_vexpand(false);
            shade.set_halign(gtk::Align::Fill);
            shade.set_valign(gtk::Align::Fill);
            shade.add_css_class("iris-album-overflow");
            let css = gtk::CssProvider::new();
            css.load_from_string(
                "box.iris-album-overflow { background-color: rgba(0,0,0,0.45); border-radius: 4px; }
                 label.iris-album-overflow-label { color: #fff; font-weight: 700; font-size: 18pt; }",
            );
            if let Some(display) = shade.display().into() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &css,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
            let label = gtk::Label::new(Some(&format!("+{extra}")));
            label.add_css_class("iris-album-overflow-label");
            label.set_halign(gtk::Align::Center);
            label.set_valign(gtk::Align::Center);
            shade.append(&label);
            shade.set_can_target(false);
            overlay.add_overlay(&shade);
            overlay.upcast()
        } else {
            widget
        }
    }

    match attachments.len() {
        0 => {}
        1 => container.append(&cell(
            &attachments[0],
            prefs,
            manager,
            attachments,
            0,
            220,
            220,
            None,
        )),
        2 => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
            let cell_w = (ALBUM_WIDTH - GAP) / 2;
            row.append(&cell(
                &attachments[0],
                prefs,
                manager,
                attachments,
                0,
                cell_w,
                150,
                None,
            ));
            row.append(&cell(
                &attachments[1],
                prefs,
                manager,
                attachments,
                1,
                cell_w,
                150,
                None,
            ));
            container.append(&row);
        }
        3 => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
            let left_w = (ALBUM_WIDTH as f32 * 0.58) as i32 - GAP / 2;
            let right_w = (ALBUM_WIDTH as f32 * 0.42) as i32 - GAP / 2;
            let tall = (ALBUM_WIDTH as f32 * 0.86) as i32;
            let small = (tall - GAP) / 2;
            row.append(&cell(
                &attachments[0],
                prefs,
                manager,
                attachments,
                0,
                left_w,
                tall,
                None,
            ));
            let stack = gtk::Box::new(gtk::Orientation::Vertical, GAP);
            stack.append(&cell(
                &attachments[1],
                prefs,
                manager,
                attachments,
                1,
                right_w,
                small,
                None,
            ));
            stack.append(&cell(
                &attachments[2],
                prefs,
                manager,
                attachments,
                2,
                right_w,
                small,
                None,
            ));
            row.append(&stack);
            container.append(&row);
        }
        _ => {
            let cell_size = (ALBUM_WIDTH - GAP) / 2;
            let row1 = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
            row1.append(&cell(
                &attachments[0],
                prefs,
                manager,
                attachments,
                0,
                cell_size,
                cell_size,
                None,
            ));
            row1.append(&cell(
                &attachments[1],
                prefs,
                manager,
                attachments,
                1,
                cell_size,
                cell_size,
                None,
            ));
            let row2 = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
            row2.append(&cell(
                &attachments[2],
                prefs,
                manager,
                attachments,
                2,
                cell_size,
                cell_size,
                None,
            ));
            let overflow = if attachments.len() > 4 {
                Some(attachments.len() - 4)
            } else {
                None
            };
            row2.append(&cell(
                &attachments[3],
                prefs,
                manager,
                attachments,
                3,
                cell_size,
                cell_size,
                overflow,
            ));
            container.append(&row1);
            container.append(&row2);
        }
    }
    container.upcast()
}

fn image_bubble_sized(
    attachment: &MessageAttachmentSnapshot,
    prefs: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
    album: &[MessageAttachmentSnapshot],
    index: usize,
    width: i32,
    height: i32,
) -> gtk::Widget {
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_size_request(width, height);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.add_css_class("card");
    picture.set_cursor_from_name(Some("pointer"));

    let url = proxied_image_url(
        attachment.htree_url.clone(),
        prefs.clone(),
        Some(((width * 2).max(220)) as u32),
        Some(((height * 2).max(220)) as u32),
        false,
    );
    image_cache::fetch_into_picture(&picture, &url);

    let popover = build_attachment_popover(attachment, manager);
    popover.set_parent(&picture);
    let popover_for_click = popover.clone();
    let right_click = gtk::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, x, y| {
        popover_for_click
            .set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover_for_click.popup();
    });
    picture.add_controller(right_click);

    let popover_for_long = popover.clone();
    let long_press = gtk::GestureLongPress::new();
    long_press.connect_pressed(move |_, x, y| {
        popover_for_long.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover_for_long.popup();
    });
    picture.add_controller(long_press);

    let album_for_click = album.to_vec();
    let prefs_for_click = prefs.clone();
    let picture_for_click = picture.clone();
    let left_click = gtk::GestureClick::new();
    left_click.set_button(1);
    left_click.connect_released(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(window) = picture_for_click
            .root()
            .and_then(|r| r.downcast::<gtk::Window>().ok())
        {
            present_image_viewer(&window, &album_for_click, index, &prefs_for_click);
        }
    });
    picture.add_controller(left_click);

    picture.upcast()
}

fn present_image_viewer(
    parent: &gtk::Window,
    album: &[MessageAttachmentSnapshot],
    initial_index: usize,
    prefs: &PreferencesSnapshot,
) {
    if album.is_empty() {
        return;
    }
    let dialog = gtk::Window::new();
    dialog.set_transient_for(Some(parent));
    dialog.set_modal(true);
    dialog.set_decorated(false);
    dialog.set_default_size(900, 700);
    dialog.add_css_class("iris-image-viewer");

    let overlay = gtk::Overlay::new();
    let stage = gtk::Box::new(gtk::Orientation::Vertical, 0);
    stage.set_hexpand(true);
    stage.set_vexpand(true);
    stage.add_css_class("iris-image-viewer-stage");
    let backdrop_css = gtk::CssProvider::new();
    backdrop_css.load_from_string("box.iris-image-viewer-stage { background-color: #000; }");
    if let Some(display) = stage.display().into() {
        gtk::style_context_add_provider_for_display(
            &display,
            &backdrop_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    stage.append(&picture);
    overlay.set_child(Some(&stage));

    let close_btn = gtk::Button::from_icon_name("window-close-symbolic");
    close_btn.add_css_class("circular");
    close_btn.add_css_class("osd");
    close_btn.set_halign(gtk::Align::End);
    close_btn.set_valign(gtk::Align::Start);
    close_btn.set_margin_top(10);
    close_btn.set_margin_end(10);
    let dialog_for_close = dialog.clone();
    close_btn.connect_clicked(move |_| {
        dialog_for_close.close();
    });
    overlay.add_overlay(&close_btn);

    let current_index = Rc::new(RefCell::new(initial_index.min(album.len() - 1)));
    let album_for_state = album.to_vec();
    let prefs_for_state = prefs.clone();
    let picture_for_state = picture.clone();

    let load_current = Rc::new(move |idx: usize| {
        if let Some(attachment) = album_for_state.get(idx) {
            let url = proxied_image_url(
                attachment.htree_url.clone(),
                prefs_for_state.clone(),
                None,
                None,
                false,
            );
            image_cache::fetch_into_picture(&picture_for_state, &url);
        }
        if idx > 0 {
            if let Some(neighbor) = album_for_state.get(idx - 1) {
                let url = proxied_image_url(
                    neighbor.htree_url.clone(),
                    prefs_for_state.clone(),
                    None,
                    None,
                    false,
                );
                image_cache::prefetch(&url);
            }
        }
        if let Some(neighbor) = album_for_state.get(idx + 1) {
            let url = proxied_image_url(
                neighbor.htree_url.clone(),
                prefs_for_state.clone(),
                None,
                None,
                false,
            );
            image_cache::prefetch(&url);
        }
    });

    load_current(*current_index.borrow());

    let prev_btn = gtk::Button::from_icon_name("go-previous-symbolic");
    let next_btn = gtk::Button::from_icon_name("go-next-symbolic");
    for btn in [&prev_btn, &next_btn] {
        btn.add_css_class("circular");
        btn.add_css_class("osd");
        btn.set_valign(gtk::Align::Center);
    }
    prev_btn.set_halign(gtk::Align::Start);
    prev_btn.set_margin_start(12);
    next_btn.set_halign(gtk::Align::End);
    next_btn.set_margin_end(12);

    let update_nav = {
        let prev_btn = prev_btn.clone();
        let next_btn = next_btn.clone();
        let album_len = album.len();
        Rc::new(move |idx: usize| {
            prev_btn.set_sensitive(idx > 0);
            next_btn.set_sensitive(idx + 1 < album_len);
            let multi = album_len > 1;
            prev_btn.set_visible(multi);
            next_btn.set_visible(multi);
        })
    };
    update_nav(*current_index.borrow());

    {
        let current_index = current_index.clone();
        let load_current = load_current.clone();
        let update_nav = update_nav.clone();
        prev_btn.connect_clicked(move |_| {
            let mut idx = current_index.borrow_mut();
            if *idx > 0 {
                *idx -= 1;
                load_current(*idx);
                update_nav(*idx);
            }
        });
    }
    {
        let current_index = current_index.clone();
        let load_current = load_current.clone();
        let update_nav = update_nav.clone();
        let album_len = album.len();
        next_btn.connect_clicked(move |_| {
            let mut idx = current_index.borrow_mut();
            if *idx + 1 < album_len {
                *idx += 1;
                load_current(*idx);
                update_nav(*idx);
            }
        });
    }
    overlay.add_overlay(&prev_btn);
    overlay.add_overlay(&next_btn);

    let dialog_for_keys = dialog.clone();
    let key_controller = gtk::EventControllerKey::new();
    let current_index_for_keys = current_index.clone();
    let load_current_for_keys = load_current.clone();
    let update_nav_for_keys = update_nav.clone();
    let album_len_for_keys = album.len();
    key_controller.connect_key_pressed(move |_, keyval, _, _| match keyval {
        gtk::gdk::Key::Escape => {
            dialog_for_keys.close();
            glib::Propagation::Stop
        }
        gtk::gdk::Key::Left => {
            let mut idx = current_index_for_keys.borrow_mut();
            if *idx > 0 {
                *idx -= 1;
                load_current_for_keys(*idx);
                update_nav_for_keys(*idx);
            }
            glib::Propagation::Stop
        }
        gtk::gdk::Key::Right => {
            let mut idx = current_index_for_keys.borrow_mut();
            if *idx + 1 < album_len_for_keys {
                *idx += 1;
                load_current_for_keys(*idx);
                update_nav_for_keys(*idx);
            }
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    dialog.add_controller(key_controller);

    let dialog_for_backdrop = dialog.clone();
    let backdrop_click = gtk::GestureClick::new();
    backdrop_click.set_button(1);
    backdrop_click.connect_released(move |_, _, _, _| {
        dialog_for_backdrop.close();
    });
    stage.add_controller(backdrop_click);

    dialog.set_child(Some(&overlay));
    dialog.present();
}

fn build_attachment_popover(
    attachment: &MessageAttachmentSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Popover {
    build_attachment_text_popover(forwardable_attachment_text(attachment), manager)
}

fn build_attachment_text_popover(text: String, manager: &Rc<AppManager>) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Top);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_margin_top(6);
    column.set_margin_bottom(6);
    column.set_margin_start(6);
    column.set_margin_end(6);

    let forward = gtk::Button::with_label("Forward");
    forward.add_css_class("flat");
    forward.set_halign(gtk::Align::Fill);
    let manager_for_forward = manager.clone();
    let text_for_forward = text.clone();
    let popover_for_forward = popover.clone();
    forward.connect_clicked(move |btn| {
        let parent = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        present_forward_dialog(parent.as_ref(), &text_for_forward, &manager_for_forward);
        popover_for_forward.popdown();
    });
    column.append(&forward);

    let copy = gtk::Button::with_label("Copy link");
    copy.add_css_class("flat");
    copy.set_halign(gtk::Align::Fill);
    let text_for_copy = text;
    let popover_for_copy = popover.clone();
    copy.connect_clicked(move |_| {
        crate::platform::clipboard::copy(&text_for_copy);
        popover_for_copy.popdown();
    });
    column.append(&copy);

    popover.set_child(Some(&column));
    popover
}

fn composer(chat: &CurrentChatSnapshot, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);
    outer.set_margin_top(8);
    outer.set_margin_bottom(8);
    outer.set_margin_start(12);
    outer.set_margin_end(12);

    let preview_scroll = gtk::ScrolledWindow::new();
    preview_scroll.set_hscrollbar_policy(gtk::PolicyType::Automatic);
    preview_scroll.set_vscrollbar_policy(gtk::PolicyType::Never);
    preview_scroll.set_propagate_natural_height(true);
    let preview_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    preview_scroll.set_child(Some(&preview_row));
    outer.append(&preview_scroll);
    rebuild_attachment_previews(&preview_row, manager, &chat.chat_id);
    preview_scroll.set_visible(preview_row.first_child().is_some());

    if state.busy.uploading_attachment {
        let progress = gtk::ProgressBar::new();
        progress.set_show_text(false);
        progress.add_css_class("osd");
        if let Some(upload) = state.busy.upload_progress.as_ref() {
            if upload.total_bytes > 0 {
                let fraction =
                    (upload.bytes_uploaded as f64 / upload.total_bytes as f64).clamp(0.0, 1.0);
                progress.set_fraction(fraction);
            } else {
                progress.pulse();
            }
        } else {
            progress.pulse();
        }
        outer.append(&progress);
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);

    let attach = gtk::Button::from_icon_name("mail-attachment-symbolic");
    attach.add_css_class("flat");
    attach.add_css_class("circular");
    attach.set_tooltip_text(Some("Attach file"));
    attach.set_sensitive(!state.busy.uploading_attachment);
    let manager_for_attach = manager.clone();
    let chat_id_for_attach = chat.chat_id.clone();
    let preview_row_for_attach = preview_row.clone();
    let preview_scroll_for_attach = preview_scroll.clone();
    attach.connect_clicked(move |btn| {
        let parent = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        let dialog = gtk::FileDialog::builder().title("Attach file").build();
        let manager = manager_for_attach.clone();
        let chat_id = chat_id_for_attach.clone();
        let preview_row = preview_row_for_attach.clone();
        let preview_scroll = preview_scroll_for_attach.clone();
        dialog.open(
            parent.as_ref(),
            gtk::gio::Cancellable::NONE,
            move |result| {
                let Ok(file) = result else { return };
                let path = match file.path() {
                    Some(p) => p.to_string_lossy().to_string(),
                    None => return,
                };
                let filename = file
                    .basename()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "attachment".to_string());
                manager.stage_attachment(
                    &chat_id,
                    OutgoingAttachment {
                        file_path: path,
                        filename,
                    },
                );
                rebuild_attachment_previews(&preview_row, &manager, &chat_id);
                preview_scroll.set_visible(preview_row.first_child().is_some());
            },
        );
    });
    row.append(&attach);

    let buffer = gtk::TextBuffer::new(None);
    let input = gtk::TextView::with_buffer(&buffer);
    input.add_css_class("composer-input");
    input.set_accepts_tab(false);
    input.set_hexpand(true);
    input.set_wrap_mode(gtk::WrapMode::WordChar);
    input.set_top_margin(9);
    input.set_bottom_margin(9);
    input.set_left_margin(12);
    input.set_right_margin(12);

    let input_scroll = gtk::ScrolledWindow::new();
    input_scroll.set_hexpand(true);
    input_scroll.set_min_content_height(40);
    input_scroll.set_max_content_height(132);
    input_scroll.set_propagate_natural_height(true);
    input_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    input_scroll.set_vscrollbar_policy(gtk::PolicyType::Automatic);
    input_scroll.set_child(Some(&input));

    let input_overlay = gtk::Overlay::new();
    input_overlay.set_hexpand(true);
    input_overlay.set_child(Some(&input_scroll));

    let placeholder = gtk::Label::new(Some("Message"));
    placeholder.add_css_class("dim-label");
    placeholder.set_halign(gtk::Align::Start);
    placeholder.set_valign(gtk::Align::Start);
    placeholder.set_margin_start(13);
    placeholder.set_margin_top(10);
    placeholder.set_can_target(false);
    input_overlay.add_overlay(&placeholder);

    // Seed the composer with the persisted draft so unsent text
    // survives navigation + relaunch. The core dedups identical draft
    // writes, so doing this before wiring `connect_changed` is fine.
    if !chat.draft.is_empty() {
        buffer.set_text(&chat.draft);
    }
    placeholder.set_visible(chat.draft.is_empty());
    row.append(&input_overlay);

    let emoji_btn = gtk::Button::from_icon_name("face-smile-symbolic");
    emoji_btn.add_css_class("flat");
    emoji_btn.add_css_class("circular");
    emoji_btn.set_tooltip_text(Some("Insert emoji"));
    let emoji_chooser = gtk::EmojiChooser::new();
    emoji_chooser.set_parent(&emoji_btn);
    {
        let buffer_for_emoji = buffer.clone();
        let input_for_emoji = input.clone();
        emoji_chooser.connect_emoji_picked(move |_, emoji_text| {
            buffer_for_emoji.insert_at_cursor(emoji_text);
            input_for_emoji.grab_focus();
        });
    }
    {
        let chooser_for_click = emoji_chooser.clone();
        emoji_btn.connect_clicked(move |_| chooser_for_click.popup());
    }
    row.append(&emoji_btn);

    if manager.should_focus_composer(&chat.chat_id) {
        let input_for_focus = input.clone();
        gtk::glib::idle_add_local_once(move || {
            input_for_focus.grab_focus();
        });
    }

    {
        let manager_for_typing = manager.clone();
        let chat_id_for_typing = chat.chat_id.clone();
        let placeholder_for_typing = placeholder.clone();
        buffer.connect_changed(move |buffer| {
            let text = composer_buffer_text(buffer);
            placeholder_for_typing.set_visible(text.is_empty());
            if text.is_empty() {
                manager_for_typing.dispatch(AppAction::StopTyping {
                    chat_id: chat_id_for_typing.clone(),
                });
            } else {
                manager_for_typing.dispatch(AppAction::SendTyping {
                    chat_id: chat_id_for_typing.clone(),
                });
            }
            // Persist the unsent draft on every change. The core
            // checks against the previous value and short-circuits
            // identical writes, so the per-keystroke dispatch is
            // cheap and matches Signal-iOS's "save on every edit"
            // contract without any debounce timer on this side.
            manager_for_typing.dispatch(AppAction::SetChatDraft {
                chat_id: chat_id_for_typing.clone(),
                text,
            });
        });
    }

    let busy = state.busy.sending_message;
    let send = gtk::Button::from_icon_name("document-send-symbolic");
    send.add_css_class("suggested-action");
    send.add_css_class("circular");
    send.set_tooltip_text(Some("Send"));
    send.set_sensitive(!busy);
    row.append(&send);

    let chat_id = chat.chat_id.clone();
    let ttl = chat.message_ttl_seconds;
    let manager_for_click = manager.clone();
    let buffer_for_click = buffer.clone();
    let preview_row_for_send = preview_row.clone();
    let preview_scroll_for_send = preview_scroll.clone();
    send.connect_clicked(move |btn| {
        if submit_composer(
            &manager_for_click,
            &chat_id,
            &buffer_for_click,
            ttl,
            &preview_row_for_send,
            &preview_scroll_for_send,
        ) {
            btn.set_sensitive(false);
        }
    });

    let chat_id = chat.chat_id.clone();
    let ttl = chat.message_ttl_seconds;
    let manager_for_enter = manager.clone();
    let buffer_for_enter = buffer.clone();
    let preview_row_for_enter = preview_row.clone();
    let preview_scroll_for_enter = preview_scroll.clone();
    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    key_controller.connect_key_pressed(move |_, keyval, _, state| {
        if !matches!(keyval, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
            || state.contains(gtk::gdk::ModifierType::SHIFT_MASK)
        {
            return glib::Propagation::Proceed;
        }

        submit_composer(
            &manager_for_enter,
            &chat_id,
            &buffer_for_enter,
            ttl,
            &preview_row_for_enter,
            &preview_scroll_for_enter,
        );
        glib::Propagation::Stop
    });
    input.add_controller(key_controller);

    outer.append(&row);
    outer.upcast()
}

fn composer_buffer_text(buffer: &gtk::TextBuffer) -> String {
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, true).to_string()
}

fn submit_composer(
    manager: &Rc<AppManager>,
    chat_id: &str,
    buffer: &gtk::TextBuffer,
    ttl_seconds: Option<u64>,
    preview_row: &gtk::Box,
    preview_scroll: &gtk::ScrolledWindow,
) -> bool {
    let text = composer_buffer_text(buffer).trim().to_string();
    let staged = manager.staged_attachments(chat_id);
    if text.is_empty() && staged.is_empty() {
        return false;
    }

    buffer.set_text("");
    dispatch_send(manager, chat_id, text, ttl_seconds);
    rebuild_attachment_previews(preview_row, manager, chat_id);
    preview_scroll.set_visible(preview_row.first_child().is_some());
    true
}

fn rebuild_attachment_previews(row: &gtk::Box, manager: &Rc<AppManager>, chat_id: &str) {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    for attachment in manager.staged_attachments(chat_id) {
        row.append(&attachment_chip(manager, chat_id, &attachment, row));
    }
}

fn attachment_chip(
    manager: &Rc<AppManager>,
    chat_id: &str,
    attachment: &OutgoingAttachment,
    row: &gtk::Box,
) -> gtk::Widget {
    if is_image_filename(&attachment.filename) {
        if let Some(widget) = image_attachment_chip(manager, chat_id, attachment, row) {
            return widget;
        }
    }

    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    chip.add_css_class("card");
    chip.set_margin_top(2);
    chip.set_margin_bottom(2);

    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    inner.set_margin_top(6);
    inner.set_margin_bottom(6);
    inner.set_margin_start(10);
    inner.set_margin_end(4);

    let icon = gtk::Image::from_icon_name(attachment_icon_name(&attachment.filename));
    icon.set_pixel_size(20);
    inner.append(&icon);

    let label = gtk::Label::new(Some(&attachment.filename));
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(24);
    label.set_xalign(0.0);
    inner.append(&label);

    let remove = gtk::Button::from_icon_name("window-close-symbolic");
    remove.add_css_class("flat");
    remove.add_css_class("circular");
    remove.set_tooltip_text(Some("Remove attachment"));
    let manager_for_remove = manager.clone();
    let chat_id_for_remove = chat_id.to_string();
    let file_path = attachment.file_path.clone();
    let row_for_remove = row.clone();
    remove.connect_clicked(move |_| {
        manager_for_remove.unstage_attachment(&chat_id_for_remove, &file_path);
        rebuild_attachment_previews(&row_for_remove, &manager_for_remove, &chat_id_for_remove);
        if let Some(scroll) = row_for_remove
            .parent()
            .and_then(|p| p.downcast::<gtk::ScrolledWindow>().ok())
        {
            scroll.set_visible(row_for_remove.first_child().is_some());
        }
    });
    inner.append(&remove);

    chip.append(&inner);
    chip.upcast()
}

fn image_attachment_chip(
    manager: &Rc<AppManager>,
    chat_id: &str,
    attachment: &OutgoingAttachment,
    row: &gtk::Box,
) -> Option<gtk::Widget> {
    let texture = gtk::gdk::Texture::from_filename(&attachment.file_path).ok()?;

    let overlay = gtk::Overlay::new();
    overlay.set_margin_top(2);
    overlay.set_margin_bottom(2);
    overlay.set_tooltip_text(Some(&attachment.filename));

    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_can_shrink(true);
    picture.set_size_request(56, 56);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.add_css_class("card");
    overlay.set_child(Some(&picture));

    let remove = gtk::Button::from_icon_name("window-close-symbolic");
    remove.add_css_class("circular");
    remove.add_css_class("osd");
    remove.set_tooltip_text(Some("Remove attachment"));
    remove.set_halign(gtk::Align::End);
    remove.set_valign(gtk::Align::Start);
    remove.set_margin_top(2);
    remove.set_margin_end(2);
    let manager_for_remove = manager.clone();
    let chat_id_for_remove = chat_id.to_string();
    let file_path = attachment.file_path.clone();
    let row_for_remove = row.clone();
    remove.connect_clicked(move |_| {
        manager_for_remove.unstage_attachment(&chat_id_for_remove, &file_path);
        rebuild_attachment_previews(&row_for_remove, &manager_for_remove, &chat_id_for_remove);
        if let Some(scroll) = row_for_remove
            .parent()
            .and_then(|p| p.downcast::<gtk::ScrolledWindow>().ok())
        {
            scroll.set_visible(row_for_remove.first_child().is_some());
        }
    });
    overlay.add_overlay(&remove);

    Some(overlay.upcast())
}

fn is_image_filename(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    matches!(
        ext,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "heif" | "avif" | "tif" | "tiff"
    )
}

fn attachment_icon_name(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "heif" => "image-x-generic",
        "mp4" | "mov" | "mkv" | "webm" | "avi" => "video-x-generic",
        "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" => "audio-x-generic",
        "zip" | "tar" | "gz" | "rar" | "7z" => "package-x-generic",
        "pdf" | "doc" | "docx" | "txt" | "md" => "text-x-generic",
        _ => "mail-attachment-symbolic",
    }
}

fn dispatch_send(manager: &Rc<AppManager>, chat_id: &str, text: String, ttl_seconds: Option<u64>) {
    let staged = manager.take_staged_attachments(chat_id);
    if !staged.is_empty() {
        manager.dispatch(AppAction::SendAttachments {
            chat_id: chat_id.to_string(),
            attachments: staged,
            caption: text,
        });
        return;
    }
    manager.dispatch(send_action(chat_id, text, ttl_seconds));
}

fn send_action(chat_id: &str, text: String, ttl_seconds: Option<u64>) -> AppAction {
    if let Some(ttl) = ttl_seconds.filter(|t| *t > 0) {
        let now = unix_now();
        AppAction::SendDisappearingMessage {
            chat_id: chat_id.to_string(),
            text,
            expires_at_secs: now + ttl,
        }
    } else {
        AppAction::SendMessage {
            chat_id: chat_id.to_string(),
            text,
        }
    }
}
