use super::*;
use iris_chat_core::ChatMessageSnapshot;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub(super) struct Paging {
    generation: u64,
    scope: Option<(String, String)>,
    loading_before: Option<String>,
    exhausted_before: Option<String>,
    recent_ids: Option<HashSet<String>>,
    read_ids: HashSet<String>,
}
impl Paging {
    pub(super) fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.scope = None;
        self.loading_before = None;
        self.exhausted_before = None;
        self.recent_ids = None;
        self.read_ids.clear();
    }
    fn scope(state: &AppState) -> Option<(String, String)> {
        let active = state
            .router
            .screen_stack
            .last()
            .unwrap_or(&state.router.default_screen);
        let Screen::Chat { chat_id } = active else {
            return None;
        };
        let account = state.account.as_ref()?;
        let chat = state
            .current_chat
            .as_ref()
            .filter(|chat| &chat.chat_id == chat_id)?;
        Some((account.public_key_hex.clone(), chat.chat_id.clone()))
    }
    pub(super) fn update_scope(&mut self, state: &AppState) {
        let scope = Self::scope(state);
        if self.scope != scope {
            self.clear();
            self.scope = scope;
            self.recent_ids = state
                .current_chat
                .as_ref()
                .map(|chat| chat.messages.iter().map(|m| m.id.clone()).collect());
        }
    }
    pub(super) fn reconcile(&mut self, previous: &AppState, incoming: &mut AppState) {
        let scope = Self::scope(incoming);
        let recent_ids = (scope.is_some() && scope == self.scope)
            .then(|| self.recent_ids.clone())
            .flatten();
        self.update_scope(incoming);
        self.recent_ids = incoming
            .current_chat
            .as_ref()
            .map(|chat| chat.messages.iter().map(|m| m.id.clone()).collect());
        preserve_page(previous, incoming, recent_ids.as_ref());
    }
    fn complete(
        &mut self,
        generation: u64,
        chat_id: &str,
        before: String,
        page: Option<iris_chat_core::CurrentChatSnapshot>,
        state: &mut AppState,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        self.loading_before = None;
        let Some(page) = page else {
            return false;
        };
        let Some(chat) = state
            .current_chat
            .as_mut()
            .filter(|chat| chat.chat_id == chat_id)
        else {
            return false;
        };
        if page.chat_id != chat_id {
            return false;
        }
        if page.messages.len() < ROUTE_CHAT_SNAPSHOT_LIMIT as usize {
            self.exhausted_before = Some(
                page.messages
                    .first()
                    .map(|m| m.id.clone())
                    .unwrap_or(before),
            );
        }
        let current_ids: HashSet<_> = chat.messages.iter().map(|m| &m.id).collect();
        let valid_page: Vec<_> = page
            .messages
            .into_iter()
            .filter(|m| !self.read_ids.contains(&m.id) || current_ids.contains(&m.id))
            .collect();
        self.read_ids.clear();
        let messages = merge_messages(&valid_page, &chat.messages);
        if chat.messages == messages {
            return false;
        }
        chat.messages = messages;
        true
    }
}

impl AppManager {
    pub fn load_older_messages(self: &Rc<Self>, chat_id: &str) -> bool {
        let state = self.local_state.borrow();
        let Some(chat) = state
            .current_chat
            .as_ref()
            .filter(|chat| chat.chat_id == chat_id)
        else {
            return false;
        };
        let Some(first) = chat.messages.first() else {
            return false;
        };
        if state.account.is_none() {
            return false;
        }
        let mut paging = self.history_paging.borrow_mut();
        paging.update_scope(&state);
        if paging.scope.is_none()
            || paging.loading_before.is_some()
            || paging.exhausted_before.as_ref() == Some(&first.id)
        {
            return false;
        }
        let before = first.id.clone();
        let generation = paging.generation;
        paging.loading_before = Some(before.clone());
        paging.read_ids = chat.messages.iter().map(|m| m.id.clone()).collect();
        let chat_id = chat_id.to_string();
        let ffi = self.ffi.clone();
        let (tx, rx) = async_channel::bounded(1);
        let query_chat = chat_id.clone();
        let query_before = before.clone();
        thread::spawn(move || {
            let page = catch_unwind(AssertUnwindSafe(|| {
                ffi.chat_snapshot_before(query_chat, query_before, ROUTE_CHAT_SNAPSHOT_LIMIT)
            }))
            .ok()
            .flatten();
            let _ = tx.send_blocking(page);
        });
        let manager = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(page) = rx.recv().await else {
                return;
            };
            let Some(manager) = manager.upgrade() else {
                return;
            };
            let updated = manager.history_paging.borrow_mut().complete(
                generation,
                &chat_id,
                before,
                page,
                &mut manager.local_state.borrow_mut(),
            );
            if updated {
                manager.redraw_ui();
            }
        });
        true
    }
}

// Second input wins overlap; stable sorting preserves the database's order
// within one timestamp and keeps a late page from undoing a live edit/receipt.
pub(super) fn merge_messages(
    older: &[ChatMessageSnapshot],
    current: &[ChatMessageSnapshot],
) -> Vec<ChatMessageSnapshot> {
    let mut ids: HashSet<&str> = current.iter().map(|message| message.id.as_str()).collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut result: Vec<_> = older
        .iter()
        .filter(|message| {
            message.expires_at_secs.is_none_or(|expiry| expiry > now) && ids.insert(&message.id)
        })
        .cloned()
        .collect();
    result.extend_from_slice(current);
    result.sort_by_key(|message| message.created_at_secs);
    result
}

pub(super) fn preserve_page(
    previous: &AppState,
    next: &mut AppState,
    old_recent_ids: Option<&HashSet<String>>,
) {
    if previous
        .account
        .as_ref()
        .map(|account| &account.public_key_hex)
        != next.account.as_ref().map(|account| &account.public_key_hex)
    {
        return;
    }
    let Some((old, current)) = previous
        .current_chat
        .as_ref()
        .zip(next.current_chat.as_mut())
    else {
        return;
    };
    if old.chat_id != current.chat_id {
        return;
    }
    let Some(recent_ids) = old_recent_ids else {
        return;
    };
    let history: Vec<_> = old
        .messages
        .iter()
        .filter(|m| !recent_ids.contains(&m.id))
        .cloned()
        .collect();
    current.messages = merge_messages(&history, &current.messages);
}

#[cfg(feature = "ui-tests")]
#[path = "history_tests.rs"]
mod tests;
#[cfg(feature = "ui-tests")]
pub use tests::verify_ui;
