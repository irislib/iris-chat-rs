use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use iris_chat_core::{
    proxied_image_url, AccountSnapshot, AppAction, AppState, AppUpdate, ChatThreadSnapshot,
    CurrentChatSnapshot, Screen,
};

use crate::app_manager::AppManager;
use crate::platform::notifications;
use crate::screens;
use crate::widgets::image_cache;

const APP_ID: &str = "to.iris.chat";

pub fn build_ui(app: &adw::Application, present_on_create: bool) {
    if let Some(window) = app
        .active_window()
        .or_else(|| app.windows().into_iter().next())
    {
        window.present();
        return;
    }

    let manager = Rc::new(AppManager::new());

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(420)
        .default_height(740)
        .title("Iris Chat")
        .build();

    let header = adw::HeaderBar::new();
    let title_label = gtk::Label::new(None);
    title_label.add_css_class("heading");
    title_label.set_xalign(0.0);
    title_label.set_halign(gtk::Align::Start);
    let title_status = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    title_status.set_halign(gtk::Align::Start);
    title_status.set_visible(false);
    let title_status_icon = gtk::Image::from_icon_name("notifications-disabled-symbolic");
    title_status_icon.add_css_class("dim-label");
    title_status.append(&title_status_icon);
    let title_status_label = gtk::Label::new(Some("muted"));
    title_status_label.add_css_class("caption");
    title_status_label.add_css_class("dim-label");
    title_status.append(&title_status_label);
    let title_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    title_column.set_valign(gtk::Align::Center);
    title_column.set_halign(gtk::Align::Start);
    title_column.append(&title_label);
    title_column.append(&title_status);
    let title_slot = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    title_slot.set_valign(gtk::Align::Center);
    title_slot.set_halign(gtk::Align::Start);
    title_slot.append(&title_column);
    // Use an empty title so the header bar doesn't reserve centered space
    // for it; we pack the avatar+name on the left edge instead.
    let empty_title = gtk::Label::new(None);
    header.set_title_widget(Some(&empty_title));
    header.pack_start(&title_slot);

    let back_button = gtk::Button::from_icon_name("go-previous-symbolic");
    back_button.set_tooltip_text(Some("Back"));
    back_button.set_visible(false);
    {
        let manager = manager.clone();
        back_button.connect_clicked(move |_| {
            let mut stack = manager.current_state().router.screen_stack;
            stack.pop();
            manager.dispatch(AppAction::UpdateScreenStack { stack });
        });
    }
    header.pack_start(&back_button);

    let new_chat_button = gtk::Button::from_icon_name("list-add-symbolic");
    new_chat_button.set_tooltip_text(Some("New chat"));
    new_chat_button.set_visible(false);
    {
        let manager = manager.clone();
        new_chat_button.connect_clicked(move |_| {
            manager.dispatch(AppAction::PushScreen {
                screen: Screen::NewChat,
            });
        });
    }
    header.pack_end(&new_chat_button);

    let settings_button = gtk::Button::new();
    settings_button.add_css_class("flat");
    settings_button.add_css_class("circular");
    settings_button.set_tooltip_text(Some("Settings"));
    settings_button.set_visible(false);
    {
        let manager = manager.clone();
        settings_button.connect_clicked(move |_| {
            manager.dispatch(AppAction::PushScreen {
                screen: Screen::Settings,
            });
        });
    }
    header.pack_start(&settings_button);

    let chat_info_button = gtk::Button::from_icon_name("dialog-information-symbolic");
    chat_info_button.set_tooltip_text(Some("Chat info"));
    chat_info_button.set_visible(false);
    {
        let manager = manager.clone();
        chat_info_button.connect_clicked(move |btn| {
            let state = manager.current_state();
            let Some(chat) = state.current_chat.as_ref() else {
                return;
            };
            if let Some(group_id) = chat.group_id.as_ref() {
                manager.dispatch(AppAction::PushScreen {
                    screen: Screen::GroupDetails {
                        group_id: group_id.clone(),
                    },
                });
            } else {
                let parent = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
                crate::screens::chat::present_chat_info(
                    parent.as_ref(),
                    crate::screens::chat::ChatInfoSnapshot {
                        chat_id: chat.chat_id.clone(),
                        display_name: chat.display_name.clone(),
                        subtitle: chat.subtitle.clone(),
                        picture_url: chat.picture_url.clone(),
                        is_muted: chat.is_muted,
                        preferences: state.preferences.clone(),
                    },
                    manager.clone(),
                );
            }
        });
    }
    header.pack_end(&chat_info_button);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);

    let content_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content_slot.set_vexpand(true);

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&content_slot));
    toolbar.set_content(Some(&toast_overlay));

    window.set_content(Some(&toolbar));

    let current = Rc::new(RefCell::new(manager.current_state()));
    let last_toast: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let header_widgets = HeaderWidgets {
        back: back_button.clone(),
        new_chat: new_chat_button.clone(),
        settings: settings_button.clone(),
        chat_info: chat_info_button.clone(),
        title: title_label.clone(),
        title_column: title_column.clone(),
        title_status: title_status.clone(),
        title_slot: title_slot.clone(),
    };
    apply_state(&content_slot, &header_widgets, &manager, &current.borrow());
    show_toast_if_changed(&toast_overlay, &last_toast, &current.borrow().toast);

    let update_rx = manager.update_rx();
    let content_for_updates = content_slot.clone();
    let header_for_updates = header_widgets.clone();
    let toast_for_updates = toast_overlay.clone();
    let last_toast_for_updates = last_toast.clone();
    let manager_for_updates = manager.clone();
    let current_for_updates = current.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(update) = update_rx.recv().await {
            match update {
                AppUpdate::FullState(state) => {
                    let mut slot = current_for_updates.borrow_mut();
                    if state.rev >= slot.rev {
                        let prev_chat_list = slot.chat_list.clone();
                        let prev_focused_chat_id =
                            slot.current_chat.as_ref().map(|c| c.chat_id.clone());
                        *slot = state;
                        manager_for_updates.sync_nearby_preference(&slot);
                        apply_state(
                            &content_for_updates,
                            &header_for_updates,
                            &manager_for_updates,
                            &slot,
                        );
                        show_toast_if_changed(
                            &toast_for_updates,
                            &last_toast_for_updates,
                            &slot.toast,
                        );
                        if slot.preferences.desktop_notifications_enabled {
                            notify_new_messages(
                                &prev_chat_list,
                                &slot.chat_list,
                                prev_focused_chat_id.as_deref(),
                            );
                        }
                    }
                }
                AppUpdate::NearbyPublishedEvent {
                    event_id,
                    kind,
                    created_at_secs,
                    event_json,
                } => {
                    manager_for_updates.publish_nearby_event(
                        event_id,
                        kind,
                        created_at_secs,
                        event_json,
                    );
                }
                AppUpdate::PersistAccountBundle { .. } => {}
            }
        }
    });

    let nearby_rx = manager.nearby_update_rx();
    let content_for_nearby = content_slot.clone();
    let header_for_nearby = header_widgets.clone();
    let manager_for_nearby = manager.clone();
    let current_for_nearby = current.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(snapshot) = nearby_rx.recv().await {
            manager_for_nearby.apply_nearby_snapshot(snapshot);
            let slot = current_for_nearby.borrow();
            apply_state(
                &content_for_nearby,
                &header_for_nearby,
                &manager_for_nearby,
                &slot,
            );
        }
    });

    let manager_for_focus = manager.clone();
    window.connect_is_active_notify(move |w| {
        if w.is_active() {
            manager_for_focus.dispatch(AppAction::AppForegrounded);
        }
    });

    if present_on_create {
        window.present();
    }
}

fn show_toast_if_changed(
    overlay: &adw::ToastOverlay,
    last_toast: &Rc<RefCell<Option<String>>>,
    current: &Option<String>,
) {
    let same = match (last_toast.borrow().as_ref(), current.as_ref()) {
        (Some(a), Some(b)) => a == b,
        (None, None) => true,
        _ => false,
    };
    if same {
        return;
    }
    *last_toast.borrow_mut() = current.clone();
    if let Some(text) = current {
        if !text.is_empty() {
            let toast = adw::Toast::new(text);
            toast.set_timeout(3);
            overlay.add_toast(toast);
        }
    }
}

#[derive(Clone)]
struct HeaderWidgets {
    back: gtk::Button,
    new_chat: gtk::Button,
    settings: gtk::Button,
    chat_info: gtk::Button,
    title: gtk::Label,
    title_column: gtk::Box,
    title_status: gtk::Box,
    title_slot: gtk::Box,
}

fn apply_state(
    slot: &gtk::Box,
    header: &HeaderWidgets,
    manager: &Rc<AppManager>,
    state: &AppState,
) {
    while let Some(child) = slot.first_child() {
        slot.remove(&child);
    }

    let screen = current_screen(state);
    header
        .back
        .set_visible(!state.router.screen_stack.is_empty());
    header
        .new_chat
        .set_visible(matches!(screen, Screen::ChatList));
    let show_settings = matches!(screen, Screen::ChatList) && state.account.is_some();
    header.settings.set_visible(show_settings);
    if show_settings {
        if let Some(account) = state.account.as_ref() {
            header
                .settings
                .set_child(Some(&build_own_avatar(account, state)));
        }
    } else {
        header.settings.set_child(gtk::Widget::NONE);
    }
    header
        .chat_info
        .set_visible(matches!(screen, Screen::Chat { .. }));

    let title_text =
        chat_title(&screen, state).unwrap_or_else(|| screens::title(&screen).to_string());
    header.title.set_label(&title_text);
    header.title_status.set_visible(
        matches!(screen, Screen::Chat { .. })
            && state
                .current_chat
                .as_ref()
                .map(|chat| chat.is_muted)
                .unwrap_or(false),
    );

    // Tear down any avatar from a previous render.
    while let Some(child) = header.title_slot.first_child() {
        if child == header.title_column.clone().upcast::<gtk::Widget>() {
            // Keep the heading column in place.
            break;
        }
        header.title_slot.remove(&child);
    }
    if matches!(screen, Screen::Chat { .. }) {
        if let Some(chat) = state.current_chat.as_ref() {
            let avatar = build_chat_header_avatar(chat, state);
            header.title_slot.prepend(&avatar);
            attach_chat_title_click(&header.title_slot, manager, chat);
        }
    } else {
        for ctrl in header
            .title_slot
            .observe_controllers()
            .into_iter()
            .flatten()
        {
            if let Ok(ev) = ctrl.downcast::<gtk::EventController>() {
                if ev.is::<gtk::GestureClick>() {
                    header.title_slot.remove_controller(&ev);
                }
            }
        }
    }

    let widget = screens::render(&screen, state, manager);
    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(560)
        .build();
    clamp.set_child(Some(&widget));
    clamp.set_vexpand(true);
    slot.append(&clamp);
}

fn build_own_avatar(account: &AccountSnapshot, state: &AppState) -> gtk::Widget {
    let label = if account.display_name.is_empty() {
        "Iris user"
    } else {
        account.display_name.as_str()
    };
    let avatar = adw::Avatar::new(28, Some(label), true);
    if let Some(url) = account.picture_url.as_deref() {
        if url.starts_with("http://") || url.starts_with("https://") {
            let proxied = proxied_image_url(
                url.to_string(),
                state.preferences.clone(),
                Some(56),
                Some(56),
                true,
            );
            image_cache::fetch_into_avatar(&avatar, &proxied);
        }
    }
    avatar.upcast()
}

fn build_chat_header_avatar(chat: &CurrentChatSnapshot, state: &AppState) -> gtk::Widget {
    let avatar = adw::Avatar::new(28, Some(&chat.display_name), true);
    if let Some(url) = chat.picture_url.as_deref() {
        if url.starts_with("http://") || url.starts_with("https://") {
            let proxied = proxied_image_url(
                url.to_string(),
                state.preferences.clone(),
                Some(56),
                Some(56),
                true,
            );
            image_cache::fetch_into_avatar(&avatar, &proxied);
        }
    }
    avatar.upcast()
}

fn attach_chat_title_click(slot: &gtk::Box, manager: &Rc<AppManager>, chat: &CurrentChatSnapshot) {
    for ctrl in slot.observe_controllers().into_iter().flatten() {
        if let Ok(ev) = ctrl.downcast::<gtk::EventController>() {
            if ev.is::<gtk::GestureClick>() {
                slot.remove_controller(&ev);
            }
        }
    }
    let gesture = gtk::GestureClick::new();
    let manager = manager.clone();
    let chat_id = chat.chat_id.clone();
    let group_id = chat.group_id.clone();
    gesture.connect_released(move |gesture, _, _, _| {
        let widget = gesture
            .widget()
            .and_then(|w| w.root())
            .and_then(|r| r.downcast::<gtk::Window>().ok());
        if let Some(group_id) = group_id.clone() {
            manager.dispatch(AppAction::PushScreen {
                screen: Screen::GroupDetails { group_id },
            });
            return;
        }
        let state = manager.current_state();
        let Some(chat) = state.current_chat.as_ref().filter(|c| c.chat_id == chat_id) else {
            return;
        };
        crate::screens::chat::present_chat_info(
            widget.as_ref(),
            crate::screens::chat::ChatInfoSnapshot {
                chat_id: chat.chat_id.clone(),
                display_name: chat.display_name.clone(),
                subtitle: chat.subtitle.clone(),
                picture_url: chat.picture_url.clone(),
                is_muted: chat.is_muted,
                preferences: state.preferences.clone(),
            },
            manager.clone(),
        );
    });
    slot.add_controller(gesture);
}

fn notify_new_messages(
    prev: &[ChatThreadSnapshot],
    current: &[ChatThreadSnapshot],
    focused_chat_id: Option<&str>,
) {
    let prev_map: HashMap<&str, &ChatThreadSnapshot> =
        prev.iter().map(|c| (c.chat_id.as_str(), c)).collect();
    for chat in current {
        if chat.is_muted {
            continue;
        }
        if Some(chat.chat_id.as_str()) == focused_chat_id {
            continue;
        }
        let last_at = chat.last_message_at_secs.unwrap_or(0);
        let prev_at = prev_map
            .get(chat.chat_id.as_str())
            .and_then(|p| p.last_message_at_secs)
            .unwrap_or(0);
        if last_at <= prev_at {
            continue;
        }
        if !matches!(chat.last_message_is_outgoing, Some(false)) {
            continue;
        }
        let body = chat
            .last_message_preview
            .clone()
            .unwrap_or_else(|| "New message".to_string());
        notifications::notify(APP_ID, &chat.display_name, &body);
    }
}

fn chat_title(screen: &Screen, state: &AppState) -> Option<String> {
    if matches!(screen, Screen::Chat { .. }) {
        if let Some(chat) = state.current_chat.as_ref() {
            return Some(chat.display_name.clone());
        }
    }
    if let Screen::GroupDetails { .. } = screen {
        if let Some(details) = state.group_details.as_ref() {
            return Some(details.name.clone());
        }
    }
    None
}

fn current_screen(state: &AppState) -> Screen {
    state
        .router
        .screen_stack
        .last()
        .cloned()
        .unwrap_or_else(|| state.router.default_screen.clone())
}
