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
    raw_ids: Option<HashSet<String>>,
    excluded_ids: HashSet<String>,
    removed_ids: HashSet<String>,
}
impl Paging {
    pub(super) fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.scope = None;
        self.loading_before = None;
        self.exhausted_before = None;
        self.recent_ids = None;
        self.raw_ids = None;
        self.excluded_ids.clear();
        self.removed_ids.clear();
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
        if previous.message_visibility_revision != incoming.message_visibility_revision {
            self.clear();
        }
        let scope = Self::scope(incoming);
        let recent_ids = (scope.is_some() && scope == self.scope)
            .then(|| self.recent_ids.clone())
            .flatten();
        if scope != self.scope {
            self.clear();
            self.scope = scope;
        }
        let raw_ids: Option<HashSet<String>> = incoming
            .current_chat
            .as_ref()
            .map(|chat| chat.messages.iter().map(|m| m.id.clone()).collect());
        if let (Some(old), Some(raw)) = (&self.raw_ids, &raw_ids) {
            self.removed_ids.extend(old.difference(raw).cloned());
            self.removed_ids.retain(|id| !raw.contains(id));
        }
        self.recent_ids = project_page(
            previous,
            incoming,
            recent_ids.as_ref(),
            self.raw_ids.as_ref(),
            &mut self.excluded_ids,
        );
        self.raw_ids = raw_ids;
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
            .filter(|m| {
                !self.removed_ids.contains(&m.id)
                    && (!self.read_ids.contains(&m.id) || current_ids.contains(&m.id))
            })
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

#[cfg(feature = "ui-tests")]
pub(super) fn preserve_page(
    previous: &AppState,
    next: &mut AppState,
    old_recent_ids: Option<&HashSet<String>>,
) {
    project_page(previous, next, old_recent_ids, None, &mut HashSet::new());
}

fn project_page(
    previous: &AppState,
    next: &mut AppState,
    old_recent_ids: Option<&HashSet<String>>,
    old_raw_ids: Option<&HashSet<String>>,
    excluded_ids: &mut HashSet<String>,
) -> Option<HashSet<String>> {
    let chat = next.current_chat.as_mut()?;
    let raw = &chat.messages;
    let first_shared = old_recent_ids.and_then(|ids| raw.iter().position(|m| ids.contains(&m.id)));
    let start =
        first_shared.or_else(|| old_raw_ids.is_none().then(|| raw.len().saturating_sub(80)));
    if let Some(start) = start {
        excluded_ids.extend(raw[..start].iter().map(|m| m.id.clone()));
    }
    let recent: Vec<_> = raw
        .iter()
        .filter(|m| !excluded_ids.contains(&m.id))
        .cloned()
        .collect();
    let recent_ids = recent.iter().map(|m| m.id.clone()).collect();
    let mut older = Vec::new();
    if previous
        .account
        .as_ref()
        .map(|account| &account.public_key_hex)
        == next.account.as_ref().map(|account| &account.public_key_hex)
    {
        if let (Some(old), Some(ids)) = (previous.current_chat.as_ref(), old_recent_ids) {
            if old.chat_id == chat.chat_id {
                let fresh: std::collections::HashMap<_, _> =
                    raw.iter().map(|m| (&m.id, m)).collect();
                older = old
                    .messages
                    .iter()
                    .filter(|m| !ids.contains(&m.id))
                    .filter(|m| {
                        fresh.contains_key(&m.id)
                            || old_raw_ids.is_none_or(|ids| !ids.contains(&m.id))
                    })
                    .map(|m| fresh.get(&m.id).copied().unwrap_or(m).clone())
                    .collect();
            }
        }
    }
    chat.messages = merge_messages(&older, &recent);
    Some(recent_ids)
}

#[cfg(feature = "ui-tests")]
#[path = "history_tests.rs"]
mod tests;
#[cfg(feature = "ui-tests")]
pub use tests::verify_ui;
