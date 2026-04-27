use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use ndr_demo_core::{AppAction, AppState, AppUpdate, ChatThreadSnapshot, Screen};

use crate::app_manager::AppManager;
use crate::platform::notifications;
use crate::screens;

const APP_ID: &str = "to.iris.chat";

pub fn build_ui(app: &adw::Application) {
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
    header.set_title_widget(Some(&title_label));

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

    let settings_button = gtk::Button::from_icon_name("preferences-system-symbolic");
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
                let parent = btn
                    .root()
                    .and_then(|r| r.downcast::<gtk::Window>().ok());
                crate::screens::chat::present_chat_info(
                    parent.as_ref(),
                    &chat.display_name,
                    &chat.chat_id,
                    chat.subtitle.as_deref(),
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
            if let AppUpdate::FullState(state) = update {
                let mut slot = current_for_updates.borrow_mut();
                if state.rev >= slot.rev {
                    let prev_chat_list = slot.chat_list.clone();
                    let prev_focused_chat_id = slot
                        .current_chat
                        .as_ref()
                        .map(|c| c.chat_id.clone());
                    *slot = state;
                    apply_state(
                        &content_for_updates,
                        &header_for_updates,
                        &manager_for_updates,
                        &slot,
                    );
                    show_toast_if_changed(&toast_for_updates, &last_toast_for_updates, &slot.toast);
                    if slot.preferences.desktop_notifications_enabled {
                        notify_new_messages(&prev_chat_list, &slot.chat_list, prev_focused_chat_id.as_deref());
                    }
                }
            }
        }
    });

    let manager_for_focus = manager.clone();
    window.connect_is_active_notify(move |w| {
        if w.is_active() {
            manager_for_focus.dispatch(AppAction::AppForegrounded);
        }
    });

    window.present();
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
    header.back.set_visible(!state.router.screen_stack.is_empty());
    header
        .new_chat
        .set_visible(matches!(screen, Screen::ChatList));
    header
        .settings
        .set_visible(matches!(screen, Screen::ChatList));
    header
        .chat_info
        .set_visible(matches!(screen, Screen::Chat { .. }));

    let title_text = chat_title(&screen, state).unwrap_or_else(|| screens::title(&screen).to_string());
    header.title.set_label(&title_text);

    let widget = screens::render(&screen, state, manager);
    slot.append(&widget);
}

fn notify_new_messages(
    prev: &[ChatThreadSnapshot],
    current: &[ChatThreadSnapshot],
    focused_chat_id: Option<&str>,
) {
    let prev_map: HashMap<&str, &ChatThreadSnapshot> =
        prev.iter().map(|c| (c.chat_id.as_str(), c)).collect();
    for chat in current {
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
