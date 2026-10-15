use adw::prelude::*;
use gtk::gio;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Serialize, Deserialize)]
pub struct NotificationTarget {
    pub owner: String,
    pub chat: String,
    session: String,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct NotificationSession {
    id: String,
    chats: BTreeSet<String>,
    #[serde(default)]
    messages: BTreeMap<String, NotificationTarget>,
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
                messages: BTreeMap::new(),
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
            message_id: None,
            kind: None,
        }
    }

    pub fn message_target(
        &mut self,
        owner: &str,
        chat: &str,
        message_id: Option<String>,
    ) -> (String, NotificationTarget) {
        let mut target = self.target(owner, chat);
        target.message_id = message_id;
        target.kind = Some("message".into());
        let id = format!("chat-{owner}-{chat}");
        self.session.messages.insert(id.clone(), target.clone());
        self.persist();
        (id, target)
    }

    pub fn clear_read(&mut self, data_dir: &Path, owner: &str, fully_read: &BTreeSet<String>) {
        let ids = self.read_ids(owner, fully_read, |messages| {
            iris_chat_core::read_desktop_notification_indexes(
                data_dir.to_string_lossy().into_owned(),
                owner.into(),
                messages,
            )
        });
        let Some(app) = gio::Application::default() else {
            return;
        };
        for id in &ids {
            app.withdraw_notification(id);
            self.session.messages.remove(id);
        }
        if !ids.is_empty() {
            self.persist();
        }
    }

    fn read_ids(
        &self,
        owner: &str,
        fully_read: &BTreeSet<String>,
        lookup: impl FnOnce(Vec<iris_chat_core::DesktopNotificationMessage>) -> Vec<u64>,
    ) -> Vec<String> {
        let matching: Vec<_> = self
            .session
            .messages
            .iter()
            .filter(|(_, target)| {
                target.kind.as_deref() == Some("message")
                    && target.owner.eq_ignore_ascii_case(owner)
            })
            .collect();
        let exact: Vec<_> = matching
            .iter()
            .filter(|(_, target)| target.message_id.as_ref().is_some_and(|id| !id.is_empty()))
            .collect();
        let messages = exact
            .iter()
            .map(|(_, target)| iris_chat_core::DesktopNotificationMessage {
                chat_id: target.chat.clone(),
                message_id: target.message_id.clone().unwrap(),
            })
            .collect();
        let read: BTreeSet<_> = lookup(messages).into_iter().collect();
        let mut ids: Vec<_> = exact
            .iter()
            .enumerate()
            .filter(|(index, _)| read.contains(&(*index as u64)))
            .map(|(_, (id, _))| id.to_string())
            .collect();
        ids.extend(
            matching
                .iter()
                .filter(|(_, target)| {
                    target.message_id.is_none() && fully_read.contains(&target.chat)
                })
                .map(|(id, _)| id.to_string()),
        );
        ids
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
            for id in self.session.messages.keys() {
                app.withdraw_notification(id);
            }
            app.withdraw_notification("iris-call");
            app.withdraw_notification("to.iris.chat");
        }
        self.clear_pending();
        self.session.id = gtk::glib::uuid_string_random().to_string();
        self.session.chats.clear();
        self.session.messages.clear();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_message_identity_clears_exactly_while_other_accounts_and_calls_remain() {
        let directory = tempfile::tempdir().unwrap();
        let owner = "a".repeat(64);
        let mut routing = NotificationRouting::new(directory.path());
        let (seen_id, _) = routing.message_target(&owner, "chat", Some("seen".into()));
        let (unread_id, _) = routing.message_target(&owner, "other-chat", Some("unread".into()));
        routing.message_target(&"b".repeat(64), "chat", Some("seen".into()));
        let call = routing.target(&owner, "chat");
        routing.session.messages.insert("iris-call".into(), call);
        let (legacy_id, _) = routing.message_target(&owner, "legacy", None);
        routing.persist();
        let restored = NotificationRouting::new(directory.path());
        let read = restored.read_ids(&owner, &BTreeSet::new(), |messages| {
            assert_eq!(messages.len(), 2);
            messages
                .iter()
                .enumerate()
                .filter_map(|(index, message)| {
                    (message.message_id == "seen").then_some(index as u64)
                })
                .collect()
        });
        assert_eq!(read, vec![seen_id]);
        assert!(restored.session.messages.contains_key(&unread_id));
        assert!(restored.session.messages.contains_key("iris-call"));
        assert_eq!(
            restored.read_ids(&owner, &BTreeSet::from(["legacy".into()]), |_| vec![]),
            vec![legacy_id]
        );
    }

    #[test]
    fn replacing_a_chat_notification_tracks_its_latest_message() {
        let directory = tempfile::tempdir().unwrap();
        let owner = "a".repeat(64);
        let mut routing = NotificationRouting::new(directory.path());
        routing.message_target(&owner, "chat", Some("seen".into()));
        routing.message_target(&owner, "chat", Some("still-unread".into()));
        let restored = NotificationRouting::new(directory.path());
        assert!(restored
            .read_ids(&owner, &BTreeSet::new(), |messages| {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].message_id, "still-unread");
                vec![]
            })
            .is_empty());
    }
}
