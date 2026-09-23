use super::*;
use crate::state::CallHistorySnapshot;

impl AppCore {
    pub(in crate::core) fn is_live_call_history(&self, message: &ChatMessageSnapshot) -> bool {
        message.call.as_ref().is_some_and(|history| {
            self.calls
                .active
                .as_ref()
                .is_some_and(|call| call.id == history.call_id)
        })
    }

    /// Save conservative summaries immediately: a process exit while ringing
    /// must not erase the call. Only local media readiness counts as answered.
    pub(super) fn persist_call_history(&mut self, disposition: Option<&str>) {
        let (Some(active), Some(snapshot)) = (&self.calls.active, &self.state.call) else {
            return;
        };
        let elsewhere = disposition == Some("answered_elsewhere");
        let answered = if elsewhere || disposition == Some("declined") {
            None
        } else {
            active.answered
        };
        let outcome = if elsewhere {
            "answered_elsewhere"
        } else if answered.is_some() {
            "answered"
        } else if disposition == Some("declined") {
            "declined"
        } else if active.outgoing {
            "canceled"
        } else {
            "missed"
        };
        let call = CallHistorySnapshot {
            call_id: active.id.clone(),
            direction: if active.outgoing {
                "outgoing"
            } else {
                "incoming"
            }
            .into(),
            outcome: outcome.into(),
            video: if snapshot.connected_at_secs.is_some() {
                active.video
            } else {
                active.offered_video
            },
            started_at_secs: snapshot.started_at_secs,
            answered_at_secs: answered.map(|(at, _)| at),
            ended_at_secs: unix_now().get().max(snapshot.started_at_secs),
            duration_secs: answered.map_or(0, |(_, clock)| clock.elapsed().as_secs()),
        };
        let owner = active.owner.clone();
        self.save_call_history(&owner, call);
    }

    fn save_call_history(&mut self, owner: &str, call: CallHistorySnapshot) {
        // A chat deleted while a call is active must stay deleted.
        if self.chat_activity_is_deleted(owner, call.started_at_secs) {
            return;
        }
        let thread = self.ensure_thread_record(owner, call.started_at_secs);
        let id = format!("call:{}", call.call_id);
        let label = match call.outcome.as_str() {
            "answered" if call.direction == "outgoing" => "Outgoing",
            "answered" => "Incoming",
            "declined" => "Declined",
            "canceled" => "Canceled",
            _ => "Missed",
        };
        let body = if call.outcome == "answered_elsewhere" {
            "Answered on another device".into()
        } else {
            format!(
                "{label} {} call",
                if call.video { "video" } else { "voice" }
            )
        };
        if let Some(message) = thread.messages.iter_mut().find(|message| message.id == id) {
            message.body = body;
            message.call = Some(call);
        } else {
            thread.updated_at_secs = thread.updated_at_secs.max(call.started_at_secs);
            thread.insert_message_sorted(ChatMessageSnapshot {
                id,
                chat_id: owner.into(),
                kind: ChatMessageKind::System,
                author: String::new(),
                author_owner_pubkey_hex: None,
                author_picture_url: None,
                body,
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: call.direction == "outgoing",
                created_at_secs: call.started_at_secs,
                expires_at_secs: None,
                delivery: DeliveryState::Seen,
                recipient_deliveries: Vec::new(),
                delivery_trace: Default::default(),
                source_event_id: None,
                call: Some(call),
            });
        }
        self.rebuild_state();
        self.persist_best_effort();
    }

    /// Persisted IDs also reject delayed offers after process restart or after
    /// their message page has left memory. Active retransmits are handled first.
    pub(super) fn call_history_contains(&self, owner: &str, id: &str) -> bool {
        let message_id = format!("call:{id}");
        self.threads.get(owner).is_some_and(|thread| {
            thread
                .messages
                .iter()
                .any(|message| message.id == message_id)
        }) || self
            .app_store
            .message_exists(owner, Some(&message_id), None)
            .unwrap_or(false)
    }
}
