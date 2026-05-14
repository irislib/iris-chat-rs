use super::*;

impl AppCore {
    pub(super) fn create_group(&mut self, name: &str, member_inputs: &[String]) {
        self.create_group_inner(name, member_inputs, None);
    }

    pub(super) fn create_group_with_picture(
        &mut self,
        name: &str,
        member_inputs: &[String],
        _picture_file_path: &str,
        _picture_filename: &str,
    ) {
        self.create_group_inner(name, member_inputs, None);
    }

    fn create_group_inner(
        &mut self,
        name: &str,
        member_inputs: &[String],
        _picture: Option<(String, String)>,
    ) {
        if self.logged_in.is_none() {
            self.state.toast = Some("Create or restore a profile first.".to_string());
            self.emit_state();
            return;
        }
        if !self.can_use_chats() {
            self.state.toast = Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
            self.emit_state();
            return;
        }

        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            self.state.toast = Some("Group name is required.".to_string());
            self.emit_state();
            return;
        }

        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let member_owners = match parse_owner_inputs(member_inputs, local_owner) {
            Ok(member_owners) => member_owners,
            Err(error) => {
                self.state.toast = Some(error.to_string());
                self.emit_state();
                return;
            }
        };

        self.state.busy.creating_group = true;
        self.emit_state();

        let now = unix_now();
        let result = self
            .protocol_engine
            .as_mut()
            .map(|engine| engine.create_group(trimmed_name.to_string(), member_owners, now));

        match result {
            Some(Ok(result)) => {
                self.process_protocol_engine_effects_with_completions(
                    result.effects,
                    &BTreeMap::new(),
                );
                self.handle_queued_protocol_targets("group.create", &result.queued_targets);
                let Some(group) = result.snapshot else {
                    self.state.toast = Some("Group could not be created.".to_string());
                    self.state.busy.creating_group = false;
                    self.rebuild_state();
                    self.persist_best_effort();
                    self.emit_state();
                    return;
                };
                let chat_id = group_chat_id(&group.group_id);
                self.apply_group_snapshot_to_threads(&group, now.get());
                self.groups.insert(group.group_id.clone(), group.clone());
                self.active_chat_id = Some(chat_id.clone());
                self.screen_stack = vec![Screen::Chat { chat_id }];
                self.apply_group_metadata_notice(None, &group);
                self.request_protocol_subscription_refresh();
                self.schedule_tracked_peer_catch_up(Duration::from_secs(
                    RESUBSCRIBE_CATCH_UP_DELAY_SECS,
                ));
            }
            Some(Err(error)) => self.state.toast = Some(error.to_string()),
            None => self.state.toast = Some("Protocol engine is not ready.".to_string()),
        }

        self.state.busy.creating_group = false;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn update_group_name(&mut self, group_id: &str, name: &str) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            self.state.toast = Some("Group name is required.".to_string());
            self.emit_state();
            return;
        }

        self.state.busy.updating_group = true;
        self.emit_state();
        let previous = self.groups.get(group_id).cloned();
        let result = self
            .protocol_engine
            .as_mut()
            .map(|engine| engine.update_group_name(group_id, trimmed.to_string()));

        match result {
            Some(Ok(result)) => {
                self.process_protocol_engine_effects_with_completions(
                    result.effects,
                    &BTreeMap::new(),
                );
                self.handle_queued_protocol_targets("group.rename", &result.queued_targets);
                if let Some(snapshot) = result.snapshot {
                    self.apply_local_group_snapshot(previous.as_ref(), snapshot, "group.rename")
                }
            }
            Some(Err(error)) => self.state.toast = Some(error.to_string()),
            None => {}
        }
        self.state.busy.updating_group = false;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn update_group_picture(
        &mut self,
        _group_id: &str,
        _file_path: &str,
        _filename: &str,
    ) {
        self.state.toast = Some(
            "Group photos are not supported on the experimental group protocol yet.".to_string(),
        );
        self.emit_state();
    }

    pub(super) fn add_group_members(&mut self, group_id: &str, member_inputs: &[String]) {
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let member_owners = match parse_owner_inputs(member_inputs, local_owner) {
            Ok(member_owners) if !member_owners.is_empty() => member_owners,
            Ok(_) => return,
            Err(error) => {
                self.state.toast = Some(error.to_string());
                self.emit_state();
                return;
            }
        };

        self.state.busy.updating_group = true;
        self.emit_state();
        let previous = self.groups.get(group_id).cloned();
        let result = self
            .protocol_engine
            .as_mut()
            .map(|engine| engine.add_group_members(group_id, member_owners));
        match result {
            Some(Ok(result)) => {
                self.process_protocol_engine_effects_with_completions(
                    result.effects,
                    &BTreeMap::new(),
                );
                self.handle_queued_protocol_targets("group.add_members", &result.queued_targets);
                if let Some(snapshot) = result.snapshot {
                    self.apply_local_group_snapshot(
                        previous.as_ref(),
                        snapshot,
                        "group.add_members",
                    );
                }
            }
            Some(Err(error)) => self.state.toast = Some(error.to_string()),
            None => self.state.toast = Some("Protocol engine is not ready.".to_string()),
        }
        self.state.busy.updating_group = false;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_group_admin(
        &mut self,
        group_id: &str,
        owner_pubkey_hex: &str,
        is_admin: bool,
    ) {
        let Ok(owner) = parse_owner_input(owner_pubkey_hex) else {
            self.state.toast = Some("Invalid member key.".to_string());
            self.emit_state();
            return;
        };
        let previous = self.groups.get(group_id).cloned();
        let result = self
            .protocol_engine
            .as_mut()
            .map(|engine| engine.set_group_admin(group_id, owner, is_admin));
        match result {
            Some(Ok(result)) => {
                self.process_protocol_engine_effects_with_completions(
                    result.effects,
                    &BTreeMap::new(),
                );
                self.handle_queued_protocol_targets(
                    if is_admin {
                        "group.add_admin"
                    } else {
                        "group.remove_admin"
                    },
                    &result.queued_targets,
                );
                if let Some(snapshot) = result.snapshot {
                    self.apply_local_group_snapshot(
                        previous.as_ref(),
                        snapshot,
                        if is_admin {
                            "group.add_admin"
                        } else {
                            "group.remove_admin"
                        },
                    );
                }
            }
            Some(Err(error)) => self.state.toast = Some(error.to_string()),
            None => self.state.toast = Some("Protocol engine is not ready.".to_string()),
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn remove_group_member(&mut self, group_id: &str, owner_pubkey_hex: &str) {
        let Ok(owner) = parse_owner_input(owner_pubkey_hex) else {
            self.state.toast = Some("Invalid member key.".to_string());
            self.emit_state();
            return;
        };
        let previous = self.groups.get(group_id).cloned();
        let result = self
            .protocol_engine
            .as_mut()
            .map(|engine| engine.remove_group_member(group_id, owner));
        match result {
            Some(Ok(result)) => {
                self.process_protocol_engine_effects_with_completions(
                    result.effects,
                    &BTreeMap::new(),
                );
                self.handle_queued_protocol_targets("group.remove_member", &result.queued_targets);
                if let Some(snapshot) = result.snapshot {
                    self.apply_local_group_snapshot(
                        previous.as_ref(),
                        snapshot,
                        "group.remove_member",
                    )
                }
            }
            Some(Err(error)) => self.state.toast = Some(error.to_string()),
            None => self.state.toast = Some("Protocol engine is not ready.".to_string()),
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    fn apply_local_group_snapshot(
        &mut self,
        previous: Option<&GroupSnapshot>,
        group: GroupSnapshot,
        debug_category: &'static str,
    ) {
        self.groups.insert(group.group_id.clone(), group.clone());
        self.apply_group_snapshot_to_threads(&group, unix_now().get());
        self.push_debug_log(debug_category, group.group_id.clone());
        self.apply_group_metadata_notice(previous, &group);
        self.request_protocol_subscription_refresh();
    }

    pub(super) fn apply_group_snapshot_to_threads(
        &mut self,
        group: &GroupSnapshot,
        updated_at_secs: u64,
    ) {
        self.ensure_thread_record(&group_chat_id(&group.group_id), updated_at_secs);
    }

    pub(super) fn apply_group_decrypted_event(&mut self, event: GroupIncomingEvent) {
        self.mark_mobile_push_dirty();
        match event {
            GroupIncomingEvent::MetadataUpdated(group) => {
                let previous = self.groups.get(&group.group_id).cloned();
                self.groups.insert(group.group_id.clone(), group.clone());
                self.apply_group_snapshot_to_threads(
                    &group,
                    unix_now().get().max(group.updated_at.get()),
                );
                self.apply_group_metadata_notice(previous.as_ref(), &group);
            }
            GroupIncomingEvent::Message(message) => {
                let chat_id = group_chat_id(&message.group_id);
                let Ok(sender_owner) = PublicKey::from_slice(&message.sender_owner.to_bytes())
                else {
                    return;
                };
                let body = String::from_utf8_lossy(&message.body).to_string();
                if let Some(runtime_rumor) = parse_runtime_rumor(&body) {
                    self.apply_group_runtime_rumor(&chat_id, sender_owner, runtime_rumor);
                    return;
                }
                let reason = if looks_like_runtime_rumor(&body) {
                    "invalid_runtime_rumor"
                } else {
                    "not_runtime_rumor"
                };
                self.push_debug_log(
                    "group.message.decode.skip",
                    format!(
                        "chat_id={chat_id} sender_owner={} bytes={} reason={reason}",
                        sender_owner.to_hex(),
                        message.body.len()
                    ),
                );
            }
        }
    }

    fn apply_group_runtime_rumor(
        &mut self,
        chat_id: &str,
        sender_owner: PublicKey,
        runtime_rumor: RuntimeRumor,
    ) {
        if runtime_rumor.pubkey != sender_owner {
            self.push_debug_log(
                "group.runtime_rumor.sender_mismatch",
                format!(
                    "chat_id={chat_id} sender_owner={} rumor_pubkey={}",
                    sender_owner.to_hex(),
                    runtime_rumor.pubkey.to_hex()
                ),
            );
            return;
        }
        let local_owner = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey);
        let is_outgoing = local_owner.is_some_and(|local_owner| sender_owner == local_owner);
        let created_at_secs = runtime_rumor.created_at_secs;
        let expires_at_secs = message_expiration_from_tags(runtime_rumor.tags.iter());
        let inner_event_id = runtime_rumor.id.clone();
        match runtime_rumor.kind {
            CHAT_MESSAGE_KIND => {
                self.apply_runtime_text_message(
                    sender_owner,
                    Some(chat_id.to_string()),
                    runtime_rumor.content,
                    created_at_secs,
                    expires_at_secs,
                    Some(inner_event_id.clone()),
                    Some(inner_event_id),
                );
            }
            REACTION_KIND => {
                let sender_hex = sender_owner.to_hex();
                for message_id in message_ids_from_tags(runtime_rumor.tags.iter()) {
                    self.apply_incoming_reaction_to_chat(
                        chat_id,
                        &message_id,
                        &sender_hex,
                        &runtime_rumor.content,
                    );
                }
            }
            RECEIPT_KIND => {
                let delivery = match runtime_rumor.content.as_str() {
                    "seen" => DeliveryState::Seen,
                    _ => DeliveryState::Received,
                };
                self.apply_receipt_to_messages(
                    chat_id,
                    &message_ids_from_tags(runtime_rumor.tags.iter()),
                    delivery,
                    is_outgoing,
                    Some(&sender_owner.to_hex()),
                );
            }
            TYPING_KIND => {
                if !is_outgoing {
                    self.apply_typing_event(
                        chat_id.to_string(),
                        sender_owner.to_hex(),
                        created_at_secs,
                        expires_at_secs,
                    );
                }
            }
            CHAT_SETTINGS_KIND => {
                let actor = self.owner_display_label(&sender_owner.to_hex());
                self.apply_chat_settings_control(
                    chat_id,
                    &actor,
                    chat_settings_ttl_seconds(&runtime_rumor.content),
                    created_at_secs,
                );
            }
            _ => {}
        }
    }

    pub(super) fn apply_group_metadata_notice(
        &mut self,
        previous: Option<&GroupSnapshot>,
        group: &GroupSnapshot,
    ) {
        let chat_id = group_chat_id(&group.group_id);
        let now = unix_now().get();
        match previous {
            None => {
                self.push_system_notice(&chat_id, format!("Group created: {}", group.name), now)
            }
            Some(previous) => {
                if previous.name != group.name {
                    self.push_system_notice(
                        &chat_id,
                        format!("Group renamed to {}", group.name),
                        now,
                    );
                }
                for owner in group
                    .members
                    .iter()
                    .filter(|owner| !previous.members.iter().any(|existing| existing == *owner))
                {
                    self.push_system_notice(
                        &chat_id,
                        format!(
                            "{} joined the group",
                            self.owner_display_label(&owner.to_string())
                        ),
                        now,
                    );
                }
                for owner in previous
                    .members
                    .iter()
                    .filter(|owner| !group.members.iter().any(|existing| existing == *owner))
                {
                    self.push_system_notice(
                        &chat_id,
                        format!(
                            "{} left the group",
                            self.owner_display_label(&owner.to_string())
                        ),
                        now,
                    );
                }
                if previous.admins != group.admins {
                    self.push_system_notice(
                        &chat_id,
                        self.admin_change_notice(previous, group),
                        now,
                    );
                }
            }
        }
    }

    pub(super) fn sync_runtime_groups(&mut self) {}

    fn admin_change_notice(&self, previous: &GroupSnapshot, group: &GroupSnapshot) -> String {
        let added = group
            .admins
            .iter()
            .find(|admin| !previous.admins.iter().any(|existing| existing == *admin));
        if let Some(owner) = added {
            return format!(
                "{} became an admin",
                self.owner_display_label(&owner.to_string())
            );
        }
        let removed = previous
            .admins
            .iter()
            .find(|admin| !group.admins.iter().any(|existing| existing == *admin));
        if let Some(owner) = removed {
            return format!(
                "{} is no longer an admin",
                self.owner_display_label(&owner.to_string())
            );
        }
        "Group admins changed".to_string()
    }
}
