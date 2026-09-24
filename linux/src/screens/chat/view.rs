use super::*;
use iris_chat_core::DirectChatCapabilityState;

pub struct ChatView {
    pub root: gtk::Box,
    pub chat_id: String,
    body: gtk::Box,
    footer: gtk::Box,
    body_overlay: gtk::Overlay,
    capability: Option<DirectChatCapabilityState>,
    capability_status: Option<gtk::Widget>,
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
        let body_overlay = gtk::Overlay::new();
        body_overlay.set_vexpand(true);
        body_overlay.set_child(Some(&body));
        root.append(&body_overlay);
        root.append(&footer);
        Self {
            root,
            chat_id: chat_id.into(),
            body,
            footer,
            body_overlay,
            capability: None,
            capability_status: None,
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
            self.clear_capability();
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
        content.direct_chat_capability = None;
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
        } else {
            None
        };

        let capability = if gate.is_none() && matches!(chat.kind, ChatKind::Direct) {
            chat.direct_chat_capability
                .clone()
                .filter(|c| !matches!(c, DirectChatCapabilityState::Available))
        } else {
            None
        };
        if capability != self.capability {
            self.clear_capability();
            if capability.is_some() {
                if let Some(status) = delayed_capability_bar(chat, manager) {
                    status.set_valign(gtk::Align::End);
                    self.body_overlay.add_overlay(&status);
                    self.capability_status = Some(status);
                }
            }
            self.capability = capability;
        }

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

    fn clear_capability(&mut self) {
        if let Some(status) = self.capability_status.take() {
            self.body_overlay.remove_overlay(&status);
        }
        self.capability = None;
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
