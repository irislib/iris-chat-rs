use super::*;

impl AppCore {
    pub(super) fn create_group(&mut self, name: &str, member_inputs: &[String]) {
        self.create_group_inner(name, member_inputs, None);
    }

    pub(super) fn create_group_with_picture(
        &mut self,
        name: &str,
        member_inputs: &[String],
        picture_file_path: &str,
        picture_filename: &str,
    ) {
        let picture = (!picture_file_path.trim().is_empty())
            .then(|| {
                (
                    picture_file_path.trim().to_string(),
                    picture_filename.trim().to_string(),
                )
            })
            .filter(|(_, filename)| !filename.is_empty());
        self.create_group_inner(name, member_inputs, picture);
    }

    fn create_group_inner(
        &mut self,
        name: &str,
        member_inputs: &[String],
        picture: Option<(String, String)>,
    ) {
        if self.logged_in.is_none() {
            self.state.toast = Some("Create or restore an account first.".to_string());
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
        let member_hexes = member_owners
            .iter()
            .map(PublicKey::to_hex)
            .collect::<Vec<_>>();
        let member_refs = member_hexes.iter().map(String::as_str).collect::<Vec<_>>();

        self.state.busy.creating_group = true;
        self.emit_state();

        let now = unix_now();
        let result = {
            let logged_in = self.logged_in.as_ref().expect("checked above");
            logged_in
                .ndr_runtime
                .with_group_context(|session_manager, group_manager, _| {
                    let mut send_pairwise = |recipient: PublicKey, rumor: &UnsignedEvent| {
                        session_manager
                            .send_event(recipient, rumor.clone())
                            .map(|_| ())
                    };
                    group_manager.create_group(
                        trimmed_name,
                        &member_refs,
                        CreateGroupOptions {
                            send_pairwise: Some(&mut send_pairwise),
                            fanout_metadata: true,
                            now_ms: Some(now.get().saturating_mul(1000)),
                        },
                    )
                })
        };

        let mut created_group_id = None;
        match result {
            Ok(result) => {
                for owner in member_owners {
                    if let Some(logged_in) = self.logged_in.as_ref() {
                        let _ = logged_in.ndr_runtime.setup_user(owner);
                    }
                }
                let chat_id = group_chat_id(&result.group.id);
                self.apply_group_snapshot_to_threads(&result.group, now.get());
                created_group_id = Some(result.group.id.clone());
                self.groups.insert(result.group.id.clone(), result.group);
                self.sync_runtime_groups();
                self.active_chat_id = Some(chat_id.clone());
                self.screen_stack = vec![Screen::Chat { chat_id }];
                self.process_runtime_events();
                self.request_protocol_subscription_refresh();
                self.schedule_tracked_peer_catch_up(Duration::from_secs(
                    RESUBSCRIBE_CATCH_UP_DELAY_SECS,
                ));
            }
            Err(error) => self.state.toast = Some(error.to_string()),
        }

        self.state.busy.creating_group = false;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();

        if let (Some(group_id), Some((file_path, filename))) = (created_group_id, picture) {
            self.update_group_picture(&group_id, &file_path, &filename);
        }
    }

    pub(super) fn update_group_name(&mut self, group_id: &str, name: &str) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            self.state.toast = Some("Group name is required.".to_string());
            self.emit_state();
            return;
        }
        let Some(local_owner_hex) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex())
        else {
            return;
        };
        let Some(group) = self.groups.get(group_id).cloned() else {
            self.state.toast = Some("Unknown group.".to_string());
            self.emit_state();
            return;
        };
        let Some(updated) = update_group_data(
            &group,
            &GroupUpdate {
                name: Some(trimmed.to_string()),
                description: None,
                picture: None,
            },
            &local_owner_hex,
        ) else {
            self.state.toast = Some("Only group admins can edit the group.".to_string());
            self.emit_state();
            return;
        };
        self.apply_local_group_update(group_id, updated, Some("group.rename"));
    }

    pub(super) fn update_group_picture(&mut self, group_id: &str, file_path: &str, filename: &str) {
        let Some(local_owner_hex) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex())
        else {
            return;
        };
        let Some(group) = self.groups.get(group_id).cloned() else {
            self.state.toast = Some("Unknown group.".to_string());
            self.emit_state();
            return;
        };
        if !group.admins.iter().any(|admin| admin == &local_owner_hex) {
            self.state.toast = Some("Only group admins can edit the group.".to_string());
            self.emit_state();
            return;
        }
        let path = PathBuf::from(file_path.trim());
        if !path.is_file() {
            self.state.toast = Some("Group photo was not found.".to_string());
            self.emit_state();
            return;
        }
        let Some(secret_hex) = self
            .logged_in
            .as_ref()
            .and_then(|logged_in| {
                logged_in
                    .owner_keys
                    .as_ref()
                    .or(Some(&logged_in.device_keys))
            })
            .map(|keys| keys.secret_key().to_secret_hex())
        else {
            return;
        };
        let filename = display_filename(filename, &path);
        let sender = self.core_sender.clone();
        let upload_group_id = group_id.to_string();
        self.state.busy.uploading_attachment = true;
        self.emit_state();
        self.runtime.spawn(async move {
            let result = upload_file_to_hashtree(&secret_hex, &path)
                .await
                .map(|nhash| format!("htree://{}/{}", nhash, urlencoding::encode(&filename)))
                .map_err(|error| error.to_string());
            let _ = sender.send(CoreMsg::Internal(Box::new(
                InternalEvent::GroupPictureUploadFinished {
                    group_id: upload_group_id,
                    result,
                },
            )));
        });
    }

    pub(super) fn handle_group_picture_upload_finished(
        &mut self,
        group_id: String,
        result: Result<String, String>,
    ) {
        self.state.busy.uploading_attachment = false;
        match result {
            Ok(picture_uri) => {
                self.update_group_picture_uri(&group_id, &picture_uri);
            }
            Err(error) => {
                self.push_debug_log("group.picture.upload.error", error);
                self.state.toast = Some("Group photo upload failed.".to_string());
                self.emit_state();
            }
        }
    }

    fn update_group_picture_uri(&mut self, group_id: &str, picture_uri: &str) {
        let Some(local_owner_hex) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex())
        else {
            return;
        };
        let Some(group) = self.groups.get(group_id).cloned() else {
            return;
        };
        let Some(updated) = update_group_data(
            &group,
            &GroupUpdate {
                name: None,
                description: None,
                picture: Some(picture_uri.to_string()),
            },
            &local_owner_hex,
        ) else {
            self.state.toast = Some("Only group admins can edit the group.".to_string());
            self.emit_state();
            return;
        };
        self.apply_local_group_update(group_id, updated, Some("group.picture"));
    }

    pub(super) fn add_group_members(&mut self, group_id: &str, member_inputs: &[String]) {
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let Some(group) = self.groups.get(group_id).cloned() else {
            self.state.toast = Some("Unknown group.".to_string());
            self.emit_state();
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

        let mut updated = group;
        for owner in &member_owners {
            let Some(next) = add_group_member(&updated, &owner.to_hex(), &local_owner.to_hex())
            else {
                continue;
            };
            updated = next;
            if let Some(logged_in) = self.logged_in.as_ref() {
                let _ = logged_in.ndr_runtime.setup_user(*owner);
            }
        }
        self.apply_local_group_update(group_id, updated, Some("group.add_members"));
    }

    pub(super) fn set_group_admin(
        &mut self,
        group_id: &str,
        owner_pubkey_hex: &str,
        is_admin: bool,
    ) {
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let Some(group) = self.groups.get(group_id).cloned() else {
            self.state.toast = Some("Unknown group.".to_string());
            self.emit_state();
            return;
        };
        let Ok(owner) = parse_owner_input(owner_pubkey_hex) else {
            self.state.toast = Some("Invalid member key.".to_string());
            self.emit_state();
            return;
        };
        let updated = if is_admin {
            add_group_admin(&group, &owner.to_hex(), &local_owner.to_hex())
        } else {
            remove_group_admin(&group, &owner.to_hex(), &local_owner.to_hex())
        };
        let Some(updated) = updated else {
            self.state.toast = Some("Only group admins can manage admins.".to_string());
            self.emit_state();
            return;
        };
        self.apply_local_group_update(
            group_id,
            updated,
            Some(if is_admin {
                "group.add_admin"
            } else {
                "group.remove_admin"
            }),
        );
    }

    pub(super) fn remove_group_member(&mut self, group_id: &str, owner_pubkey_hex: &str) {
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let Some(group) = self.groups.get(group_id).cloned() else {
            self.state.toast = Some("Unknown group.".to_string());
            self.emit_state();
            return;
        };
        let Ok(owner) = parse_owner_input(owner_pubkey_hex) else {
            self.state.toast = Some("Invalid member key.".to_string());
            self.emit_state();
            return;
        };
        let Some(updated) = remove_group_member(&group, &owner.to_hex(), &local_owner.to_hex())
        else {
            self.state.toast = Some("Only group admins can remove members.".to_string());
            self.emit_state();
            return;
        };
        self.apply_local_group_update(group_id, updated, Some("group.remove_member"));
    }

    fn apply_local_group_update(
        &mut self,
        group_id: &str,
        group: GroupData,
        debug_category: Option<&'static str>,
    ) {
        self.state.busy.updating_group = true;
        self.emit_state();

        let previous = self.groups.get(group_id).cloned();
        self.groups.insert(group.id.clone(), group.clone());
        self.apply_group_snapshot_to_threads(&group, unix_now().get());
        self.sync_runtime_groups();
        if let Some(category) = debug_category {
            self.push_debug_log(category, group.id.clone());
        }
        self.fanout_group_metadata(group.clone(), None);
        self.apply_group_metadata_notice(previous.as_ref(), &group);
        self.request_protocol_subscription_refresh();

        self.state.busy.updating_group = false;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn fanout_group_metadata(
        &mut self,
        group: GroupData,
        exclude_secret_for: Option<String>,
    ) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        let result =
            logged_in
                .ndr_runtime
                .with_group_context(|session_manager, group_manager, _| {
                    let mut send_pairwise = |recipient: PublicKey, rumor: &UnsignedEvent| {
                        session_manager
                            .send_event(recipient, rumor.clone())
                            .map(|_| ())
                    };
                    group_manager.fan_out_group_metadata(
                        group,
                        FanoutGroupMetadataOptions {
                            send_pairwise: &mut send_pairwise,
                            exclude_secret_for: exclude_secret_for.as_deref(),
                            now_ms: Some(unix_now().get().saturating_mul(1000)),
                        },
                    )
                });
        if let Err(error) = result {
            self.push_debug_log("group.metadata.fanout.error", error.to_string());
        }
        self.process_runtime_events();
    }

    pub(super) fn apply_group_snapshot_to_threads(
        &mut self,
        group: &GroupData,
        updated_at_secs: u64,
    ) {
        self.ensure_thread_record(&group_chat_id(&group.id), updated_at_secs);
    }

    pub(super) fn apply_group_metadata_rumor(
        &mut self,
        sender_owner: PublicKey,
        event: &UnsignedEvent,
    ) {
        let Some(metadata) = parse_group_metadata(&event.content) else {
            return;
        };
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };

        let local_owner_hex = local_owner.to_hex();
        let sender_hex = sender_owner.to_hex();
        let previous = self.groups.get(&metadata.id).cloned();
        let next = if let Some(existing) = previous.as_ref() {
            match validate_metadata_update(existing, &metadata, &sender_hex, &local_owner_hex) {
                MetadataValidation::Accept => apply_metadata_update(existing, &metadata),
                MetadataValidation::Removed => {
                    self.groups.remove(&metadata.id);
                    self.sync_runtime_groups();
                    self.push_system_notice(
                        &group_chat_id(&metadata.id),
                        "You were removed from the group.".to_string(),
                        event.created_at.as_secs(),
                    );
                    return;
                }
                MetadataValidation::Reject => return,
            }
        } else {
            if !validate_metadata_creation(&metadata, &sender_hex, &local_owner_hex) {
                return;
            }
            GroupData {
                id: metadata.id.clone(),
                name: metadata.name.clone(),
                description: metadata.description.clone(),
                picture: metadata.picture.clone(),
                members: metadata.members.clone(),
                admins: metadata.admins.clone(),
                created_at: event.created_at.as_secs().saturating_mul(1000),
                secret: metadata.secret.clone(),
                accepted: Some(true),
            }
        };

        self.groups.insert(next.id.clone(), next.clone());
        self.apply_group_snapshot_to_threads(&next, event.created_at.as_secs());
        self.sync_runtime_groups();
        self.apply_group_metadata_notice(previous.as_ref(), &next);
    }

    pub(super) fn apply_group_decrypted_event(&mut self, event: GroupDecryptedEvent) {
        // Group session-key advancement counts as session-state churn
        // for the mobile-push snapshot.
        self.mark_mobile_push_dirty();
        let sender_owner = event.sender_owner_pubkey.unwrap_or(event.inner.pubkey);
        let kind = event.inner.kind.as_u16() as u32;
        let created_at_secs = event.inner.created_at.as_secs();
        let expires_at_secs = message_expiration_from_tags(event.inner.tags.iter());
        let chat_id = group_chat_id(&event.group_id);
        let message_id = event
            .inner
            .id
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| Some(event.outer_event_id.clone()));
        let is_outgoing = self
            .logged_in
            .as_ref()
            .map(|logged_in| sender_owner == logged_in.owner_pubkey)
            .unwrap_or(false);

        match kind {
            GROUP_METADATA_KIND => self.apply_group_metadata_rumor(sender_owner, &event.inner),
            CHAT_MESSAGE_KIND => {
                self.apply_runtime_text_message(
                    sender_owner,
                    Some(chat_id.clone()),
                    event.inner.content.clone(),
                    created_at_secs,
                    expires_at_secs,
                    message_id.clone(),
                    Some(event.outer_event_id.clone()),
                );
                if !is_outgoing && self.preferences.send_read_receipts {
                    if let Some(message_id) = message_id {
                        self.send_group_receipt(&chat_id, "delivered", vec![message_id]);
                    }
                }
            }
            REACTION_KIND => {
                let sender_hex = sender_owner.to_hex();
                for message_id in event_message_ids(&event.inner) {
                    self.apply_incoming_reaction_to_chat(
                        &chat_id,
                        &message_id,
                        &sender_hex,
                        &event.inner.content,
                    );
                }
            }
            RECEIPT_KIND => {
                let delivery = match event.inner.content.as_str() {
                    "seen" => DeliveryState::Seen,
                    _ => DeliveryState::Received,
                };
                self.apply_receipt_to_messages(
                    &chat_id,
                    &event_message_ids(&event.inner),
                    delivery,
                    is_outgoing,
                );
            }
            TYPING_KIND => {
                if !is_outgoing {
                    self.apply_typing_event(
                        chat_id,
                        sender_owner.to_hex(),
                        created_at_secs,
                        expires_at_secs,
                    );
                }
            }
            CHAT_SETTINGS_KIND => {
                let actor = self.owner_display_label(&sender_owner.to_hex());
                self.apply_chat_settings_control(
                    &chat_id,
                    &actor,
                    chat_settings_ttl_seconds(&event.inner.content),
                    created_at_secs,
                );
            }
            _ => {}
        }
    }

    fn send_group_receipt(&mut self, chat_id: &str, receipt_type: &str, message_ids: Vec<String>) {
        let tags = message_ids
            .into_iter()
            .map(|id| vec!["e".to_string(), id])
            .collect();
        self.send_group_event(chat_id, RECEIPT_KIND, receipt_type, tags, None);
    }

    pub(super) fn apply_group_metadata_notice(
        &mut self,
        previous: Option<&GroupData>,
        group: &GroupData,
    ) {
        let chat_id = group_chat_id(&group.id);
        let now = unix_now().get();
        match previous {
            None => {
                self.push_system_notice(&chat_id, format!("Group created: {}", group.name), now);
            }
            Some(previous) => {
                if previous.name != group.name {
                    self.push_system_notice(
                        &chat_id,
                        format!("Group renamed to {}", group.name),
                        now,
                    );
                }
                if previous.picture != group.picture {
                    self.push_system_notice(&chat_id, "Group photo changed".to_string(), now);
                }
                for owner in group
                    .members
                    .iter()
                    .filter(|owner| !previous.members.iter().any(|existing| existing == *owner))
                {
                    self.push_system_notice(
                        &chat_id,
                        format!("{} joined the group", self.owner_display_label(owner)),
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
                        format!("{} left the group", self.owner_display_label(owner)),
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

    pub(super) fn sync_runtime_groups(&mut self) {
        if let Some(logged_in) = self.logged_in.as_ref() {
            if let Err(error) = logged_in
                .ndr_runtime
                .sync_groups(self.groups.values().cloned().collect())
            {
                self.push_debug_log("group.sync.error", error.to_string());
            }
        }
    }
    fn admin_change_notice(&self, previous: &GroupData, group: &GroupData) -> String {
        let added = group
            .admins
            .iter()
            .find(|admin| !previous.admins.iter().any(|existing| existing == *admin));
        if let Some(owner) = added {
            return format!("{} became an admin", self.owner_display_label(owner));
        }
        let removed = previous
            .admins
            .iter()
            .find(|admin| !group.admins.iter().any(|existing| existing == *admin));
        if let Some(owner) = removed {
            return format!("{} is no longer an admin", self.owner_display_label(owner));
        }
        "Group admins changed".to_string()
    }
}
