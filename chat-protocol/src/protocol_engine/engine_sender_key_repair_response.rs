use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepairResponseThrottleKey {
    requester_owner_hex: String,
    requester_device_hex: Option<String>,
    group_id: String,
    sender_event_pubkey_hex: String,
    key_id: Option<u32>,
    message_number: Option<u32>,
    required_revision: Option<u64>,
}

impl ProtocolEngine {
    pub(super) fn sender_key_repair_response_effects(
        &mut self,
        requester_owner: NdrOwnerPubkey,
        request: &SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        self.sender_key_repair_response_effects_for_device(requester_owner, None, request, now)
    }

    pub(super) fn sender_key_repair_response_effects_for_device(
        &mut self,
        requester_owner: NdrOwnerPubkey,
        requester_device: Option<NdrDevicePubkey>,
        request: &SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        let Some(group) = self.group_manager.group(&request.group_id) else {
            return Ok(Vec::new());
        };
        if !group.protocol.is_sender_key_v1() || !group.members.contains(&requester_owner) {
            return Ok(Vec::new());
        }

        let is_local_sibling_requester = requester_owner == self.local_owner;
        // Remote receivers only accept MetadataSnapshot from group admins or
        // their own linked devices. Sending a non-admin snapshot wastes wire
        // bandwidth and queues an unapplicable payload on the requester.
        let can_supply_metadata =
            is_local_sibling_requester || group.admins.contains(&self.local_owner);

        let local_sender_record =
            self.group_manager
                .snapshot()
                .sender_keys
                .into_iter()
                .find(|record| {
                    record.group_id == request.group_id
                        && record.sender_owner == self.local_owner
                        && record.sender_event_pubkey == request.sender_event_pubkey
                        && record.sender_event_secret_key.is_some()
                });

        let mut distributions = Vec::new();
        let mut latest_key_id = None;
        let mut has_initial_snapshot = false;
        if let Some(record) = local_sender_record.as_ref() {
            latest_key_id = record.latest_key_id;
            distributions = match (request.key_id, request.message_number) {
                (Some(key_id), Some(message_number)) => {
                    repair_distribution_for(record, requester_owner, key_id, message_number)
                        .into_iter()
                        .collect()
                }
                _ => repair_distributions_for_requester(record, requester_owner),
            };
            has_initial_snapshot = distributions.iter().any(|distribution| {
                requester_received_initial_repair_snapshot(record, requester_owner, distribution)
            });
        }

        let include_required_revision_metadata = can_supply_metadata
            && request
                .required_revision
                .is_some_and(|required| group.revision >= required);
        let include_context_metadata = can_supply_metadata
            && request.required_revision.is_none()
            && !is_local_sibling_requester
            && has_initial_snapshot;

        if distributions.is_empty() && !include_required_revision_metadata {
            return Ok(Vec::new());
        }

        let revision_only = distributions.is_empty() && include_required_revision_metadata;
        let throttle_key = Self::repair_response_throttle_key(
            requester_owner,
            requester_device,
            request,
            revision_only,
        );
        let metadata_eligible = include_required_revision_metadata || include_context_metadata;
        let has_new_material = self.repair_response_has_new_material_for_key(
            &throttle_key,
            revision_only,
            metadata_eligible,
            group.revision,
            latest_key_id,
        );
        if !has_new_material && self.repair_response_throttled_for_key(&throttle_key, now) {
            return Ok(Vec::new());
        }
        let prior_response_count = if has_new_material {
            0
        } else {
            self.repair_response_count_for_key(&throttle_key)
        };

        // On the first blind key-only response, send the latest eligible chain
        // first: nearly all repairs are for the current or newly rotated epoch.
        // If the requester still retries, subsequent responses include all
        // remaining eligible historical chains (oldest first) so older epochs
        // still recover cleanly.
        if request.key_id.is_none() && distributions.len() > 1 && prior_response_count == 0 {
            if let Some(latest) = latest_key_id
                .and_then(|kid| {
                    distributions
                        .iter()
                        .find(|dist| dist.key_id == kid)
                        .cloned()
                })
                .or_else(|| {
                    distributions
                        .iter()
                        .max_by_key(|dist| dist.created_at)
                        .cloned()
                })
            {
                distributions = vec![latest];
            }
        }

        let send_metadata = include_required_revision_metadata
            || (include_context_metadata && prior_response_count < 2);

        let mut prepared = GroupPreparedSend {
            group_id: request.group_id.clone(),
            remote: GroupPreparedPublish::empty(),
            local_sibling: GroupPreparedPublish::empty(),
        };
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);

        for distribution in distributions {
            let part = self.prepare_repair_payload_to_requester(
                &mut ctx,
                requester_owner,
                requester_device,
                &GroupPairwiseCommand::SenderKeyDistribution { distribution },
            )?;
            merge_group_publish(&mut prepared.remote, part.remote);
            merge_group_publish(&mut prepared.local_sibling, part.local_sibling);
        }
        if send_metadata {
            let part = self.prepare_repair_payload_to_requester(
                &mut ctx,
                requester_owner,
                requester_device,
                &GroupPairwiseCommand::MetadataSnapshot {
                    snapshot: group.clone(),
                },
            )?;
            merge_group_publish(&mut prepared.remote, part.remote);
            merge_group_publish(&mut prepared.local_sibling, part.local_sibling);
        }

        let response_deferred = !prepared.remote.pending_fanouts.is_empty()
            || !prepared.local_sibling.pending_fanouts.is_empty();
        let output = self.protocol_group_send_from_prepared(&prepared, None)?;
        if !output.effects.is_empty() || response_deferred {
            self.remember_group_sender_key_repair_response_for_key(
                throttle_key,
                request.created_at.get(),
                now,
                revision_only,
                has_new_material,
                group.revision,
                latest_key_id,
            );
        }
        self.invalidate_known_message_author_cache();
        Ok(output.effects)
    }

    pub(super) fn prepare_group_repair_request_send(
        &mut self,
        request: &SenderKeyRepairRequest,
        request_count: u32,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<GroupPreparedSend> {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);

        // Missing-revision repairs can only be satisfied by a current admin or
        // a local sibling device. Target those directly on the initial attempt
        // and steady-state retries, keeping a periodic fallback pass (retry #1,
        // #5, ...) in case a newly promoted admin is only in the full roster.
        if request.required_revision.is_some() && request_count % 4 != 1 {
            if let Some(group) = self.group_manager.group(&request.group_id) {
                if !group.protocol.is_sender_key_v1() || !group.members.contains(&self.local_owner)
                {
                    return Ok(GroupPreparedSend {
                        group_id: request.group_id.clone(),
                        remote: GroupPreparedPublish::empty(),
                        local_sibling: GroupPreparedPublish::empty(),
                    });
                }
                let remote_admins = group
                    .admins
                    .iter()
                    .copied()
                    .filter(|admin| *admin != self.local_owner && group.members.contains(admin))
                    .collect::<Vec<_>>();
                let has_siblings = self.has_authorized_local_siblings();
                if !remote_admins.is_empty() || has_siblings {
                    let payload = JsonGroupPayloadCodecV1.encode_pairwise_command(
                        nostr_double_ratchet::GroupPayloadEncodeContext {
                            local_device_pubkey: self.local_device,
                            created_at: now,
                        },
                        &GroupPairwiseCommand::SenderKeyRepairRequest {
                            request: request.clone(),
                        },
                    )?;
                    let mut remote = GroupPreparedPublish::empty();
                    for admin in remote_admins {
                        let next = self.session_manager.prepare_remote_send(
                            &mut ctx,
                            admin,
                            payload.clone(),
                        )?;
                        remote.deliveries.extend(next.deliveries);
                        remote.invite_responses.extend(next.invite_responses);
                        if !next.relay_gaps.is_empty() {
                            remote.pending_fanouts.push(GroupPendingFanout::Remote {
                                recipient_owner: admin,
                                payload: payload.clone(),
                            });
                        }
                        remote.relay_gaps.extend(next.relay_gaps);
                    }
                    remote.relay_gaps.sort();
                    remote.relay_gaps.dedup();
                    let local_sibling = if has_siblings {
                        let next = self
                            .session_manager
                            .prepare_local_sibling_send(&mut ctx, payload.clone())?;
                        let pending_fanouts = if next.relay_gaps.is_empty() {
                            Vec::new()
                        } else {
                            vec![GroupPendingFanout::LocalSiblings { payload }]
                        };
                        GroupPreparedPublish {
                            deliveries: next.deliveries,
                            invite_responses: next.invite_responses,
                            sender_key_messages: Vec::new(),
                            relay_gaps: next.relay_gaps,
                            pending_fanouts,
                        }
                    } else {
                        GroupPreparedPublish::empty()
                    };
                    return Ok(GroupPreparedSend {
                        group_id: request.group_id.clone(),
                        remote,
                        local_sibling,
                    });
                }
            }
        }

        Ok(self.group_manager.request_sender_key_repair(
            &mut self.session_manager,
            &mut ctx,
            request,
        )?)
    }

    fn prepare_repair_payload_to_requester(
        &mut self,
        ctx: &mut ProtocolContext<'_, OsRng>,
        requester_owner: NdrOwnerPubkey,
        requester_device: Option<NdrDevicePubkey>,
        command: &GroupPairwiseCommand,
    ) -> anyhow::Result<GroupPreparedSend> {
        let group_id = match command {
            GroupPairwiseCommand::MetadataSnapshot { snapshot } => snapshot.group_id.clone(),
            GroupPairwiseCommand::GroupMessage { group_id, .. }
            | GroupPairwiseCommand::SenderKeyDistribution {
                distribution: nostr_double_ratchet::SenderKeyDistribution { group_id, .. },
            }
            | GroupPairwiseCommand::SenderKeyRepairRequest {
                request: SenderKeyRepairRequest { group_id, .. },
            } => group_id.clone(),
        };
        let payload = JsonGroupPayloadCodecV1.encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: self.local_device,
                created_at: ctx.now,
            },
            command,
        )?;
        let targeted_device = requester_device.filter(|device| {
            self.session_manager
                .roster(requester_owner)
                .is_some_and(|roster| roster.get_device(device).is_some())
        });

        if requester_owner == self.local_owner {
            if requester_device == Some(self.local_device) {
                return Ok(GroupPreparedSend {
                    group_id,
                    remote: GroupPreparedPublish::empty(),
                    local_sibling: GroupPreparedPublish::empty(),
                });
            }
            let next = match targeted_device {
                Some(device) => self.session_manager.prepare_local_sibling_send_to_devices(
                    ctx,
                    [device],
                    payload.clone(),
                )?,
                None => self
                    .session_manager
                    .prepare_local_sibling_send(ctx, payload.clone())?,
            };
            let pending_fanouts = if next.relay_gaps.is_empty() {
                Vec::new()
            } else {
                vec![GroupPendingFanout::LocalSiblings { payload }]
            };
            return Ok(GroupPreparedSend {
                group_id,
                remote: GroupPreparedPublish::empty(),
                local_sibling: GroupPreparedPublish {
                    deliveries: next.deliveries,
                    invite_responses: next.invite_responses,
                    sender_key_messages: Vec::new(),
                    relay_gaps: next.relay_gaps,
                    pending_fanouts,
                },
            });
        }

        let next = match targeted_device {
            Some(device) => self.session_manager.prepare_remote_send_to_devices(
                ctx,
                requester_owner,
                [device],
                payload.clone(),
            )?,
            None => {
                self.session_manager
                    .prepare_remote_send(ctx, requester_owner, payload.clone())?
            }
        };
        let pending_fanouts = if next.relay_gaps.is_empty() {
            Vec::new()
        } else {
            vec![GroupPendingFanout::Remote {
                recipient_owner: requester_owner,
                payload,
            }]
        };
        Ok(GroupPreparedSend {
            group_id,
            remote: GroupPreparedPublish {
                deliveries: next.deliveries,
                invite_responses: next.invite_responses,
                sender_key_messages: Vec::new(),
                relay_gaps: next.relay_gaps,
                pending_fanouts,
            },
            local_sibling: GroupPreparedPublish::empty(),
        })
    }

    #[cfg(test)]
    pub(super) fn group_sender_key_repair_response_throttled(
        &self,
        requester_owner: NdrOwnerPubkey,
        request: &SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) -> bool {
        let revision_only = request.required_revision.is_some()
            && !self.group_manager.is_local_sender_key_stream(
                &request.group_id,
                request.sender_event_pubkey,
                self.local_device,
            );
        let key = Self::repair_response_throttle_key(requester_owner, None, request, revision_only);
        self.repair_response_throttled_for_key(&key, now)
    }

    fn repair_response_throttle_key(
        requester_owner: NdrOwnerPubkey,
        requester_device: Option<NdrDevicePubkey>,
        request: &SenderKeyRepairRequest,
        revision_only: bool,
    ) -> RepairResponseThrottleKey {
        RepairResponseThrottleKey {
            requester_owner_hex: requester_owner.to_hex(),
            requester_device_hex: requester_device.map(|device| device.to_hex()),
            group_id: request.group_id.clone(),
            sender_event_pubkey_hex: if revision_only {
                String::new()
            } else {
                request.sender_event_pubkey.to_hex()
            },
            key_id: if revision_only { None } else { request.key_id },
            message_number: if revision_only {
                None
            } else {
                request.message_number
            },
            required_revision: request.required_revision,
        }
    }

    fn repair_response_entry_matches(
        answered: &ProtocolAnsweredGroupSenderKeyRepair,
        key: &RepairResponseThrottleKey,
    ) -> bool {
        answered.requester_owner_hex == key.requester_owner_hex
            && answered.requester_device_hex == key.requester_device_hex
            && answered.group_id == key.group_id
            && answered.sender_event_pubkey_hex == key.sender_event_pubkey_hex
            && answered.key_id == key.key_id
            && answered.message_number == key.message_number
            && answered.required_revision == key.required_revision
    }

    fn repair_response_has_new_material_for_key(
        &self,
        key: &RepairResponseThrottleKey,
        revision_only: bool,
        metadata_eligible: bool,
        group_revision: u64,
        latest_key_id: Option<u32>,
    ) -> bool {
        let Some(answered) = self
            .answered_group_sender_key_repairs
            .iter()
            .find(|answered| Self::repair_response_entry_matches(answered, key))
        else {
            return false;
        };
        let key_rotated = !revision_only
            && latest_key_id.is_some()
            && answered
                .last_latest_key_id
                .is_some_and(|previous| Some(previous) != latest_key_id);
        let metadata_advanced = metadata_eligible
            && answered.last_group_revision > 0
            && group_revision > answered.last_group_revision;
        key_rotated || metadata_advanced
    }

    fn repair_response_throttled_for_key(
        &self,
        key: &RepairResponseThrottleKey,
        now: NdrUnixSeconds,
    ) -> bool {
        // Peers may recreate the same repair with a fresh timestamp. The missing
        // stream/key/revision owns the backoff, not the request envelope.
        self.answered_group_sender_key_repairs
            .iter()
            .any(|answered| {
                Self::repair_response_entry_matches(answered, key)
                    && answered.next_response_at_secs > now.get()
            })
    }

    fn repair_response_count_for_key(&self, key: &RepairResponseThrottleKey) -> u32 {
        self.answered_group_sender_key_repairs
            .iter()
            .find(|answered| Self::repair_response_entry_matches(answered, key))
            .map_or(0, |answered| answered.response_count)
    }

    #[allow(clippy::too_many_arguments)]
    fn remember_group_sender_key_repair_response_for_key(
        &mut self,
        key: RepairResponseThrottleKey,
        request_created_at_secs: u64,
        now: NdrUnixSeconds,
        revision_only: bool,
        reset_backoff: bool,
        group_revision: u64,
        latest_key_id: Option<u32>,
    ) {
        let position = self
            .answered_group_sender_key_repairs
            .iter()
            .position(|answered| Self::repair_response_entry_matches(answered, &key));
        let response_count = if reset_backoff {
            1
        } else {
            position
                .and_then(|index| {
                    self.answered_group_sender_key_repairs
                        .get(index)
                        .map(|answered| answered.response_count.saturating_add(1))
                })
                .unwrap_or(1)
        };
        let next_response_at_secs =
            protocol_sender_key_repair_response_next_retry_at(now, response_count, revision_only)
                .get();
        if let Some(index) = position {
            if let Some(answered) = self.answered_group_sender_key_repairs.get_mut(index) {
                answered.last_responded_at_secs = now.get();
                answered.request_created_at_secs = request_created_at_secs;
                answered.response_count = response_count;
                answered.next_response_at_secs = next_response_at_secs;
                answered.last_group_revision = group_revision;
                answered.last_latest_key_id = latest_key_id;
            }
        } else {
            self.answered_group_sender_key_repairs
                .push(ProtocolAnsweredGroupSenderKeyRepair {
                    requester_owner_hex: key.requester_owner_hex,
                    requester_device_hex: key.requester_device_hex,
                    group_id: key.group_id,
                    sender_event_pubkey_hex: key.sender_event_pubkey_hex,
                    key_id: key.key_id,
                    message_number: key.message_number,
                    required_revision: key.required_revision,
                    request_created_at_secs,
                    last_responded_at_secs: now.get(),
                    response_count,
                    next_response_at_secs,
                    last_group_revision: group_revision,
                    last_latest_key_id: latest_key_id,
                });
        }
        let excess = self
            .answered_group_sender_key_repairs
            .len()
            .saturating_sub(ANSWERED_GROUP_SENDER_KEY_REPAIR_LIMIT);
        if excess > 0 {
            self.answered_group_sender_key_repairs.drain(0..excess);
        }
    }
}

fn repair_distribution_for(
    record: &nostr_double_ratchet::GroupSenderKeyRecordSnapshot,
    requester_owner: NdrOwnerPubkey,
    key_id: u32,
    message_number: u32,
) -> Option<nostr_double_ratchet::SenderKeyDistribution> {
    record
        .repair_snapshots
        .iter()
        .filter(|snapshot| snapshot.key_id == key_id)
        .filter(|snapshot| snapshot.distribution.iteration <= message_number)
        .filter(|snapshot| snapshot.recipients.contains(&requester_owner))
        .max_by_key(|snapshot| snapshot.distribution.iteration)
        .map(|snapshot| snapshot.distribution.clone())
}

fn repair_distributions_for_requester(
    record: &nostr_double_ratchet::GroupSenderKeyRecordSnapshot,
    requester_owner: NdrOwnerPubkey,
) -> Vec<nostr_double_ratchet::SenderKeyDistribution> {
    // One chain key at a lower iteration derives every later message key of
    // the same key_id. Deduplicating per key_id avoids sending redundant
    // higher-iteration snapshots to a requester already authorized for earlier
    // positions in that epoch. Emit oldest epochs first to match history order.
    let mut best_by_key: BTreeMap<u32, nostr_double_ratchet::SenderKeyDistribution> =
        BTreeMap::new();
    for snapshot in &record.repair_snapshots {
        if !snapshot.recipients.contains(&requester_owner) {
            continue;
        }
        best_by_key
            .entry(snapshot.key_id)
            .and_modify(|existing| {
                if snapshot.distribution.iteration < existing.iteration {
                    *existing = snapshot.distribution.clone();
                }
            })
            .or_insert_with(|| snapshot.distribution.clone());
    }
    let mut distributions = best_by_key.into_values().collect::<Vec<_>>();
    distributions.sort_by_key(|dist| (dist.created_at.get(), dist.key_id));
    distributions
}

fn requester_received_initial_repair_snapshot(
    record: &nostr_double_ratchet::GroupSenderKeyRecordSnapshot,
    requester_owner: NdrOwnerPubkey,
    distribution: &nostr_double_ratchet::SenderKeyDistribution,
) -> bool {
    let Some(first_iteration) = record
        .repair_snapshots
        .iter()
        .filter(|snapshot| snapshot.key_id == distribution.key_id)
        .map(|snapshot| snapshot.distribution.iteration)
        .min()
    else {
        return false;
    };
    first_iteration == distribution.iteration
        && record.repair_snapshots.iter().any(|snapshot| {
            snapshot.key_id == distribution.key_id
                && snapshot.distribution.iteration == first_iteration
                && snapshot.recipients.contains(&requester_owner)
        })
}

fn merge_group_publish(target: &mut GroupPreparedPublish, extra: GroupPreparedPublish) {
    target.deliveries.extend(extra.deliveries);
    target.invite_responses.extend(extra.invite_responses);
    target.sender_key_messages.extend(extra.sender_key_messages);
    target.relay_gaps.extend(extra.relay_gaps);
    target.relay_gaps.sort();
    target.relay_gaps.dedup();
    target.pending_fanouts.extend(extra.pending_fanouts);
}
