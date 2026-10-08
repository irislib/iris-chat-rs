use super::*;

const OPTIONS: &[(&str, Option<u64>)] = &[
    ("Off", None),
    ("5 minutes", Some(300)),
    ("1 hour", Some(3_600)),
    ("24 hours", Some(86_400)),
    ("1 week", Some(604_800)),
    ("1 month", Some(2_592_000)),
    ("3 months", Some(7_776_000)),
];

pub(crate) fn settings_row(manager: &Rc<AppManager>, chat_id: &str) -> Option<adw::ActionRow> {
    let state = manager.current_state();
    let chat = state
        .current_chat
        .as_ref()
        .filter(|chat| chat.chat_id == chat_id)?;
    let row = adw::ActionRow::builder()
        .title("Disappearing messages")
        .subtitle(label(chat.message_ttl_seconds))
        .activatable(true)
        .sensitive(!is_removed_group(chat))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(
        "preferences-system-time-symbolic",
    ));
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let manager = manager.clone();
    let chat_id = chat_id.to_owned();
    row.connect_activated(move |row| {
        let parent = row.root().and_downcast::<gtk::Window>();
        present_options(parent.as_ref(), &manager, &chat_id, row);
    });
    Some(row)
}

fn present_options(
    parent: Option<&gtk::Window>,
    manager: &Rc<AppManager>,
    chat_id: &str,
    settings: &adw::ActionRow,
) {
    if !can_change_chat(manager, chat_id) {
        return;
    }
    let dialog = adw::Dialog::builder()
        .title("Disappearing messages")
        .content_width(320)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    list.set_margin_start(16);
    list.set_margin_end(16);
    list.set_margin_bottom(16);
    let current = manager
        .current_state()
        .current_chat
        .and_then(|chat| chat.message_ttl_seconds)
        .filter(|seconds| *seconds > 0);
    for &(label, ttl_seconds) in OPTIONS {
        let option = adw::ActionRow::builder()
            .title(label)
            .activatable(true)
            .build();
        if ttl_seconds == current {
            option.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
        }
        let closing = dialog.downgrade();
        let settings = settings.downgrade();
        let manager = manager.clone();
        let chat_id = chat_id.to_owned();
        option.connect_activated(move |_| {
            if can_change_chat(&manager, &chat_id) {
                manager.dispatch(AppAction::SetChatMessageTtl {
                    chat_id: chat_id.clone(),
                    ttl_seconds,
                });
                if let Some(settings) = settings.upgrade() {
                    settings.set_subtitle(label);
                }
            }
            if let Some(dialog) = closing.upgrade() {
                dialog.close();
            }
        });
        list.append(&option);
    }
    toolbar.set_content(Some(&list));
    dialog.set_child(Some(&toolbar));
    crate::widgets::dialogs::present(&dialog, parent);
}

fn label(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds.filter(|seconds| *seconds > 0) else {
        return "Off".into();
    };
    if let Some((label, _)) = OPTIONS.iter().find(|(_, value)| *value == Some(seconds)) {
        return (*label).into();
    }
    let (amount, unit) = if seconds < 60 {
        (seconds, "second")
    } else if seconds < 3_600 {
        (seconds / 60, "minute")
    } else if seconds < 86_400 {
        (seconds / 3_600, "hour")
    } else {
        (seconds / 86_400, "day")
    };
    format!("{amount} {unit}{}", if amount == 1 { "" } else { "s" })
}
