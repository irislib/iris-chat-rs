use super::*;

impl AppCore {
    pub(in crate::core) fn next_device_sync_control_millis(
        &self,
        chat: &str,
        kind: u32,
        tags: &[nostr::Tag],
        proposed: u64,
    ) -> u64 {
        let Some(account) = self.logged_in.as_ref() else {
            return proposed;
        };
        let keys = match kind {
            REACTION_KIND => message_ids_from_tags(tags.iter())
                .into_iter()
                .map(|target| {
                    serde_json::json!(["reaction", chat, target, account.owner_pubkey.to_hex()])
                        .to_string()
                })
                .collect::<Vec<_>>(),
            CHAT_SETTINGS_KIND => parse_group_id_from_chat_id(chat)
                .map(|id| vec![serde_json::json!(["groupSettings", id]).to_string()])
                .unwrap_or_default(),
            _ => return proposed,
        };
        keys.into_iter().fold(proposed, |millis, key| {
            let previous = match self.load_sync_record(&RecordLocator::Head(key)) {
                Some(DeviceSyncRecord::Reaction { reaction }) => {
                    effective_ms(reaction.created_at, reaction.created_at_ms)
                }
                Some(DeviceSyncRecord::GroupSettings { settings }) => {
                    effective_ms(settings.created_at, settings.created_at_ms)
                }
                _ => return millis,
            };
            millis.max(previous.saturating_add(1))
        })
    }
    pub(in crate::core) fn cache_device_sync_profile(&mut self, event: &Event) -> bool {
        if event.kind != Kind::Metadata
            || event.verify().is_err()
            || event.created_at.as_secs() > unix_now().get().saturating_add(300)
        {
            return false;
        }
        let record = DeviceSyncRecord::Profile {
            event: event.clone(),
        };
        let existed = self.sync_record_is_stored(&record);
        let Ok(accepted) = self.store_sync_record(&record) else {
            return false;
        };
        if accepted && !existed {
            self.broadcast_device_sync_snapshot();
        }
        accepted
    }
    pub(in crate::core) fn capture_device_sync_unsigned(
        &mut self,
        chat: &str,
        event: &UnsignedEvent,
    ) -> bool {
        let mut event = event.clone();
        event.ensure_id();
        let Some(id) = event.id else { return false };
        self.capture_device_sync_control(
            chat,
            &id.to_hex(),
            &event.pubkey.to_hex(),
            event.created_at.as_secs(),
            event.kind.as_u16() as u32,
            &event.content,
            &event.tags.iter().cloned().collect::<Vec<_>>(),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(in crate::core) fn capture_device_sync_control(
        &mut self,
        chat: &str,
        id: &str,
        author: &str,
        created_at: u64,
        kind: u32,
        content: &str,
        tags: &[nostr::Tag],
    ) -> bool {
        if kind == MESSAGE_EDIT_KIND || kind == MESSAGE_DELETE_KIND {
            let targets = message_ids_from_tags(tags.iter());
            let [target] = targets.as_slice() else {
                return true;
            };
            let ms_tags: Vec<_> = tags
                .iter()
                .filter(|tag| tag.as_slice().first().is_some_and(|s| s == "ms"))
                .collect();
            if ms_tags.len() > 1 {
                return true;
            }
            let created_at_ms = match ms_tags.first() {
                Some(tag) => match tag.as_slice().get(1).and_then(|s| s.parse().ok()) {
                    Some(ms) => Some(ms),
                    None => return true,
                },
                None => None,
            };
            let record = DeviceSyncRecord::MessageMutation {
                mutation: MessageMutation {
                    chat_id: chat.to_string(),
                    id: id.to_string(),
                    author: author.to_string(),
                    created_at,
                    created_at_ms,
                    expires_at: message_expiration_from_tags(tags.iter()),
                    message_id: target.clone(),
                    operation: if kind == MESSAGE_EDIT_KIND {
                        "edit"
                    } else {
                        "delete"
                    }
                    .into(),
                    content: content.to_string(),
                },
            };
            let DeviceSyncRecord::MessageMutation { mutation } = &record else {
                unreachable!()
            };
            match self.message_mutation_allowed(mutation) {
                Ok(true) => {}
                Ok(false) => return true,
                Err(_) => return false,
            }
            let existed = self.sync_record_is_stored(&record);
            let accepted = self.apply_sync_record(record);
            if accepted && !existed {
                self.broadcast_device_sync_snapshot();
            }
            return accepted;
        }
        if kind != REACTION_KIND && kind != CHAT_SETTINGS_KIND {
            return true;
        }
        let created_at_ms = match tags
            .iter()
            .find(|tag| tag.as_slice().first().is_some_and(|key| key == "ms"))
        {
            Some(tag) => match tag.as_slice().get(1).and_then(|ms| ms.parse().ok()) {
                Some(ms) => Some(ms),
                None => return false,
            },
            None => None,
        };
        let records = if kind == REACTION_KIND {
            let targets = message_ids_from_tags(tags.iter());
            // A typed reaction ID names one original event and one target.
            // Ambiguous legacy batches cannot invent independent provenance.
            if targets.len() != 1 {
                return true;
            }
            let Some(target) = targets.first() else {
                return false;
            };
            let legacy = serde_json::from_str::<serde_json::Value>(content).ok();
            let emoji = legacy
                .as_ref()
                .filter(|value| {
                    value["type"] == "reaction"
                        && value["messageId"].as_str() == Some(target.as_str())
                })
                .and_then(|value| value["emoji"].as_str())
                .unwrap_or(content);
            targets
                .into_iter()
                .map(|message_id| DeviceSyncRecord::Reaction {
                    reaction: DeviceSyncReaction {
                        chat_id: chat.to_string(),
                        id: id.to_string(),
                        author: author.to_string(),
                        created_at,
                        created_at_ms,
                        message_id,
                        emoji: emoji.to_string(),
                    },
                })
                .collect::<Vec<_>>()
        } else if let Some(group_id) = parse_group_id_from_chat_id(chat) {
            let Some(ttl) = chat_settings_ttl_seconds(content) else {
                return false;
            };
            vec![DeviceSyncRecord::GroupSettings {
                settings: DeviceSyncGroupSettings {
                    group_id,
                    id: id.to_string(),
                    author: author.to_string(),
                    created_at,
                    created_at_ms,
                    message_ttl_seconds: (ttl > 0).then_some(ttl),
                },
            }]
        } else {
            return true;
        };
        let mut accepted = false;
        for record in records {
            if !self.sync_record_allowed(&record) {
                continue;
            }
            let existed = self.sync_record_is_stored(&record);
            if self.store_sync_record(&record).unwrap_or(false) {
                accepted = true;
                if !existed {
                    self.broadcast_device_sync_snapshot();
                }
            }
        }
        accepted
    }
    pub(in crate::core) fn project_device_sync_reaction_target(
        &mut self,
        chat: &str,
        target: &str,
        author: &str,
        content: &str,
    ) {
        if self.has_reaction_head(chat, target, author) {
            self.restore_device_sync_reactions(chat, target);
        } else if self.sync_record_author_allowed(chat, author, false)
            && !self.sync_reaction_target_expired(chat, target)
        {
            let legacy = serde_json::from_str::<serde_json::Value>(content).ok();
            let emoji = legacy
                .as_ref()
                .filter(|value| value["type"] == "reaction" && value["messageId"] == target)
                .and_then(|value| value["emoji"].as_str())
                .unwrap_or(content);
            if emoji.len() <= 256 {
                self.apply_incoming_reaction_to_chat(chat, target, author, emoji);
            }
        }
    }
    pub(in crate::core) fn restore_device_sync_reactions(&mut self, chat: &str, message: &str) {
        self.project_message_mutations(chat, message);
        for record in self.reaction_records_for_message(chat, message) {
            if let DeviceSyncRecord::Reaction { reaction: r } = record {
                if r.chat_id == chat
                    && r.message_id == message
                    && self.sync_record_author_allowed(chat, &r.author, false)
                {
                    self.apply_incoming_reaction_to_chat(chat, message, &r.author, &r.emoji);
                }
            }
        }
    }
}
