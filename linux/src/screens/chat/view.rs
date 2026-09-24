use super::*;

pub struct ChatView {
    pub root: gtk::Box,
    pub chat_id: String,
    body: gtk::Box,
    footer: gtk::Box,
    composer: Option<composer::Composer>,
    rendered: Option<(CurrentChatSnapshot, PreferencesSnapshot)>,
}

impl ChatView {
    pub fn new(chat_id: &str) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_vexpand(true);
        let footer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&body);
        root.append(&footer);
        Self {
            root,
            chat_id: chat_id.into(),
            body,
            footer,
            composer: None,
            rendered: None,
        }
    }

    pub fn update(&mut self, state: &AppState, manager: &Rc<AppManager>) {
        let Some(chat) = state
            .current_chat
            .as_ref()
            .filter(|c| c.chat_id == self.chat_id)
        else {
            clear(&self.body);
            clear(&self.footer);
            self.composer = None;
            self.rendered = None;
            let loading = gtk::Label::new(Some("Loading chat…"));
            loading.add_css_class("dim-label");
            loading.set_vexpand(true);
            self.body.append(&loading);
            return;
        };
        mark_visible_seen(chat, manager);

        // Draft-only updates must not reset message scrolling either.
        let mut content = chat.clone();
        content.draft.clear();
        let key = (content, state.preferences.clone());
        if self.rendered.as_ref() != Some(&key) {
            clear(&self.body);
            self.body.append(&ttl_strip(chat, manager));
            self.body
                .append(&messages_view(chat, &state.preferences, manager));
            self.rendered = Some(key);
        }

        let gate = if matches!(chat.kind, ChatKind::Direct)
            && is_user_blocked(&state.preferences, &chat.chat_id)
        {
            Some(blocked_bar(chat, manager))
        } else if matches!(chat.kind, ChatKind::Direct) && chat.is_request {
            Some(message_request_bar(chat, manager))
        } else if let Some(capability) = chat
            .direct_chat_capability
            .as_ref()
            .filter(|c| !matches!(c, DirectChatCapabilityState::Available))
        {
            Some(capability_bar(chat, capability, manager))
        } else {
            None
        };

        if let Some(gate) = gate {
            clear(&self.footer);
            self.composer = None;
            self.footer.append(&gate);
        } else if let Some(composer) = &self.composer {
            composer.update(chat, state);
        } else {
            clear(&self.footer);
            let composer = composer::Composer::new(chat, state, manager);
            self.footer.append(&composer.root);
            self.composer = Some(composer);
        }
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
