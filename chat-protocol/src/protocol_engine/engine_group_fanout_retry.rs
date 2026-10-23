#[cfg(test)]
thread_local! {
    static GROUP_FANOUT_RETRY_VISITED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Default)]
struct GroupFanoutRetryUndo {
    replaced: Vec<(usize, ProtocolPendingGroupFanout)>,
    removed: Vec<(usize, ProtocolPendingGroupFanout)>,
}

impl GroupFanoutRetryUndo {
    fn apply(
        &mut self,
        queue: &mut Vec<ProtocolPendingGroupFanout>,
        updates: Vec<(usize, Option<ProtocolPendingGroupFanout>)>,
    ) {
        let mut removals = Vec::new();
        for (index, replacement) in updates {
            if let Some(replacement) = replacement {
                self.replaced
                    .push((index, std::mem::replace(&mut queue[index], replacement)));
            } else {
                removals.push(index);
            }
        }
        if removals.is_empty() {
            return;
        }
        // Candidate indices are ordered. Compact once only when entries really
        // retire; ordinary backoff updates never move the untouched tail.
        let mut index = 0;
        let mut next_removal = 0;
        self.removed = queue
            .extract_if(.., |_| {
                let remove = removals.get(next_removal) == Some(&index);
                next_removal += usize::from(remove);
                index += 1;
                remove
            })
            .enumerate()
            .map(|(position, pending)| (removals[position], pending))
            .collect();
    }

    fn restore(self, queue: &mut Vec<ProtocolPendingGroupFanout>) {
        if !self.removed.is_empty() {
            let original_len = queue.len() + self.removed.len();
            let mut remaining = std::mem::take(queue).into_iter();
            let mut removed = self.removed.into_iter().peekable();
            queue.reserve(original_len);
            for index in 0..original_len {
                if removed.peek().is_some_and(|(removed, _)| *removed == index) {
                    queue.push(removed.next().unwrap().1);
                } else {
                    queue.push(remaining.next().expect("unchanged fanout entry"));
                }
            }
        }
        for (index, original) in self.replaced {
            queue[index] = original;
        }
    }
}

impl ProtocolEngine {
    fn retry_pending_group_fanouts(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        if !self
            .pending_group_fanouts
            .iter()
            .any(|pending| pending.next_retry_at_secs <= now.get())
        {
            return Ok(ProtocolGroupIncomingResult::default());
        }
        self.with_group_fanout_checkpoint(|engine, undo| {
            engine.retry_pending_group_fanouts_inner(now, undo)
        })
    }

    fn group_fanout_retry_candidates(
        &self,
        now: NdrUnixSeconds,
    ) -> Vec<(usize, Option<ProtocolPendingGroupFanout>)> {
        let mut candidates = Vec::new();
        // This copies group metadata only, never sender-key state. With no
        // inactive group or held peer, the tail cannot require retirement.
        let inactive_groups = self
            .group_manager
            .groups()
            .into_iter()
            .filter(|group| !group.members.contains(&self.local_owner))
            .map(|group| group.group_id)
            .collect::<HashSet<_>>();
        let can_stop_after_due = self.held_direct_chats.is_empty() && inactive_groups.is_empty();
        let mut held_owners = BTreeMap::new();
        let mut due = 0;
        for (index, pending) in self.pending_group_fanouts.iter().enumerate() {
            #[cfg(test)]
            GROUP_FANOUT_RETRY_VISITED.with(|visited| visited.set(visited.get() + 1));
            let held_wake = match pending.fanout.as_ref() {
                GroupPendingFanout::Remote {
                    recipient_owner,
                    payload,
                } if !self.held_direct_chats.is_empty() => {
                    *held_owners.entry(*recipient_owner).or_insert_with(|| {
                        self.held_direct_chats.contains(&recipient_owner.to_hex())
                    }) && serde_json::from_slice::<UnsignedEvent>(payload)
                        .is_ok_and(|event| event.kind.as_u16() == 21_113)
                }
                _ => false,
            };
            let revoked_payload = inactive_groups.contains(&pending.group_id) && {
                let payload = match pending.fanout.as_ref() {
                    GroupPendingFanout::Remote { payload, .. }
                    | GroupPendingFanout::LocalSiblings { payload } => payload,
                };
                // After removal, even deferred key handoffs must retire; only
                // membership metadata may remain available for delivery.
                !matches!(
                    JsonGroupPayloadCodecV1.decode_pairwise_command(payload),
                    Ok(Some(GroupPairwiseCommand::MetadataSnapshot { snapshot }))
                        if snapshot.group_id == pending.group_id
                )
            };
            if held_wake || revoked_payload {
                candidates.push((index, None));
            } else if pending.next_retry_at_secs <= now.get()
                && due < PENDING_GROUP_FANOUT_RETRY_BATCH_SIZE
            {
                due += 1;
                candidates.push((index, Some(pending.clone())));
                if can_stop_after_due && due == PENDING_GROUP_FANOUT_RETRY_BATCH_SIZE {
                    break;
                }
            }
        }
        candidates
    }

    fn retry_pending_group_fanouts_inner(
        &mut self,
        now: NdrUnixSeconds,
        undo: &mut GroupFanoutRetryUndo,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        // Keep the original ordered queue installed while preparing deliveries.
        // Only selected entries are cloned; deferred metadata stays untouched.
        let candidates = self.group_fanout_retry_candidates(now);
        let mut updates = Vec::with_capacity(candidates.len());
        let mut effects = Vec::new();
        let mut persist_needed = false;
        let mut session_changed = false;
        let mut ready_publications_changed = false;
        for (index, pending) in candidates {
            let Some(mut pending) = pending else {
                persist_needed = true;
                updates.push((index, None));
                continue;
            };
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(now, &mut rng);
            if let Some(devices) = pending.remaining_devices.as_mut() {
                let owner = match pending.fanout.as_ref() {
                    GroupPendingFanout::Remote {
                        recipient_owner, ..
                    } => *recipient_owner,
                    GroupPendingFanout::LocalSiblings { .. } => self.local_owner,
                };
                if let Some(current) = self.session_manager.roster(owner) {
                    let previous_len = devices.len();
                    devices.retain(|device| current.get_device(device).is_some());
                    persist_needed |= devices.len() != previous_len;
                }
                if devices.is_empty() {
                    persist_needed = true;
                    updates.push((index, None));
                    continue;
                }
            }
            // The original send already reached every device absent from this
            // set. Retrying the entire owner causes duplicate encryption and
            // relay traffic for each unavailable linked device.
            let prepared = match (pending.fanout.as_ref(), &pending.remaining_devices) {
                (
                    GroupPendingFanout::Remote {
                        recipient_owner,
                        payload,
                    },
                    Some(devices),
                ) => self.session_manager.prepare_remote_send_to_devices(
                    &mut ctx,
                    *recipient_owner,
                    devices.iter().copied(),
                    payload.clone(),
                ),
                (GroupPendingFanout::LocalSiblings { payload }, Some(devices)) => {
                    self.session_manager.prepare_local_sibling_send_to_devices(
                        &mut ctx,
                        devices.iter().copied(),
                        payload.clone(),
                    )
                }
                (
                    GroupPendingFanout::Remote {
                        recipient_owner,
                        payload,
                    },
                    None,
                ) => self.session_manager.prepare_remote_send(
                    &mut ctx,
                    *recipient_owner,
                    payload.clone(),
                ),
                (GroupPendingFanout::LocalSiblings { payload }, None) => self
                    .session_manager
                    .prepare_local_sibling_send_reusing_all_sessions(&mut ctx, payload.clone()),
            }
            .map(|prepared| {
                group_publish_from_prepared_send(prepared, pending.fanout.as_ref().clone())
            });
            let prepared = match prepared {
                Ok(prepared) => {
                    if !prepared.deliveries.is_empty()
                        || !prepared.invite_responses.is_empty()
                        || !prepared.sender_key_messages.is_empty()
                    {
                        session_changed = true;
                    }
                    prepared
                }
                Err(_) => {
                    pending.next_retry_at_secs =
                        next_pending_retry_at_secs(pending.created_at_secs, now);
                    updates.push((index, Some(pending)));
                    continue;
                }
            };
            let remaining_devices =
                self.group_fanout_missing_devices(pending.fanout.as_ref(), &prepared.relay_gaps);
            persist_needed |= pending.remaining_devices != remaining_devices;
            pending.remaining_devices = remaining_devices;
            let still_has_gap = !prepared.relay_gaps.is_empty();
            // Save progress and removed targets, but keep backoff-only retries
            // in memory instead of serializing unchanged ratchet state.
            persist_needed |= !still_has_gap;
            let mut event_ids = Vec::new();
            let chat_id = group_chat_id(&pending.group_id);
            let ready = protocol_effects_from_group_prepared_publish(
                &prepared,
                self,
                pending.inner_event_id.clone(),
                chat_id,
                &mut event_ids,
            )?;
            let payload = match pending.fanout.as_ref() {
                GroupPendingFanout::Remote { payload, .. }
                | GroupPendingFanout::LocalSiblings { payload } => payload,
            };
            let inner = decode_local_sibling_payload(payload).map(|(_, _, payload)| payload);
            if serde_json::from_slice::<UnsignedEvent>(inner.as_deref().unwrap_or(payload))
                .is_ok_and(|event| event.kind.as_u16() == 21_113)
            {
                ready_publications_changed |= !ready.is_empty();
                let recipient = match pending.fanout.as_ref() {
                    GroupPendingFanout::Remote {
                        recipient_owner, ..
                    } => Some(*recipient_owner),
                    GroupPendingFanout::LocalSiblings { .. } => None,
                };
                self.retain_group_publications(&ready, recipient)?;
            }
            effects.extend(ready);
            if still_has_gap {
                pending.next_retry_at_secs =
                    next_pending_retry_at_secs(pending.created_at_secs, now);
                updates.push((index, Some(pending)));
            } else {
                updates.push((index, None));
            }
        }
        undo.apply(&mut self.pending_group_fanouts, updates);
        if session_changed {
            self.invalidate_known_message_author_cache();
        }
        if persist_needed || session_changed {
            // A late recipient's ready wake must survive the same checkpoint
            // that removes its plaintext intent and advances its session.
            if ready_publications_changed {
                self.persist_now()?;
            } else {
                self.persist()?;
            }
        }
        Ok(ProtocolGroupIncomingResult {
            effects,
            ..Default::default()
        })
    }
}
