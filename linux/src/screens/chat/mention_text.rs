use super::*;

pub(super) fn label(mention: &iris_chat_core::MessageMention, state: &AppState) -> String {
    let name = chat_info_for_owner(
        &mention.owner_pubkey_hex,
        mention.label.trim_start_matches('@'),
        None,
        state,
    )
    .display_name;
    format!("@{name}")
}

pub(super) fn byte_at_utf16(text: &str, offset: u32) -> Option<usize> {
    let mut units = 0u32;
    for (byte, ch) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += ch.len_utf16() as u32;
    }
    (units == offset).then_some(text.len())
}

pub(super) fn linked(text: &str, state: &AppState) -> chat_links::LinkifiedText {
    let mut output = chat_links::LinkifiedText {
        markup: String::new(),
        urls: vec![],
    };
    let mut cursor = 0;
    for mention in iris_chat_core::message_mentions(text.to_string()) {
        let (Some(start), Some(end)) = (
            byte_at_utf16(text, mention.start),
            byte_at_utf16(text, mention.start + mention.length),
        ) else {
            continue;
        };
        let plain = linkified_text(&text[cursor..start]);
        output.markup.push_str(&plain.markup);
        output.urls.extend(plain.urls);
        output.markup.push_str(&format!(
            "<a href=\"irischat-mention:{}\"><b>{}</b></a>",
            mention.owner_pubkey_hex,
            glib::markup_escape_text(&label(&mention, state))
        ));
        cursor = end;
    }
    let plain = linkified_text(&text[cursor..]);
    output.markup.push_str(&plain.markup);
    output.urls.extend(plain.urls);
    output
}

pub(super) fn install_actions(label: &gtk::Label, manager: &Rc<AppManager>) {
    let manager = manager.clone();
    label.connect_activate_link(move |label, uri| {
        let Some(owner) = uri
            .strip_prefix("irischat-mention:")
            .filter(|owner| iris_chat_core::is_valid_peer_input(owner.to_string()))
        else {
            return glib::Propagation::Proceed;
        };
        let Some(parent) = label
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
        else {
            return glib::Propagation::Stop;
        };
        let info = chat_info_for_owner(owner, "Profile", None, &manager.current_state());
        present_chat_info(Some(&parent), info, manager.clone());
        glib::Propagation::Stop
    });
}
