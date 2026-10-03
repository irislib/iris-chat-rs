use super::*;

impl AppCore {
    pub(in crate::core) fn send_group_message(
        &mut self,
        chat_id: &str,
        text: &str,
        now: UnixSeconds,
        expires_at_secs: Option<u64>,
    ) {
        if self.reject_removed_group_send(chat_id) {
            return;
        }
        let Some(group_id) = parse_group_id_from_chat_id(chat_id) else {
            self.state.toast = Some("Invalid group id.".to_string());
            return;
        };
        let mut options = pairwise_codec::EncodeOptions::new(now.get(), unix_now_ms());
        options.expiration = expires_at_secs;
        let mut rumor =
            match self.prepare_group_event(&group_id, CHAT_MESSAGE_KIND, text, Vec::new(), options)
            {
                Ok(rumor) => rumor,
                Err(error) => {
                    self.state.toast = Some(error.to_string());
                    return;
                }
            };
        let message_id = rumor.id().to_hex();
        let payload = match serde_json::to_vec(&rumor) {
            Ok(payload) => payload,
            Err(error) => {
                self.state.toast = Some(error.to_string());
                return;
            }
        };
        // Group encryption and checkpointing can be expensive with a backlog.
        // Show the authored row first; completion updates this same ID.
        self.push_outgoing_message_with_id(
            message_id.clone(),
            chat_id,
            text.to_string(),
            now.get(),
            expires_at_secs,
            DeliveryState::Queued,
        );
        self.emit_pending_message_state();
        let result = self
            .protocol_engine
            .as_mut()
            .map(|engine| engine.send_group_payload(&group_id, payload, Some(message_id.clone())));
        match result {
            Some(Ok(result)) => {
                let delivery = if result.event_ids.is_empty() {
                    DeliveryState::Queued
                } else {
                    DeliveryState::Pending
                };
                let publish_effects = result
                    .effects
                    .iter()
                    .filter(|effect| matches!(effect, ProtocolEffect::Publish(_)))
                    .count();
                let delivery_publish_effects = result
                    .effects
                    .iter()
                    .filter(|effect| {
                        matches!(
                            effect,
                            ProtocolEffect::Publish(publish) if publish.inner_event_id.is_some()
                        )
                    })
                    .count();
                self.push_debug_log(
                    "message.group.send.appcore",
                    format!(
                        "chat_id={chat_id} message_id={message_id} event_ids={} effects={} signed={} delivery_publish={} targets={}",
                        result.event_ids.len(),
                        result.effects.len(),
                        publish_effects,
                        delivery_publish_effects,
                        summarize_group_send_effect_targets(&result.effects)
                    ),
                );
                self.update_message_delivery(chat_id, &message_id, delivery);
                self.process_protocol_engine_effects(result.effects);
                self.sync_message_delivery_trace(chat_id, &message_id);
                self.reconcile_outgoing_message_delivery(chat_id, &message_id);
                self.request_protocol_subscription_refresh();
            }
            Some(Err(error)) => {
                self.update_message_delivery(chat_id, &message_id, DeliveryState::Failed);
                self.state.toast = Some(error.to_string());
            }
            None => {
                self.update_message_delivery(chat_id, &message_id, DeliveryState::Failed);
                self.state.toast = Some("Protocol engine is not ready.".to_string());
            }
        }
    }

    pub(in crate::core) fn send_group_event(
        &mut self,
        chat_id: &str,
        kind: u32,
        content: &str,
        tags: Vec<Vec<String>>,
        now_ms: Option<u64>,
    ) {
        if self.is_removed_from_group(chat_id) {
            return;
        }
        if kind == CHAT_MESSAGE_KIND {
            self.send_group_message(chat_id, content, unix_now(), None);
            return;
        }
        let Some(group_id) = parse_group_id_from_chat_id(chat_id) else {
            return;
        };
        let millis = now_ms.unwrap_or_else(unix_now_ms);
        let tags = match tags
            .into_iter()
            .map(nostr::Tag::parse)
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(tags) => tags,
            Err(error) => {
                self.push_debug_log("group.control.encode", error.to_string());
                return;
            }
        };
        let unsigned = match self.prepare_group_event(
            &group_id,
            kind,
            content,
            tags,
            pairwise_codec::EncodeOptions::new(millis / 1000, millis),
        ) {
            Ok(unsigned) => unsigned,
            Err(error) => {
                self.push_debug_log("group.control.encode", error.to_string());
                return;
            }
        };
        self.capture_device_sync_unsigned(chat_id, &unsigned);
        let payload = match serde_json::to_vec(&unsigned) {
            Ok(payload) => payload,
            Err(error) => {
                self.push_debug_log("group.control.encode", error.to_string());
                return;
            }
        };
        let inner_event_id = unsigned.id.as_ref().map(ToString::to_string);
        let result = self
            .protocol_engine
            .as_mut()
            .map(|engine| engine.send_group_payload(&group_id, payload, inner_event_id.clone()));
        match result {
            Some(Ok(result)) => {
                self.process_protocol_engine_effects(result.effects);
                self.request_protocol_subscription_refresh();
            }
            Some(Err(error)) => self.push_debug_log("group.control.send", error.to_string()),
            None => {}
        }
    }

    pub(in crate::core) fn prepare_group_event(
        &self,
        group_id: &str,
        kind: u32,
        content: &str,
        mut tags: Vec<nostr::Tag>,
        options: pairwise_codec::EncodeOptions,
    ) -> anyhow::Result<UnsignedEvent> {
        let owner = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Create or restore a profile first."))?
            .owner_pubkey;
        // Each intentional send needs a distinct ID, even when linked devices
        // send the same text or repeat a reaction during the same clock tick.
        tags.push(nostr::Tag::parse(["l", group_id])?);
        tags.push(nostr::Tag::parse(["ms", &options.millis.to_string()])?);
        tags.push(nostr::Tag::parse([
            "iris-message-id",
            &uuid::Uuid::new_v4().simple().to_string(),
        ])?);
        if let Some(expiration) = options.expiration {
            tags.push(nostr::Tag::parse(["expiration", &expiration.to_string()])?);
        }
        let mut rumor = UnsignedEvent::new(
            owner,
            Timestamp::from_secs(options.created_at_secs),
            Kind::Custom(kind as u16),
            tags,
            content.to_string(),
        );
        rumor.ensure_id();
        Ok(rumor)
    }
}
