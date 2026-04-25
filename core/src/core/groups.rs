use super::*;

impl AppCore {
    pub(super) fn create_group(&mut self, name: &str, member_inputs: &[String]) {
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
            self.state.toast = Some("Create or restore an account first.".to_string());
            self.emit_state();
            return;
        };

        let member_owners = match parse_owner_inputs(member_inputs, local_owner) {
            Ok(member_owners) if !member_owners.is_empty() => member_owners,
            Ok(_) => {
                self.state.toast = Some("Groups need at least one other member.".to_string());
                self.emit_state();
                return;
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
                self.emit_state();
                return;
            }
        };
        let target_owner_hexes = sorted_owner_hexes(&member_owners);

        self.state.busy.creating_group = true;
        self.emit_state();

        let now = unix_now();
        let create_result = {
            let logged_in = self.logged_in.as_mut().expect("checked above");
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(now, &mut rng);
            let (session_manager, group_manager) =
                (&mut logged_in.session_manager, &mut logged_in.group_manager);
            group_manager.create_group(
                session_manager,
                &mut ctx,
                trimmed_name.to_string(),
                member_owners,
            )
        };

        match create_result {
            Ok(result) => {
                let create_kind = PendingGroupControlKind::Create {
                    name: trimmed_name.to_string(),
                    member_owner_hexes: target_owner_hexes.clone(),
                };
                let chat_id = group_chat_id(&result.group.group_id);
                self.apply_group_snapshot_to_threads(&result.group, now.get());
                self.active_chat_id = Some(chat_id.clone());
                self.screen_stack = vec![Screen::Chat {
                    chat_id: chat_id.clone(),
                }];
                self.publish_group_local_sibling_best_effort(&result.prepared);

                if let Some(reason) = pending_reason_from_group_prepared(&result.prepared) {
                    let operation_id = self.allocate_message_id();
                    self.queue_pending_group_control(
                        operation_id,
                        result.group.group_id.clone(),
                        target_owner_hexes,
                        None,
                        reason.clone(),
                        now.get().saturating_add(PENDING_RETRY_DELAY_SECS),
                        create_kind,
                    );
                    self.nudge_protocol_state_for_pending_reason(&reason);
                } else {
                    match build_group_prepared_publish_batch(&result.prepared, None) {
                        Ok(Some(batch)) => {
                            let operation_id = self.allocate_message_id();
                            let publish_mode = publish_mode_for_batch(&batch);
                            self.queue_pending_group_control(
                                operation_id.clone(),
                                result.group.group_id.clone(),
                                target_owner_hexes,
                                Some(batch.clone()),
                                pending_reason_for_publish_mode(&publish_mode),
                                retry_deadline_for_publish_mode(now.get(), &publish_mode),
                                create_kind.clone(),
                            );
                            self.set_pending_group_control_in_flight(&operation_id, true);
                            self.start_group_control_publish(operation_id, publish_mode, batch);
                        }
                        Ok(None) => {}
                        Err(error) => self.state.toast = Some(error.to_string()),
                    }
                }

                self.request_protocol_subscription_refresh();
                self.schedule_tracked_peer_catch_up(Duration::from_secs(
                    RESUBSCRIBE_CATCH_UP_DELAY_SECS,
                ));
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
            }
        }

        self.schedule_next_pending_retry(now.get());
        self.state.busy.creating_group = false;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn prepare_group_control(
        &mut self,
        group_id: &str,
        kind: &PendingGroupControlKind,
        now: UnixSeconds,
    ) -> anyhow::Result<(
        GroupSnapshot,
        Vec<String>,
        nostr_double_ratchet::GroupPreparedSend,
    )> {
        let logged_in = self.logged_in.as_mut().expect("logged in checked above");
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        let (session_manager, group_manager) =
            (&mut logged_in.session_manager, &mut logged_in.group_manager);

        match kind {
            PendingGroupControlKind::Create {
                name,
                member_owner_hexes,
            } => {
                let members = owner_pubkeys_from_hexes(member_owner_hexes)?;
                let result =
                    group_manager.create_group(session_manager, &mut ctx, name.clone(), members)?;
                return Ok((
                    result.group.clone(),
                    member_owner_hexes.clone(),
                    result.prepared,
                ));
            }
            PendingGroupControlKind::Rename { name } => {
                let prepared =
                    group_manager.update_name(session_manager, &mut ctx, group_id, name.clone())?;
                let snapshot = group_manager
                    .group(group_id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown group."))?;
                return Ok((
                    snapshot.clone(),
                    sorted_owner_hexes(
                        &snapshot
                            .members
                            .iter()
                            .copied()
                            .filter(|member| *member != logged_in.owner_pubkey)
                            .collect::<Vec<_>>(),
                    ),
                    prepared,
                ));
            }
            PendingGroupControlKind::AddMembers { member_owner_hexes } => {
                let members = owner_pubkeys_from_hexes(member_owner_hexes)?;
                let prepared =
                    group_manager.add_members(session_manager, &mut ctx, group_id, members)?;
                let snapshot = group_manager
                    .group(group_id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown group."))?;
                return Ok((
                    snapshot.clone(),
                    sorted_owner_hexes(
                        &snapshot
                            .members
                            .iter()
                            .copied()
                            .filter(|member| *member != logged_in.owner_pubkey)
                            .collect::<Vec<_>>(),
                    ),
                    prepared,
                ));
            }
            PendingGroupControlKind::RemoveMember { owner_pubkey_hex } => {
                let owner = parse_owner_input(owner_pubkey_hex)?;
                let prepared = group_manager.remove_members(
                    session_manager,
                    &mut ctx,
                    group_id,
                    vec![owner],
                )?;
                let snapshot = group_manager
                    .group(group_id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown group."))?;
                return Ok((
                    snapshot.clone(),
                    sorted_owner_hexes(
                        &snapshot
                            .members
                            .iter()
                            .copied()
                            .filter(|member| *member != logged_in.owner_pubkey)
                            .collect::<Vec<_>>(),
                    ),
                    prepared,
                ));
            }
        }
    }

    pub(super) fn rebuild_group_control(
        &mut self,
        group_id: &str,
        kind: &PendingGroupControlKind,
        now: UnixSeconds,
    ) -> anyhow::Result<(
        GroupSnapshot,
        Vec<String>,
        nostr_double_ratchet::GroupPreparedSend,
    )> {
        let logged_in = self.logged_in.as_mut().expect("logged in checked above");
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        let (session_manager, group_manager) =
            (&mut logged_in.session_manager, &mut logged_in.group_manager);

        match kind {
            PendingGroupControlKind::Create {
                member_owner_hexes, ..
            } => {
                let members = owner_pubkeys_from_hexes(member_owner_hexes)?;
                let prepared = group_manager.retry_create_group(
                    session_manager,
                    &mut ctx,
                    group_id,
                    members,
                )?;
                let snapshot = group_manager
                    .group(group_id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown group."))?;
                Ok((snapshot, member_owner_hexes.clone(), prepared))
            }
            PendingGroupControlKind::Rename { .. } => {
                let prepared =
                    group_manager.retry_update_name(session_manager, &mut ctx, group_id)?;
                let snapshot = group_manager
                    .group(group_id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown group."))?;
                let target_owner_hexes = sorted_owner_hexes(
                    &snapshot
                        .members
                        .iter()
                        .copied()
                        .filter(|member| *member != logged_in.owner_pubkey)
                        .collect::<Vec<_>>(),
                );
                Ok((snapshot, target_owner_hexes, prepared))
            }
            PendingGroupControlKind::AddMembers { member_owner_hexes } => {
                let members = owner_pubkeys_from_hexes(member_owner_hexes)?;
                let prepared = group_manager.retry_add_members(
                    session_manager,
                    &mut ctx,
                    group_id,
                    members,
                )?;
                let snapshot = group_manager
                    .group(group_id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown group."))?;
                let target_owner_hexes = sorted_owner_hexes(
                    &snapshot
                        .members
                        .iter()
                        .copied()
                        .filter(|member| *member != logged_in.owner_pubkey)
                        .collect::<Vec<_>>(),
                );
                Ok((snapshot, target_owner_hexes, prepared))
            }
            PendingGroupControlKind::RemoveMember { owner_pubkey_hex } => {
                let owner = parse_owner_input(owner_pubkey_hex)?;
                let prepared = group_manager.retry_remove_members(
                    session_manager,
                    &mut ctx,
                    group_id,
                    vec![owner],
                )?;
                let snapshot = group_manager
                    .group(group_id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown group."))?;
                let mut targets = snapshot
                    .members
                    .iter()
                    .copied()
                    .filter(|member| *member != logged_in.owner_pubkey)
                    .map(|member| member.to_string())
                    .collect::<HashSet<_>>();
                if owner != logged_in.owner_pubkey {
                    targets.insert(owner.to_string());
                }
                Ok((snapshot, sorted_hexes(targets), prepared))
            }
        }
    }

    pub(super) fn apply_group_snapshot_to_threads(
        &mut self,
        group: &GroupSnapshot,
        updated_at_secs: u64,
    ) {
        let chat_id = group_chat_id(&group.group_id);
        let thread = self.ensure_thread_record(&chat_id, updated_at_secs);
        thread.updated_at_secs = thread.updated_at_secs.max(updated_at_secs);
    }

    pub(super) fn apply_group_snapshot_to_threads_with_notices(
        &mut self,
        previous: Option<&GroupSnapshot>,
        group: &GroupSnapshot,
        updated_at_secs: u64,
    ) {
        self.apply_group_snapshot_to_threads(group, updated_at_secs);
        let chat_id = group_chat_id(&group.group_id);
        for notice in self.group_metadata_notices(previous, group) {
            self.push_system_notice(&chat_id, notice, updated_at_secs);
        }
    }

    pub(super) fn group_metadata_notices(
        &self,
        previous: Option<&GroupSnapshot>,
        group: &GroupSnapshot,
    ) -> Vec<String> {
        let Some(previous) = previous else {
            return Vec::new();
        };
        let mut notices = Vec::new();
        if previous.name != group.name {
            notices.push(format!("Group renamed to {}", group.name));
        }

        let previous_members = previous.members.iter().copied().collect::<HashSet<_>>();
        let current_members = group.members.iter().copied().collect::<HashSet<_>>();
        let mut added = current_members
            .difference(&previous_members)
            .copied()
            .collect::<Vec<_>>();
        let mut removed = previous_members
            .difference(&current_members)
            .copied()
            .collect::<Vec<_>>();
        added.sort_by_key(|owner| owner.to_string());
        removed.sort_by_key(|owner| owner.to_string());

        if !added.is_empty() {
            notices.push(format!(
                "{} added",
                self.group_notice_owner_list(added.as_slice())
            ));
        }
        if !removed.is_empty() {
            notices.push(format!(
                "{} removed",
                self.group_notice_owner_list(removed.as_slice())
            ));
        }
        notices
    }

    fn group_notice_owner_list(&self, owners: &[OwnerPubkey]) -> String {
        match owners {
            [] => String::new(),
            [owner] => self.owner_display_label(&owner.to_string()),
            owners => format!("{} members", owners.len()),
        }
    }

    pub(super) fn queue_pending_group_control(
        &mut self,
        operation_id: String,
        group_id: String,
        target_owner_hexes: Vec<String>,
        prepared_publish: Option<PreparedPublishBatch>,
        reason: PendingSendReason,
        next_retry_at_secs: u64,
        kind: PendingGroupControlKind,
    ) {
        self.pending_group_controls.push(PendingGroupControl {
            operation_id,
            group_id,
            target_owner_hexes,
            prepared_publish,
            reason,
            next_retry_at_secs,
            in_flight: false,
            kind,
        });
    }

    pub(super) fn set_pending_group_control_in_flight(
        &mut self,
        operation_id: &str,
        in_flight: bool,
    ) {
        if let Some(pending) = self
            .pending_group_controls
            .iter_mut()
            .find(|pending| pending.operation_id == operation_id)
        {
            pending.in_flight = in_flight;
        }
    }

    pub(super) fn start_group_control_publish(
        &mut self,
        operation_id: String,
        publish_mode: OutboundPublishMode,
        batch: PreparedPublishBatch,
    ) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };

        for event in batch
            .invite_events
            .iter()
            .chain(batch.message_events.iter())
        {
            self.remember_event(event.id.to_string());
        }

        let tx = self.core_sender.clone();
        match publish_mode {
            OutboundPublishMode::OrdinaryFirstAck => {
                self.runtime.spawn(async move {
                    let success = publish_events_first_ack(
                        &client,
                        &relay_urls,
                        &batch.message_events,
                        "group control",
                    )
                    .await
                    .is_ok();
                    let _ = tx.send(CoreMsg::Internal(Box::new(
                        InternalEvent::GroupControlPublishFinished {
                            operation_id,
                            success,
                        },
                    )));
                });
            }
            OutboundPublishMode::FirstContactStaged => {
                self.runtime.spawn(async move {
                    let invite_publish = publish_events_with_retry(
                        &client,
                        &relay_urls,
                        batch.invite_events,
                        "group control",
                    )
                    .await;
                    if invite_publish.is_err() {
                        let _ = tx.send(CoreMsg::Internal(Box::new(
                            InternalEvent::GroupControlPublishFinished {
                                operation_id,
                                success: false,
                            },
                        )));
                        return;
                    }

                    sleep(Duration::from_millis(FIRST_CONTACT_STAGE_DELAY_MS)).await;
                    let success = publish_events_with_retry(
                        &client,
                        &relay_urls,
                        batch.message_events,
                        "group control",
                    )
                    .await
                    .is_ok();
                    let _ = tx.send(CoreMsg::Internal(Box::new(
                        InternalEvent::GroupControlPublishFinished {
                            operation_id,
                            success,
                        },
                    )));
                });
            }
            OutboundPublishMode::WaitForPeer => {}
        }
    }

    pub(super) fn start_best_effort_publish(
        &mut self,
        label: &'static str,
        batch: PreparedPublishBatch,
    ) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        if batch.message_events.is_empty() {
            return;
        }

        for event in batch
            .invite_events
            .iter()
            .chain(batch.message_events.iter())
        {
            self.remember_event(event.id.to_string());
        }

        let tx = self.core_sender.clone();
        match publish_mode_for_batch(&batch) {
            OutboundPublishMode::OrdinaryFirstAck => {
                self.runtime.spawn(async move {
                    let success = publish_events_first_ack(
                        &client,
                        &relay_urls,
                        &batch.message_events,
                        label,
                    )
                    .await
                    .is_ok();
                    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                        category: "publish.best_effort".to_string(),
                        detail: format!("label={label} success={success}"),
                    })));
                });
            }
            OutboundPublishMode::FirstContactStaged => {
                self.runtime.spawn(async move {
                    let success = if publish_events_with_retry(
                        &client,
                        &relay_urls,
                        batch.invite_events,
                        label,
                    )
                    .await
                    .is_ok()
                    {
                        sleep(Duration::from_millis(FIRST_CONTACT_STAGE_DELAY_MS)).await;
                        publish_events_with_retry(&client, &relay_urls, batch.message_events, label)
                            .await
                            .is_ok()
                    } else {
                        false
                    };
                    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                        category: "publish.best_effort".to_string(),
                        detail: format!("label={label} success={success}"),
                    })));
                });
            }
            OutboundPublishMode::WaitForPeer => {}
        }
    }

    pub(super) fn publish_group_local_sibling_best_effort(
        &mut self,
        prepared: &nostr_double_ratchet::GroupPreparedSend,
    ) {
        match build_group_local_sibling_publish_batch(prepared) {
            Ok(Some(batch)) => self.start_best_effort_publish("group sibling sync", batch),
            Ok(None) => {}
            Err(error) => self.state.toast = Some(error.to_string()),
        }
    }

    pub(super) fn update_group_name(&mut self, group_id: &str, name: &str) {
        self.run_group_control(
            group_id,
            PendingGroupControlKind::Rename {
                name: name.trim().to_string(),
            },
        );
    }

    pub(super) fn add_group_members(&mut self, group_id: &str, member_inputs: &[String]) {
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            self.state.toast = Some("Create or restore an account first.".to_string());
            self.emit_state();
            return;
        };
        let member_owners = match parse_owner_inputs(member_inputs, local_owner) {
            Ok(member_owners) if !member_owners.is_empty() => member_owners,
            Ok(_) => {
                self.state.toast = Some("Pick at least one member to add.".to_string());
                self.emit_state();
                return;
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
                self.emit_state();
                return;
            }
        };
        self.run_group_control(
            group_id,
            PendingGroupControlKind::AddMembers {
                member_owner_hexes: sorted_owner_hexes(&member_owners),
            },
        );
    }

    pub(super) fn remove_group_member(&mut self, group_id: &str, owner_pubkey_hex: &str) {
        let Ok((owner_pubkey_hex, _)) = parse_peer_input(owner_pubkey_hex) else {
            self.state.toast = Some("Invalid member key.".to_string());
            self.emit_state();
            return;
        };
        self.run_group_control(
            group_id,
            PendingGroupControlKind::RemoveMember { owner_pubkey_hex },
        );
    }

    pub(super) fn run_group_control(&mut self, group_id: &str, kind: PendingGroupControlKind) {
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

        let Some(group_id) = normalize_group_id(group_id) else {
            self.state.toast = Some("Unknown group.".to_string());
            self.emit_state();
            return;
        };
        self.state.busy.updating_group = true;
        self.emit_state();

        let now = unix_now();
        let previous_group = self
            .logged_in
            .as_ref()
            .and_then(|logged_in| logged_in.group_manager.group(&group_id));
        let control_result = self.prepare_group_control(&group_id, &kind, now);
        match control_result {
            Ok((snapshot, target_owner_hexes, prepared)) => {
                self.apply_group_snapshot_to_threads_with_notices(
                    previous_group.as_ref(),
                    &snapshot,
                    now.get(),
                );
                self.publish_group_local_sibling_best_effort(&prepared);
                if let Some(reason) = pending_reason_from_group_prepared(&prepared) {
                    let operation_id = self.allocate_message_id();
                    self.queue_pending_group_control(
                        operation_id,
                        group_id,
                        target_owner_hexes,
                        None,
                        reason.clone(),
                        now.get().saturating_add(PENDING_RETRY_DELAY_SECS),
                        kind,
                    );
                    self.nudge_protocol_state_for_pending_reason(&reason);
                } else {
                    match build_group_prepared_publish_batch(&prepared, None) {
                        Ok(Some(batch)) => {
                            let operation_id = self.allocate_message_id();
                            let publish_mode = publish_mode_for_batch(&batch);
                            self.queue_pending_group_control(
                                operation_id.clone(),
                                group_id.clone(),
                                target_owner_hexes,
                                Some(batch.clone()),
                                pending_reason_for_publish_mode(&publish_mode),
                                retry_deadline_for_publish_mode(now.get(), &publish_mode),
                                kind.clone(),
                            );
                            self.set_pending_group_control_in_flight(&operation_id, true);
                            self.start_group_control_publish(operation_id, publish_mode, batch);
                        }
                        Ok(None) => {}
                        Err(error) => self.state.toast = Some(error.to_string()),
                    }
                }

                self.request_protocol_subscription_refresh();
                self.schedule_tracked_peer_catch_up(Duration::from_secs(
                    RESUBSCRIBE_CATCH_UP_DELAY_SECS,
                ));
            }
            Err(error) => self.state.toast = Some(error.to_string()),
        }

        self.schedule_next_pending_retry(now.get());
        self.state.busy.updating_group = false;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn retry_pending_group_controls(&mut self, now: UnixSeconds) {
        if self.logged_in.is_none() || self.pending_group_controls.is_empty() {
            return;
        }

        let pending = std::mem::take(&mut self.pending_group_controls);
        let mut still_pending = Vec::new();

        for mut control in pending {
            if control.next_retry_at_secs > now.get() || control.in_flight {
                still_pending.push(control);
                continue;
            }

            if let Some(batch) = control.prepared_publish.clone() {
                control.in_flight = true;
                let publish_mode = publish_mode_for_batch(&batch);
                self.start_group_control_publish(control.operation_id.clone(), publish_mode, batch);
                still_pending.push(control);
                continue;
            }

            match self.rebuild_group_control(&control.group_id, &control.kind, now) {
                Ok((snapshot, target_owner_hexes, prepared)) => {
                    self.apply_group_snapshot_to_threads(&snapshot, now.get());
                    control.target_owner_hexes = target_owner_hexes;
                    self.publish_group_local_sibling_best_effort(&prepared);
                    if let Some(reason) = pending_reason_from_group_prepared(&prepared) {
                        self.push_debug_log(
                            "retry.group_control.pending",
                            format!(
                                "group_id={} reason={reason:?} gaps={}",
                                control.group_id,
                                summarize_relay_gaps(&prepared.remote.relay_gaps)
                            ),
                        );
                        control.reason = reason.clone();
                        control.next_retry_at_secs =
                            now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                        self.nudge_protocol_state_for_pending_reason(&reason);
                        still_pending.push(control);
                    } else {
                        match build_group_prepared_publish_batch(&prepared, None) {
                            Ok(Some(batch)) => {
                                control.prepared_publish = Some(batch.clone());
                                control.reason = pending_reason_for_publish_mode(
                                    &publish_mode_for_batch(&batch),
                                );
                                control.next_retry_at_secs = retry_deadline_for_publish_mode(
                                    now.get(),
                                    &publish_mode_for_batch(&batch),
                                );
                                control.in_flight = true;
                                self.start_group_control_publish(
                                    control.operation_id.clone(),
                                    publish_mode_for_batch(&batch),
                                    batch,
                                );
                                still_pending.push(control);
                            }
                            Ok(None) => {
                                control.next_retry_at_secs =
                                    now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                                self.nudge_protocol_state_for_pending_reason(
                                    &PendingSendReason::MissingDeviceInvite,
                                );
                                still_pending.push(control);
                            }
                            Err(error) => self.state.toast = Some(error.to_string()),
                        }
                    }
                }
                Err(error) => self.state.toast = Some(error.to_string()),
            }
        }

        self.pending_group_controls = still_pending;
        self.schedule_next_pending_retry(now.get());
    }
}
