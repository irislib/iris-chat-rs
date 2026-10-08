use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

impl AppCore {
    // Share one cancellation flag between all carriers for an event. Weak
    // entries keep this registry bounded to tasks which are still alive.
    pub(in crate::core) fn publication_task_token(&self, event_id: &str) -> Arc<AtomicBool> {
        let mut tasks = self.publication_task_cancellations.borrow_mut();
        tasks.retain(|_, token| token.strong_count() > 0);
        if let Some(token) = tasks.get(event_id).and_then(std::sync::Weak::upgrade) {
            return token;
        }
        let token = Arc::new(AtomicBool::new(false));
        tasks.insert(event_id.to_owned(), Arc::downgrade(&token));
        token
    }

    pub(in crate::core) fn cancel_pending_publication_tasks(&self, target: &str) {
        let mut tasks = self.publication_task_cancellations.borrow_mut();
        for pending in self
            .pending_relay_publishes
            .values()
            .filter(|pending| pending.chat_id.as_deref() == Some(target))
        {
            if let Some(token) = tasks
                .remove(&pending.event_id)
                .and_then(|token| token.upgrade())
            {
                token.store(true, Ordering::Release);
            }
            if let Some(mesh) = &self.device_sync {
                if let Ok(mut outbox) = mesh.nearby_outbox.write() {
                    outbox.forget(&pending.event_id);
                }
            }
        }
    }

    pub(in crate::core) fn cancel_all_publication_tasks(&self) {
        for token in self.publication_task_cancellations.borrow_mut().values() {
            if let Some(token) = token.upgrade() {
                token.store(true, Ordering::Release);
            }
        }
        self.publication_task_cancellations.borrow_mut().clear();
    }
}
