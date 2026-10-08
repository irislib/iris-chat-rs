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
    file_drop_target: Option<gtk::DropTarget>,
    rendered: Option<(CurrentChatSnapshot, PreferencesSnapshot)>,
    timeline: Option<timeline::Timeline>,
    notice: gtk::Box,
}

impl ChatView {
    pub fn new(chat_id: &str) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        let css = gtk::CssProvider::new();
        css.load_from_data(
            ".file-drop-target { outline: 2px solid @accent_color; outline-offset: -4px; }",
        );
        #[allow(deprecated)]
        root.style_context()
            .add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
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
            file_drop_target: None,
            rendered: None,
            timeline: None,
            notice: gtk::Box::new(gtk::Orientation::Vertical, 0),
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
            self.root
                .insert_action_group("message", gtk::gio::ActionGroup::NONE);
            if let Some(target) = self.file_drop_target.take() {
                self.root.remove_controller(&target);
                self.root.remove_css_class("file-drop-target");
            }
            self.rendered = None;
            self.timeline = None;
            let loading = gtk::Label::new(Some("Loading chat…"));
            loading.add_css_class("dim-label");
            loading.set_vexpand(true);
            self.body.append(&loading);
            return;
        };
        mark_visible_seen(chat, manager);

        if self.timeline.is_none() {
            clear(&self.body);
            let timeline = timeline::Timeline::new(&self.chat_id, manager);
            self.body.append(&timeline.viewport.scroll);
            self.body.append(&self.notice);
            self.timeline = Some(timeline);
        }
        self.timeline
            .as_mut()
            .unwrap()
            .update(chat, &state.preferences, manager);
        // Metadata and drafts do not recreate the viewport or unchanged rows.
        let mut content = chat.clone();
        content.draft.clear();
        content.messages.clear();
        content.typing_indicators.clear();
        content.direct_chat_capability = None;
        let key = (content, state.preferences.clone());
        if self.rendered.as_ref() != Some(&key) {
            clear(&self.notice);
            self.notice
                .append(&crate::widgets::contact_actions::name_notice(chat, manager));
            self.rendered = Some(key);
        }

        let gate = if is_removed_group(chat) {
            Some(removed_group_bar())
        } else if matches!(chat.kind, ChatKind::Direct)
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
            self.root
                .insert_action_group("message", gtk::gio::ActionGroup::NONE);
            if let Some(target) = self.file_drop_target.take() {
                self.root.remove_controller(&target);
                self.root.remove_css_class("file-drop-target");
            }
            self.footer.append(&gate);
        } else if let Some(composer) = &self.composer {
            composer.update(chat, state);
        } else {
            clear(&self.footer);
            let composer = composer::Composer::new(chat, state, manager);
            composer.install_edit_action(&self.root, manager, &self.chat_id);
            let viewport = self.timeline.as_ref().unwrap().viewport.clone();
            composer.on_send(move || viewport.follow_latest());
            let target = composer.file_drop_target(manager, &self.chat_id);
            self.root.add_controller(target.clone());
            self.file_drop_target = Some(target);
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
