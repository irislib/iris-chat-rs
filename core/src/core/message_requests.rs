use super::*;

pub(super) const MAX_UNCONNECTED_REQUESTS: usize = 50;
const MAX_REQUEST_MESSAGES: usize = 20;
const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestAccess {
    Accepted,
    Network,
    Unconnected,
    Denied,
}

impl AppCore {
    // This decision is made only after the transport authenticates/decrypts the
    // sender. Never put social identities or the graph into network filters.
    pub(super) fn direct_request_access(&self, peer: &str) -> RequestAccess {
        let Some(login) = &self.logged_in else {
            return RequestAccess::Denied;
        };
        if peer == login.owner_pubkey.to_hex() {
            return RequestAccess::Accepted;
        }
        if self.is_owner_blocked(peer) {
            return RequestAccess::Denied;
        }
        let decision = self.request_decision(peer);
        if decision
            .as_ref()
            .is_some_and(|state| state.decision == "rejected")
        {
            return RequestAccess::Denied;
        }
        if decision
            .as_ref()
            .is_some_and(|state| state.decision == "accepted")
            || (decision.is_none()
                && (self
                    .preferences
                    .accepted_owner_pubkeys
                    .iter()
                    .any(|id| id == peer)
                    || self.threads.get(peer).is_some_and(|thread| {
                        thread.messages.iter().any(|message| message.is_outgoing)
                    })))
        {
            return RequestAccess::Accepted;
        }
        let graph = self
            .social_graph
            .as_ref()
            .filter(|graph| graph.get_root() == login.owner_pubkey.to_hex());
        let distance = graph.map_or(1000, |graph| graph.get_follow_distance(peer));
        if distance == 1 && decision.is_none() {
            return RequestAccess::Accepted;
        }
        if !self.preferences.accept_overmuted_direct_messages
            && graph.is_some_and(|graph| graph.is_overmuted(peer, 1.0))
        {
            return RequestAccess::Denied;
        }
        if distance < 1000 {
            if self.preferences.accept_social_graph_direct_messages {
                RequestAccess::Network
            } else {
                RequestAccess::Denied
            }
        } else if self.preferences.accept_unknown_direct_messages {
            RequestAccess::Unconnected
        } else {
            RequestAccess::Denied
        }
    }

    pub(super) fn message_request_policy_changed(&mut self) {
        self.request_protocol_subscription_refresh();
        self.mark_mobile_push_dirty();
        self.rebuild_persist_and_emit_state();
    }

    pub(super) fn accept_followed_message_requests(&mut self) {
        let peers: Vec<_> = self
            .threads
            .keys()
            .filter(|peer| {
                !is_group_chat_id(peer)
                    && self.request_decision(peer).is_none()
                    && !self.preferences.accepted_owner_pubkeys.contains(peer)
                    && self.direct_request_access(peer) == RequestAccess::Accepted
            })
            .cloned()
            .collect();
        for peer in peers {
            self.sync_request_acceptance(&peer);
        }
    }

    pub(super) fn prepare_direct_request_message(&mut self, peer: &str, body: &str) -> bool {
        match self.direct_request_access(peer) {
            RequestAccess::Denied => return false,
            RequestAccess::Accepted => {
                // Make following-based acceptance durable, so unfollowing later
                // does not turn an established conversation back into a request.
                if !self
                    .preferences
                    .accepted_owner_pubkeys
                    .iter()
                    .any(|id| id == peer)
                    && !self.sync_request_acceptance(peer)
                {
                    return false;
                }
                return true;
            }
            RequestAccess::Network | RequestAccess::Unconnected => {}
        }
        let pending = self.threads.get(peer);
        let count = pending.map_or(0, |thread| thread.messages.len());
        let bytes = pending.map_or(0, |thread| {
            thread
                .messages
                .iter()
                .map(|message| {
                    message.body.len()
                        + serde_json::to_vec(&message.attachments).map_or(0, |v| v.len())
                })
                .sum::<usize>()
        });
        let Ok((stored_count, stored_bytes)) = self.app_store.request_message_usage(peer) else {
            return false;
        };
        if count.max(stored_count) >= MAX_REQUEST_MESSAGES
            || bytes.max(stored_bytes).saturating_add(body.len()) > MAX_REQUEST_BYTES
        {
            return false;
        }
        if self
            .app_store
            .record_request_arrival(peer, unix_now_ms())
            .is_err()
        {
            return false;
        }
        self.make_room_for_unconnected_request(peer)
    }

    fn make_room_for_unconnected_request(&mut self, incoming: &str) -> bool {
        if self.direct_request_access(incoming) != RequestAccess::Unconnected {
            return true;
        }
        let Ok(arrivals) = self.app_store.request_arrivals() else {
            return false;
        };
        let mut pending: Vec<_> = arrivals
            .into_iter()
            .filter(|(peer, _)| {
                self.threads.contains_key(peer)
                    && peer != incoming
                    && self.direct_request_access(peer) == RequestAccess::Unconnected
            })
            .collect();
        pending.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        let remove = pending.len().saturating_sub(MAX_UNCONNECTED_REQUESTS - 1);
        for (peer, _) in pending.into_iter().take(remove) {
            let Some(thread) = self.threads.get(&peer) else {
                continue;
            };
            let cutoff = unix_now().get().max(thread.updated_at_secs);
            // Reuse durable, privately synced deletion cutoffs. Replayed old
            // messages cannot resurrect an evicted request after restart.
            if !self.apply_chat_deletion(&peer, cutoff) {
                return false;
            }
        }
        if remove > 0 {
            self.broadcast_device_sync_snapshot();
        }
        true
    }
}
