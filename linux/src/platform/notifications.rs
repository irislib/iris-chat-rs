use adw::prelude::*;
use gtk::gio;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Serialize, Deserialize)]
pub struct NotificationTarget {
    pub owner: String,
    pub chat: String,
    session: String,
}

#[derive(Serialize, Deserialize)]
struct NotificationSession {
    id: String,
    chats: BTreeSet<String>,
}

/// Keep notification destinations bound to the account and local login session,
/// including notifications retained by the desktop after the app has exited.
pub struct NotificationRouting {
    path: PathBuf,
    session: NotificationSession,
    pending: Option<(NotificationTarget, Instant)>,
}

impl NotificationRouting {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("notification-session.json");
        let session = std::fs::read(&path)
            .ok()
            .and_then(|data| serde_json::from_slice::<NotificationSession>(&data).ok())
            .filter(|session| !session.id.is_empty())
            .unwrap_or_else(|| NotificationSession {
                id: gtk::glib::uuid_string_random().to_string(),
                chats: BTreeSet::new(),
            });
        let routing = Self {
            path,
            session,
            pending: None,
        };
        routing.persist();
        routing
    }

    pub fn target(&mut self, owner: &str, chat: &str) -> NotificationTarget {
        if self.session.chats.insert(chat.to_owned()) {
            self.persist();
        }
        NotificationTarget {
            owner: owner.to_owned(),
            chat: chat.to_owned(),
            session: self.session.id.clone(),
        }
    }

    pub fn receive(&mut self, payload: &str) -> bool {
        if payload.len() > 2048 {
            return false;
        }
        let Ok(target) = serde_json::from_str::<NotificationTarget>(payload) else {
            return false;
        };
        if target.session != self.session.id
            || target.owner.len() != 64
            || !target.owner.bytes().all(|byte| byte.is_ascii_hexdigit())
            || target.chat.trim().is_empty()
            || target.chat.len() > 512
            || target.chat.chars().any(char::is_control)
        {
            return false;
        }
        self.pending = Some((target, Instant::now()));
        true
    }

    pub fn take(
        &mut self,
        owner: Option<&str>,
        chats: &[iris_chat_core::ChatThreadSnapshot],
    ) -> Option<String> {
        let (target, received) = self.pending.as_ref()?;
        if received.elapsed() > Duration::from_secs(600) {
            self.clear_pending();
            return None;
        }
        let owner = owner?;
        if !target.owner.eq_ignore_ascii_case(owner) {
            self.clear_pending();
            return None;
        }
        if !chats.iter().any(|chat| chat.chat_id == target.chat) {
            return None;
        }
        self.pending.take().map(|(target, _)| target.chat)
    }

    pub fn clear_pending(&mut self) {
        self.pending = None;
    }

    pub fn invalidate(&mut self) {
        if let Some(app) = gio::Application::default() {
            for chat in &self.session.chats {
                app.withdraw_notification(&format!("chat-{chat}"));
            }
            app.withdraw_notification("iris-call");
            app.withdraw_notification("to.iris.chat");
        }
        self.clear_pending();
        self.session.id = gtk::glib::uuid_string_random().to_string();
        self.session.chats.clear();
        self.persist();
    }

    fn persist(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_vec(&self.session) {
            let _ = std::fs::write(&self.path, data);
        }
    }
}

pub fn install_open_chat_action(actions: &impl IsA<gio::ActionMap>, open: impl Fn(&str) + 'static) {
    let action =
        gio::SimpleAction::new("open-notification-chat", Some(gtk::glib::VariantTy::STRING));
    action.connect_activate(move |_, target| {
        if let Some(payload) = target.and_then(|value| value.str()) {
            open(payload);
        }
    });
    actions.add_action(&action);
}

pub fn notify(id: &str, title: &str, body: &str, target: &NotificationTarget) {
    let Some(app) = gio::Application::default() else {
        return;
    };
    let Ok(payload) = serde_json::to_string(target) else {
        return;
    };
    let notification = gio::Notification::new(title);
    notification.set_body(Some(body));
    notification.set_default_action_and_target_value(
        "app.open-notification-chat",
        Some(&payload.to_variant()),
    );
    app.send_notification(Some(id), &notification);
}
