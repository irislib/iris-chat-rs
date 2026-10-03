use iris_chat_core::{present_person_name, AppState, ChatKind};

pub fn explicit(nickname: Option<&str>, profile_name: Option<&str>) -> Option<String> {
    [nickname, profile_name]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .map(str::to_owned)
}

pub fn explicit_for(owner: &str, state: &AppState) -> Option<String> {
    if let Some(chat) = state
        .current_chat
        .as_ref()
        .filter(|chat| chat.kind == ChatKind::Direct && chat.chat_id == owner)
    {
        return explicit(chat.nickname.as_deref(), chat.profile_name.as_deref());
    }
    state
        .chat_list
        .iter()
        .find(|chat| chat.kind == ChatKind::Direct && chat.chat_id == owner)
        .and_then(|chat| explicit(chat.nickname.as_deref(), chat.profile_name.as_deref()))
}

pub fn markup(label: &str, identity: &str, explicit: Option<String>) -> String {
    let name = present_person_name(label.to_owned(), identity.to_owned(), explicit);
    let escaped = gtk::glib::markup_escape_text(&name.name);
    if name.is_fallback {
        format!("<i>{escaped}</i>")
    } else {
        escaped.to_string()
    }
}
