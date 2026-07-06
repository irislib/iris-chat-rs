use super::protocol::PROTOCOL_RECONNECT_CHECK_SECS;
use super::*;

fn coalesce_protocol_fetch_effects(effects: &mut Vec<ProtocolEffect>) {
    let mut seen = HashSet::new();
    effects.retain(|effect| match effect {
        ProtocolEffect::FetchProtocolState { filters, reason } => {
            seen.insert(format!("{reason}:{filters:?}"))
        }
        _ => true,
    });
}

impl AppCore {
    pub(super) fn handle_relay_event(&mut self, event: Event) {
        self.handle_relay_event_with_channel(event, "message servers");
    }

    pub(super) fn handle_relay_event_with_channel(&mut self, event: Event, channel: &str) {
        let event_id = event.id.to_string();
        let kind = event.kind.as_u16() as u32;
        let is_app_keys_protocol_event = kind == APP_KEYS_EVENT_KIND && is_app_keys_event(&event);
        let is_group_roster_protocol_event =
            kind == GROUP_ROSTER_FACT_KIND && is_group_roster_fact_event(&event);
        let is_invite_protocol_event = is_protocol_invite_event(&event);
        let is_invite_response_protocol_event = kind == INVITE_RESPONSE_KIND;
        let message_has_header = kind == MESSAGE_EVENT_KIND && event_has_tag(&event, "header");
        let is_known_group_sender_key_event = kind == MESSAGE_EVENT_KIND
            && self
                .protocol_engine
                .as_ref()
                .is_some_and(|engine| engine.is_known_group_sender_event_author(event.pubkey));
        let should_try_group_sender_key_event =
            kind == MESSAGE_EVENT_KIND && (!message_has_header || is_known_group_sender_key_event);
        if self.has_seen_event(&event_id) {
            // Only persist + rebuild + emit when the transport-channel
            // set actually grew. Without this guard, every mirrored
            // relay re-delivery of an already-seen event burns a full
            // snapshot serialize + state rebuild — measurably hot on
            // accounts subscribed to a handful of relays.
            if self.add_transport_channel_for_event_id(&event_id, channel) {
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
            }
            if !self.should_replay_seen_protocol_event(&event, is_invite_protocol_event) {
                return;
            }
        }
        self.event_transport_channels
            .insert(event_id.clone(), channel.to_string());

        if self.logged_in.is_none() {
            if self.handle_pending_link_device_response(event) {
                self.remember_event(event_id);
            }
            return;
        }

        self.push_debug_log("relay.event", format!("kind_raw={} id={event_id}", kind));
        let protocol_inputs_changed = is_app_keys_protocol_event
            || is_group_roster_protocol_event
            || is_invite_protocol_event
            || is_invite_response_protocol_event;

        if kind == 0 {
            if self.apply_profile_metadata_event(&event) {
                self.remember_event(event_id);
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
                return;
            }
            self.remember_event(event_id);
            return;
        }

        match kind {
            APP_KEYS_EVENT_KIND if is_app_keys_protocol_event => {
                self.debug_event_counters.app_keys_events += 1;
                match self.apply_app_keys_event(&event) {
                    Ok(_) => {
                        self.remember_event(event_id);
                        self.request_protocol_subscription_refresh();
                        self.persist_best_effort();
                        self.rebuild_state();
                        self.emit_state();
                    }
                    Err(error) => {
                        self.push_debug_log("appcore.protocol.app_keys.error", error.to_string());
                        self.rebuild_state();
                        self.emit_state();
                    }
                }
                return;
            }
            GROUP_ROSTER_FACT_KIND if is_group_roster_protocol_event => {
                match self
                    .protocol_engine
                    .as_mut()
                    .map(|engine| engine.ingest_group_roster_fact_event(&event))
                    .transpose()
                {
                    Ok(Some(Some(result))) => {
                        if let Some(snapshot) = result.snapshot {
                            let previous = self.groups.get(&snapshot.group_id).cloned();
                            self.apply_group_roster_snapshot(
                                snapshot.clone(),
                                unix_now().get().max(snapshot.updated_at.get()),
                            );
                            self.apply_group_metadata_notice(previous.as_ref(), &snapshot);
                            self.request_protocol_subscription_refresh();
                        }
                        self.process_protocol_engine_retry_batch(
                            "group_roster_fact",
                            result.retry_batch,
                        );
                        self.remember_event(event_id);
                        self.persist_best_effort();
                        self.rebuild_state();
                        self.emit_state();
                    }
                    Ok(Some(None)) => {
                        self.remember_event(event_id);
                    }
                    Ok(None) => {
                        self.remember_event(event_id);
                    }
                    Err(error) => {
                        self.push_debug_log(
                            "appcore.protocol.group_roster.error",
                            error.to_string(),
                        );
                        self.rebuild_state();
                        self.emit_state();
                    }
                }
                return;
            }
            INVITE_EVENT_KIND if is_invite_protocol_event => {
                self.debug_event_counters.invite_events += 1;
                let retry_results = self
                    .protocol_engine
                    .as_mut()
                    .map(|engine| engine.observe_invite_event(&event))
                    .transpose();
                match retry_results {
                    Ok(Some(results)) => {
                        self.process_protocol_engine_retry_batch("invite_event", results);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.push_debug_log("appcore.protocol.invite.error", error.to_string());
                        self.rebuild_state();
                        self.emit_state();
                        return;
                    }
                }
            }
            INVITE_EVENT_KIND => {
                self.remember_event(event_id);
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
                return;
            }
            INVITE_RESPONSE_KIND => {
                self.debug_event_counters.invite_response_events += 1;
                if self.handle_private_chat_invite_response(&event) {
                    self.remember_event(event_id);
                    self.persist_best_effort();
                    self.rebuild_state();
                    self.emit_state();
                    return;
                }
                let retry_results = self
                    .protocol_engine
                    .as_mut()
                    .map(|engine| engine.observe_invite_response_event(&event))
                    .transpose();
                match retry_results {
                    Ok(Some(results)) => {
                        self.process_protocol_engine_retry_batch("invite_response", results);
                        self.refresh_local_authorization_state();
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.push_debug_log(
                            "appcore.protocol.invite_response.error",
                            error.to_string(),
                        );
                        self.rebuild_state();
                        self.emit_state();
                        return;
                    }
                }
            }
            MESSAGE_EVENT_KIND => {
                if should_try_group_sender_key_event {
                    let group_result = match self
                        .protocol_engine
                        .as_mut()
                        .map(|engine| engine.process_group_outer_event(&event))
                    {
                        Some(Ok(group_result)) => group_result,
                        Some(Err(error)) => {
                            self.push_debug_log(
                                "appcore.protocol.group.outer.error",
                                error.to_string(),
                            );
                            self.persist_best_effort();
                            self.rebuild_state();
                            self.emit_state();
                            return;
                        }
                        None => Default::default(),
                    };
                    if group_result.consumed
                        || !group_result.events.is_empty()
                        || !group_result.effects.is_empty()
                        || !group_result.queued_targets.is_empty()
                    {
                        self.debug_event_counters.group_events += 1;
                        let should_remember_group_event = group_result.consumed
                            || !group_result.events.is_empty()
                            || !group_result.effects.is_empty();
                        if !group_result.queued_targets.is_empty() {
                            self.handle_queued_protocol_targets(
                                "group.outer",
                                &group_result.queued_targets,
                            );
                        }
                        for group_event in group_result.events {
                            self.apply_group_decrypted_event(group_event);
                        }
                        if !group_result.effects.is_empty() {
                            self.process_protocol_engine_effects(group_result.effects);
                        }
                        if should_remember_group_event {
                            self.remember_event(event_id);
                        }
                        self.schedule_fast_protocol_retry_if_pending();
                        self.persist_best_effort();
                        self.rebuild_state();
                        self.emit_state();
                        return;
                    }

                    self.remember_event(event_id);
                    return;
                }
                let unknown_message_author = self
                    .protocol_engine
                    .as_ref()
                    .is_some_and(|engine| !engine.is_known_message_author(event.pubkey));
                if unknown_message_author {
                    let targets_local_recipient =
                        message_has_header && self.message_targets_local_protocol_recipient(&event);
                    let is_potential_group_sender_key_event = message_has_header
                        && self.protocol_engine.as_ref().is_some_and(|engine| {
                            engine.is_potential_group_sender_key_event(&event)
                        });
                    let is_group_sender_key_candidate_with_group_context = message_has_header
                        && self.protocol_engine.as_ref().is_some_and(|engine| {
                            engine.is_group_sender_key_candidate_with_local_group_context(&event)
                        });
                    let has_tracked_sender_session = message_has_header
                        && self.protocol_engine.as_ref().is_some_and(|engine| {
                            engine.header_message_sender_has_tracked_session(&event)
                        });
                    let should_keep_unknown_header = is_potential_group_sender_key_event
                        || is_group_sender_key_candidate_with_group_context
                        || has_tracked_sender_session
                        || !self.private_chat_invites.is_empty()
                        || targets_local_recipient;
                    if !message_has_header || !should_keep_unknown_header {
                        self.push_debug_log(
                            "appcore.protocol.message.ignored",
                            format!(
                                "unknown message author header={message_has_header} targets_local_recipient={targets_local_recipient} group_candidate={is_potential_group_sender_key_event} group_context_candidate={is_group_sender_key_candidate_with_group_context} tracked_sender_session={has_tracked_sender_session}"
                            ),
                        );
                        return;
                    }
                    self.push_debug_log(
                        "appcore.protocol.message.pending_header",
                        format!(
                            "unknown message author targets_local_recipient={targets_local_recipient} group_candidate={is_potential_group_sender_key_event} group_context_candidate={is_group_sender_key_candidate_with_group_context} tracked_sender_session={has_tracked_sender_session}"
                        ),
                    );
                }
                self.debug_event_counters.message_events += 1;
            }
            _ => {
                self.debug_event_counters.other_events += 1;
            }
        }

        if kind == MESSAGE_EVENT_KIND {
            if self
                .protocol_engine
                .as_ref()
                .is_some_and(|engine| engine.has_pending_inbound_direct_event_id(&event_id))
            {
                self.push_debug_log(
                    "appcore.protocol.message.pending_replay",
                    "already stored as pending inbound",
                );
                self.request_protocol_subscription_refresh();
                self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                    PROTOCOL_RECONNECT_CHECK_SECS,
                ));
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
                return;
            }
            if let Some(protocol_engine) = self.protocol_engine.as_mut() {
                match protocol_engine.process_direct_message_event(&event) {
                    Ok(Some(decrypted)) => {
                        let event_id = decrypted.event_id.clone();
                        self.apply_decrypted_runtime_message_with_metadata(
                            decrypted.sender,
                            decrypted.sender_device,
                            decrypted.conversation_owner,
                            decrypted.content,
                            decrypted.event_id,
                        );
                        if let Some(event_id) = event_id {
                            self.remember_event(event_id);
                        }
                        self.mark_mobile_push_dirty();
                        self.request_protocol_subscription_refresh();
                        self.retry_protocol_engine_pending_work("direct_message");
                        self.persist_best_effort();
                        self.rebuild_state();
                        self.emit_state();
                        return;
                    }
                    Ok(None) => {
                        self.push_debug_log(
                            "appcore.protocol.message.pending",
                            format!("event_id={event_id} author={}", event.pubkey),
                        );
                        let (queued_targets, effects) = self
                            .protocol_engine
                            .as_ref()
                            .map(|engine| {
                                engine.queued_protocol_backfill_effects(
                                    NdrUnixSeconds(unix_now().get()),
                                    "direct_message.pending",
                                )
                            })
                            .unwrap_or_default();
                        self.process_protocol_engine_effects(effects);
                        if queued_targets.is_empty() {
                            self.request_protocol_subscription_refresh();
                            self.schedule_protocol_subscription_liveness_check(
                                Duration::from_secs(PROTOCOL_RECONNECT_CHECK_SECS),
                            );
                        } else {
                            self.handle_queued_protocol_targets(
                                "direct_message.pending",
                                &queued_targets,
                            );
                        }
                        if queued_targets.is_empty() && self.fetch_recent_protocol_state() {
                            self.state.busy.syncing_network = true;
                        }
                        self.schedule_fast_protocol_retry_if_pending();
                    }
                    Err(error) => {
                        self.push_debug_log(
                            "appcore.protocol.message.error",
                            format!("event_id={event_id} author={} error={error}", event.pubkey),
                        );
                        self.remember_event(event_id);
                        return;
                    }
                }
            }
            self.persist_best_effort();
            self.rebuild_state();
            self.emit_state();
            return;
        }
        self.remember_event(event_id);
        if protocol_inputs_changed {
            self.request_protocol_subscription_refresh();
            if self.fetch_recent_protocol_state() {
                self.state.busy.syncing_network = true;
            }
            self.schedule_tracked_peer_catch_up(Duration::from_secs(2));
        }
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    fn message_targets_local_protocol_recipient(&self, event: &Event) -> bool {
        self.protocol_message_recipient_pubkeys()
            .into_iter()
            .any(|pubkey| event_has_pubkey_tag(event, pubkey))
    }

    fn handle_pending_link_device_response(&mut self, event: Event) -> bool {
        let kind = event.kind.as_u16() as u32;
        if kind == APP_KEYS_EVENT_KIND && is_app_keys_event(&event) {
            let event_id = event.id.to_string();
            let Some((owner_pubkey, pending_response, device_keys)) = ({
                let Some(pending) = self.pending_linked_device.as_mut() else {
                    return false;
                };
                let owner = match resolve_app_keys_owner_for_device(
                    &event,
                    pending.device_keys.public_key(),
                ) {
                    Ok(Some(owner)) => owner,
                    _ => return false,
                };
                pending.authorized_owner_pubkey = Some(owner);
                pending.authorized_app_keys_event = Some(event.clone());
                Some((
                    owner,
                    pending.pending_response.take(),
                    pending.device_keys.clone(),
                ))
            }) else {
                return false;
            };

            if let Ok(app_keys) = AppKeys::from_event(&event) {
                self.app_keys.insert(
                    owner_pubkey.to_hex(),
                    known_app_keys_from_ndr(owner_pubkey, &app_keys, event.created_at.as_secs()),
                );
            }

            self.push_debug_log(
                "session.link_authorized",
                format!("event_id={event_id} owner={}", owner_pubkey.to_hex()),
            );

            if let Some(response) = pending_response {
                match self.complete_pending_linked_device(
                    owner_pubkey,
                    response.peer_device_id,
                    response.session_state,
                    device_keys,
                ) {
                    Ok(()) => {
                        let _ = self.apply_app_keys_event(&event);
                    }
                    Err(error) => {
                        self.state.toast = Some(error.to_string());
                        self.rebuild_state();
                        self.emit_state();
                    }
                }
            }
            return true;
        }

        if kind != INVITE_RESPONSE_KIND {
            return false;
        }
        let Some((pairing_invite, device_secret)) =
            self.pending_linked_device.as_ref().map(|pending| {
                (
                    pending.pairing_invite.clone(),
                    pending.device_keys.secret_key().to_secret_bytes(),
                )
            })
        else {
            return false;
        };

        self.debug_event_counters.invite_response_events += 1;
        let event_id = event.id.to_string();
        let response = match nostr_double_ratchet::process_invite_response_event(
            &pairing_invite,
            &event,
            device_secret,
        ) {
            Ok(Some(response)) => response,
            Ok(None) => return false,
            Err(error) => {
                self.push_debug_log(
                    "session.link_response.error",
                    format!("event_id={event_id} error={error}"),
                );
                return false;
            }
        };

        let mut pending_response = Some(PendingLinkInviteResponse {
            peer_device_id: response
                .device_id
                .clone()
                .unwrap_or_else(|| response.invitee_identity.to_hex()),
            session_state: response.session.state,
        });
        let owner_and_device = {
            let Some(pending) = self.pending_linked_device.as_mut() else {
                return false;
            };
            pending
                .authorized_owner_pubkey
                .map(|owner_pubkey| {
                    (
                        owner_pubkey,
                        pending.device_keys.clone(),
                        pending.authorized_app_keys_event.clone(),
                    )
                })
                .or_else(|| {
                    pending.pending_response = pending_response.take();
                    None
                })
        };
        let Some((owner_pubkey, device_keys, app_keys_event)) = owner_and_device else {
            self.push_debug_log(
                "session.link_response",
                format!("event_id={event_id} waiting_for_app_keys_authorization"),
            );
            return true;
        };
        let Some(response) = pending_response else {
            return false;
        };
        let peer_device_id = response.peer_device_id;
        let session_state = response.session_state;

        self.push_debug_log(
            "session.link_response",
            format!(
                "event_id={event_id} owner={} peer_device={peer_device_id}",
                owner_pubkey.to_hex()
            ),
        );

        match self.complete_pending_linked_device(
            owner_pubkey,
            peer_device_id,
            session_state,
            device_keys,
        ) {
            Ok(()) => {
                if let Some(app_keys_event) = app_keys_event {
                    let _ = self.apply_app_keys_event(&app_keys_event);
                }
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
                self.rebuild_state();
                self.emit_state();
            }
        }
        true
    }

    pub(super) fn process_protocol_engine_effects(&mut self, mut effects: Vec<ProtocolEffect>) {
        coalesce_protocol_fetch_effects(&mut effects);
        for effect in effects {
            match effect {
                ProtocolEffect::Publish(publish) => {
                    self.publish_protocol_event(publish);
                }
                ProtocolEffect::FetchProtocolState { filters, reason } => {
                    self.fetch_protocol_state_for_filters(filters, reason);
                }
            }
        }
    }

    pub(super) fn ack_pending_decrypted_deliveries_after_app_persist(&mut self) {
        if let Some(protocol_engine) = self.protocol_engine.as_mut() {
            if let Err(error) = protocol_engine.ack_pending_decrypted_deliveries() {
                self.push_debug_log("appcore.protocol.decrypted_ack.error", error.to_string());
            }
        }
        self.pending_decrypted_delivery_acks.clear();
    }

    /// Process an app-keys event (kind 37368) — adds/removes devices
    /// for an owner. The mobile-push snapshot indexes by tracked owner
    /// + device, so any change there invalidates the cache.
    pub(super) fn apply_app_keys_event(&mut self, event: &Event) -> anyhow::Result<bool> {
        let should_publish_backfilled_owner_app_keys =
            self.logged_in.as_ref().is_some_and(|logged_in| {
                self.defer_owner_app_keys_publish
                    && logged_in.owner_keys.is_some()
                    && logged_in.owner_pubkey == event.pubkey
            });
        let app_keys = self
            .logged_in
            .as_ref()
            .and_then(|logged_in| {
                logged_in
                    .owner_keys
                    .as_ref()
                    .filter(|keys| keys.public_key() == event.pubkey)
                    .and_then(|keys| AppKeys::from_event_with_labels(event, keys).ok())
            })
            .or_else(|| AppKeys::from_event(event).ok());
        let Some(app_keys) = app_keys else {
            return Ok(false);
        };

        let owner_hex = event.pubkey.to_hex();
        let current = self.app_keys.get(&owner_hex).cloned();
        let current_app_keys = current.as_ref().and_then(known_app_keys_to_ndr);
        let current_created_at = current
            .as_ref()
            .map(|known| known.created_at_secs)
            .unwrap_or_default();
        let required_device = self
            .logged_in
            .as_ref()
            .filter(|logged_in| {
                self.defer_owner_app_keys_publish
                    && logged_in.owner_keys.is_some()
                    && logged_in.owner_pubkey == event.pubkey
            })
            .map(|logged_in| {
                DeviceEntry::new(logged_in.device_keys.public_key(), unix_now().get())
            });
        let (effective_app_keys, effective_created_at) = if should_publish_backfilled_owner_app_keys
        {
            let mut merged = current_app_keys.unwrap_or_else(|| AppKeys::new(Vec::new()));
            for device in app_keys.get_all_devices() {
                merged.add_device(device);
            }
            if let Some(required_device) = required_device {
                merged.add_device(required_device);
            }
            (
                merged,
                next_app_keys_created_at(
                    unix_now().get().max(event.created_at.as_secs()),
                    current_created_at,
                ),
            )
        } else {
            let applied = apply_app_keys_snapshot_with_required_device(
                current_app_keys.as_ref(),
                current_created_at,
                &app_keys,
                event.created_at.as_secs(),
                required_device,
            );
            (applied.app_keys, applied.created_at)
        };

        let protocol_retry_batch = if let Some(protocol_engine) = self.protocol_engine.as_mut() {
            protocol_engine.ingest_app_keys_snapshot(
                event.pubkey,
                effective_app_keys.clone(),
                effective_created_at,
            )?
        } else {
            ProtocolRetryBatch::default()
        };

        let mut known =
            known_app_keys_from_ndr(event.pubkey, &effective_app_keys, effective_created_at);
        if should_publish_backfilled_owner_app_keys {
            if let Some(device_pubkey) = self
                .logged_in
                .as_ref()
                .filter(|logged_in| logged_in.owner_pubkey == event.pubkey)
                .map(|logged_in| logged_in.device_keys.public_key())
            {
                self.apply_current_device_labels_to_known_app_keys(&mut known, device_pubkey);
            }
        }
        if current.as_ref() != Some(&known) {
            self.app_keys.insert(owner_hex, known);
        }
        if should_publish_backfilled_owner_app_keys {
            self.defer_owner_app_keys_publish = false;
        }
        self.migrate_verified_device_owner_threads(event.pubkey, &effective_app_keys);
        self.mark_mobile_push_dirty();
        let _authorization_changed = self.refresh_local_authorization_state();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
        self.process_protocol_engine_retry_batch("app_keys", protocol_retry_batch);
        if should_publish_backfilled_owner_app_keys {
            self.publish_local_app_keys();
        }
        Ok(true)
    }
}

impl AppCore {
    fn should_replay_seen_protocol_event(
        &self,
        _event: &Event,
        _is_invite_protocol_event: bool,
    ) -> bool {
        false
    }
}

fn is_protocol_invite_event(event: &Event) -> bool {
    event.kind.as_u16() as u32 == INVITE_EVENT_KIND
        && event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().map(|value| value.as_str()) == Some("d")
                && values
                    .get(1)
                    .is_some_and(|value| value.starts_with(NDR_INVITES_D_TAG_PREFIX))
        })
}

fn event_has_tag(event: &Event, name: &str) -> bool {
    event
        .tags
        .iter()
        .any(|tag| tag.as_slice().first().map(|value| value.as_str()) == Some(name))
}

fn event_has_pubkey_tag(event: &Event, pubkey: PublicKey) -> bool {
    let pubkey_hex = pubkey.to_hex();
    event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(|value| value.as_str()) == Some("p")
            && values.get(1).map(|value| value.as_str()) == Some(pubkey_hex.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Tag;

    #[test]
    fn parses_message_expiration_tag_seconds_and_milliseconds() {
        let tags = vec![Tag::parse(["expiration", "1704067260123"]).expect("expiration tag")];

        assert_eq!(message_expiration_from_tags(&tags), Some(1_704_067_260));
    }
}
