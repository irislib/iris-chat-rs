use super::*;
use crate::state::MessageEditSnapshot;

pub(super) const MESSAGE_EDIT_KIND: u32 = 1009;
pub(super) const MESSAGE_DELETE_KIND: u32 = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessageMutation {
    pub chat_id: String,
    pub id: String,
    pub author: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    pub message_id: String,
    pub operation: String,
    pub content: String,
}

impl MessageMutation {
    pub(super) fn millis(&self) -> u64 {
        self.created_at_ms
            .unwrap_or(self.created_at.saturating_mul(1000))
    }
}

pub(super) fn editable_message(message: &ChatMessageSnapshot) -> bool {
    !message.deleted_for_everyone
        && matches!(message.kind, ChatMessageKind::User)
        && !message.body.trim().is_empty()
        && message.attachments.is_empty()
        && message.direct_transfer.is_none()
        && message.call.is_none()
        && !message.body.starts_with("iris-direct-file-v1:")
}

impl AppCore {
    pub(super) fn message_for_mutation(&self, chat: &str, id: &str) -> Option<ChatMessageSnapshot> {
        self.message_for_mutation_result(chat, id).ok().flatten()
    }

    pub(super) fn message_for_mutation_result(
        &self,
        chat: &str,
        id: &str,
    ) -> anyhow::Result<Option<ChatMessageSnapshot>> {
        if let Some(message) = self
            .threads
            .get(chat)
            .and_then(|t| t.messages.iter().find(|m| m.id == id))
        {
            return Ok(Some(message.clone()));
        }
        Ok(self
            .app_store
            .load_messages_around(chat, id, 0, 0)?
            .first()
            .map(chats::chat_message_from_persisted))
    }

    pub(super) fn mutate_own_message(&mut self, chat: &str, id: &str, text: Option<&str>) {
        let Some(chat) = self.normalize_chat_id(chat) else {
            return;
        };
        if self.reject_removed_group_send(&chat) {
            return;
        }
        let Some(owner) = self.logged_in.as_ref().map(|login| login.owner_pubkey) else {
            return;
        };
        let Some(message) = self.message_for_mutation(&chat, id) else {
            return;
        };
        if message.author_owner_pubkey_hex.as_deref() != Some(owner.to_hex().as_str())
            || !message.is_outgoing
            || message.deleted_for_everyone
            || !matches!(message.kind, ChatMessageKind::User)
            || matches!(
                message.delivery,
                DeliveryState::Queued | DeliveryState::Failed
            )
            || message
                .expires_at_secs
                .is_some_and(|until| until <= unix_now().get())
        {
            return;
        }
        let content = text.unwrap_or("").trim();
        if text.is_some()
            && (!editable_message(&message)
                || content.is_empty()
                || content.len() > 32 * 1024
                || content == message.body
                || !extract_message_attachments(content).1.is_empty()
                || content.starts_with("iris-direct-file-v1:"))
        {
            return;
        }
        let kind = if text.is_some() {
            MESSAGE_EDIT_KIND
        } else {
            MESSAGE_DELETE_KIND
        };
        let millis = self
            .message_mutation_records(&chat, id)
            .iter()
            .fold(unix_now_ms(), |ms, record| {
                ms.max(record.millis().saturating_add(1))
            });
        let event = (|| -> anyhow::Result<UnsignedEvent> {
            let mut tags = vec![
                nostr::Tag::parse(["e", id])?,
                nostr::Tag::parse(["k", "14"])?,
            ];
            if let Some(expiry) = message.expires_at_secs {
                tags.push(nostr::Tag::parse(["expiration", &expiry.to_string()])?);
            }
            if let Some(group) = parse_group_id_from_chat_id(&chat) {
                self.prepare_group_event(
                    &group,
                    kind,
                    content,
                    tags,
                    pairwise_codec::EncodeOptions::new(millis / 1000, millis),
                )
            } else {
                tags.push(nostr::Tag::parse(["p", chat.as_str()])?);
                tags.push(nostr::Tag::parse(["ms", &millis.to_string()])?);
                tags.push(nostr::Tag::parse([
                    "iris-message-id",
                    &uuid::Uuid::new_v4().simple().to_string(),
                ])?);
                let mut event = UnsignedEvent::new(
                    owner,
                    Timestamp::from_secs(millis / 1000),
                    Kind::Custom(kind as u16),
                    tags,
                    content,
                );
                event.ensure_id();
                Ok(event)
            }
        })();
        let Ok(event) = event else { return };
        // Only project after the encrypted send has been accepted into the durable protocol queue.
        let sent = if let Some(group) = parse_group_id_from_chat_id(&chat) {
            let result = serde_json::to_vec(&event).ok().and_then(|payload| {
                self.protocol_engine.as_mut().map(|engine| {
                    engine.send_group_payload(&group, payload, event.id.map(|id| id.to_hex()))
                })
            });
            match result {
                Some(Ok(result)) => {
                    self.process_protocol_engine_effects(result.effects);
                    true
                }
                Some(Err(error)) => {
                    self.push_debug_log("message.mutation.send", error.to_string());
                    false
                }
                None => false,
            }
        } else {
            let result = PublicKey::from_hex(&chat).ok().and_then(|peer| {
                self.protocol_engine.as_mut().map(|engine| {
                    engine.send_direct_unsigned_event(peer, &chat, event.clone(), unix_now())
                })
            });
            match result {
                Some(Ok(result)) => {
                    self.process_protocol_engine_effects(result.effects);
                    true
                }
                Some(Err(error)) => {
                    self.push_debug_log("message.mutation.send", error.to_string());
                    false
                }
                None => false,
            }
        };
        if sent {
            if !self.capture_device_sync_unsigned(&chat, &event) {
                self.state.toast = Some(
                    "Message update sent, but couldn't be saved here. Please try again.".into(),
                );
            }
            self.request_protocol_subscription_refresh();
            self.persist_best_effort();
        } else {
            self.state.toast = Some("Couldn't update message. Please try again.".into());
        }
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn project_message_mutations(&mut self, chat: &str, target: &str) -> bool {
        let mut message = match self.message_for_mutation_result(chat, target) {
            Ok(Some(message)) => message,
            Ok(None) => return true,
            Err(_) => return false,
        };
        if !matches!(message.kind, ChatMessageKind::User) {
            return true;
        }
        let Some(author) = message.author_owner_pubkey_hex.clone() else {
            return true;
        };
        let Ok(mut mutations) = self.message_mutation_records_result(chat, target) else {
            return false;
        };
        if mutations.is_empty() && !message.deleted_for_everyone {
            return true;
        }
        mutations.retain(|m| {
            m.author == author
                && m.created_at >= message.created_at_secs
                && m.expires_at.is_none_or(|expiry| expiry > unix_now().get())
        });
        mutations.sort_by(|a, b| (a.millis(), &a.id).cmp(&(b.millis(), &b.id)));
        // A stored deletion was accepted under the preference at receipt time.
        // Changing that preference later cannot undo an accepted retraction.
        let deleted =
            message.deleted_for_everyone || mutations.iter().any(|m| m.operation == "delete");
        if deleted {
            message.deleted_for_everyone = true;
            message.body.clear();
            message.edit_history.clear();
            message.attachments.clear();
            message.reactions.clear();
            message.reactors.clear();
            message.call = None;
            message.direct_transfer = None;
        } else if editable_message(&message) {
            let edits: Vec<_> = mutations.iter().filter(|m| m.operation == "edit").collect();
            if !edits.is_empty() {
                let original =
                    message
                        .edit_history
                        .first()
                        .cloned()
                        .unwrap_or_else(|| MessageEditSnapshot {
                            id: message.id.clone(),
                            body: message.body.clone(),
                            created_at_secs: message.created_at_secs,
                        });
                message.edit_history = vec![original];
                for edit in edits {
                    message.edit_history.push(MessageEditSnapshot {
                        id: edit.id.clone(),
                        body: edit.content.clone(),
                        created_at_secs: edit.created_at,
                    });
                }
                if let Some(latest) = message.edit_history.last() {
                    message.body = latest.body.clone();
                }
            }
        }
        // Honor a durable deletion in memory even if its database projection
        // fails. Deferred originals must never be displayed or re-synced while
        // the retained control is waiting for a successful retry.
        if deleted {
            if let Some(current) = self
                .threads
                .get_mut(chat)
                .and_then(|t| t.messages.iter_mut().find(|m| m.id == target))
            {
                *current = message.clone();
            }
        }
        if let Err(error) = self.app_store.save_message_mutation_projection(&message) {
            self.push_debug_log("message.mutation.save", error.to_string());
            return false;
        }
        if let Some(current) = self
            .threads
            .get_mut(chat)
            .and_then(|t| t.messages.iter_mut().find(|m| m.id == target))
        {
            *current = message;
        }
        true
    }
}
