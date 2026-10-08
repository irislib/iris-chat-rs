use super::*;

pub(super) const BLOCK_CONTROL_KIND: u32 = 10454;
const MAX_REVISION: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BlockState {
    v: u8,
    owner: String,
    pub(super) target: String,
    blocked: bool,
    revision: u64,
    blocked_since: Option<u64>,
    deleted_at: u64,
}

pub(super) fn block_event_state(event: &Event) -> Option<BlockState> {
    if event.kind.as_u16() != BLOCK_CONTROL_KIND as u16
        || event.content.len() > 2048
        || event.tags.len() != 2
        || event.verify().is_err()
    {
        return None;
    }
    let state: BlockState = serde_json::from_str(&event.content).ok()?;
    if state.v != 1
        || state.revision > MAX_REVISION
        || PublicKey::from_hex(&state.owner).ok()?.to_hex() != state.owner
        || PublicKey::from_hex(&state.target).ok()?.to_hex() != state.target
        || state.owner == state.target
        || (state.blocked && state.blocked_since.is_none())
        || state
            .blocked_since
            .is_some_and(|time| time == 0 || time > event.created_at.as_secs())
        || state.deleted_at > event.created_at.as_secs().saturating_add(300)
        || !event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["p", &state.owner])
        || !event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["d", &format!("iris:block:{}", state.target)])
    {
        return None;
    }
    Some(state)
}

pub(super) fn block_event_version(event: &Event) -> Option<(u64, bool, String, String)> {
    let state = block_event_state(event)?;
    // A concurrent block wins an unblock at the same revision. A subsequent
    // explicit unblock increments the observed revision and always wins.
    Some((
        state.revision,
        state.blocked,
        event.pubkey.to_hex(),
        event.id.to_hex(),
    ))
}

// Every transition remains a signed State record. Unlike a last-value head,
// this log can repair multiple offline block/unblock intervals without widening
// them across the allowed gaps. The in-memory projection keeps reads cheap.
#[derive(Default)]
pub(super) struct PrivateBlockPolicy {
    events: BTreeMap<String, Event>,
    intervals: Vec<(u64, Option<u64>)>,
}
impl PrivateBlockPolicy {
    fn head(&self) -> Option<&Event> {
        self.events
            .values()
            .max_by_key(|event| block_event_version(event))
    }
    fn rebuild(&mut self) {
        let mut events = self
            .events
            .values()
            .filter_map(|event| {
                Some((
                    block_event_version(event)?,
                    event,
                    block_event_state(event)?,
                ))
            })
            .collect::<Vec<_>>();
        events.sort_by(|a, b| a.0.cmp(&b.0));
        self.intervals.clear();
        for (index, (_, event, state)) in events.iter().enumerate() {
            if state.blocked {
                let since = state.blocked_since.unwrap_or(event.created_at.as_secs());
                let until = events[index + 1..]
                    .iter()
                    .find(|(_, _, next)| !next.blocked)
                    .map(|(_, event, _)| event.created_at.as_secs());
                if until.is_none_or(|until| until > since) {
                    self.intervals.push((since, until));
                }
            } else if let Some(since) = state.blocked_since {
                // The unblock explicitly attests its observed interval, so live
                // delivery remains safe when it arrives before the older block.
                let until = event.created_at.as_secs();
                if until > since {
                    self.intervals.push((since, Some(until)));
                }
            }
        }
        self.normalize_intervals();
    }
    fn normalize_intervals(&mut self) {
        self.intervals
            .sort_by_key(|(since, until)| (*since, until.unwrap_or(u64::MAX)));
        let mut merged: Vec<(u64, Option<u64>)> = Vec::new();
        for (since, until) in self.intervals.drain(..) {
            if let Some(previous) = merged
                .last_mut()
                .filter(|previous| previous.1.is_none_or(|end| since <= end))
            {
                previous.1 = match (previous.1, until) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                };
            } else {
                merged.push((since, until));
            }
        }
        self.intervals = merged;
    }
    fn suppresses(&self, created: u64) -> bool {
        self.intervals
            .iter()
            .any(|(since, until)| created >= *since && until.is_none_or(|until| created < until))
    }
}

impl AppCore {
    pub(super) fn block_visibility_revision(&self) -> u64 {
        self.private_block_revision
    }

    pub(super) fn block_intervals_with_event(&self, event: &Event) -> anyhow::Result<String> {
        let owner = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No account"))?
            .owner_pubkey
            .to_hex();
        let mut events = self
            .private_blocks
            .values()
            .flat_map(|policy| policy.events.values().cloned())
            .collect::<Vec<_>>();
        events.push(event.clone());
        blocked_intervals_json(events, &owner)
    }

    pub(super) fn private_block_event_allowed(&self, event: &Event) -> bool {
        block_event_state(event).is_some_and(|state| {
            self.logged_in
                .as_ref()
                .is_some_and(|login| state.owner == login.owner_pubkey.to_hex())
        }) && event.created_at.as_secs() <= unix_now().get().saturating_add(300)
    }

    pub(super) fn private_block_event(&self, target: &str) -> Option<Event> {
        self.private_blocks
            .get(target)
            .and_then(|policy| policy.head())
            .cloned()
    }

    pub(super) fn block_allows_history(&self, chat: &str, author: &str, created: u64) -> bool {
        if !is_group_chat_id(chat) && self.is_owner_blocked(author) {
            return false;
        }
        self.private_blocks
            .get(author)
            .is_none_or(|policy| !policy.suppresses(created))
    }

    pub(super) fn project_private_block(&mut self, event: &Event) -> bool {
        let Some(incoming) = block_event_state(event) else {
            return false;
        };
        let policy = self
            .private_blocks
            .entry(incoming.target.clone())
            .or_default();
        let policy_changed = policy
            .events
            .insert(event.id.to_hex(), event.clone())
            .is_none();
        policy.rebuild();
        if policy_changed {
            self.private_block_revision = self.private_block_revision.wrapping_add(1);
        }
        let Some(state) = policy.head().and_then(block_event_state) else {
            return false;
        };
        let deleted_at = policy
            .events
            .values()
            .filter_map(block_event_state)
            .map(|state| state.deleted_at)
            .max()
            .unwrap_or(0);
        if deleted_at > 0
            && self
                .chat_deletions
                .get(&state.target)
                .is_none_or(|old| *old < deleted_at)
            && !self.apply_chat_deletion(&state.target, deleted_at)
        {
            return false;
        }
        let changed = self.is_owner_blocked(&state.target) != state.blocked;
        self.preferences
            .blocked_owner_pubkeys
            .retain(|key| key != &state.target);
        if state.blocked {
            self.preferences
                .blocked_owner_pubkeys
                .push(state.target.clone());
            self.preferences.blocked_owner_pubkeys.sort();
            self.cancel_direct_files_for_chat(&state.target);
            if self.calls.active.is_some()
                && self
                    .state
                    .call
                    .as_ref()
                    .is_some_and(|call| call.chat_id == state.target)
            {
                self.finish_call("Call ended");
            }
        }
        if changed {
            self.bump_user_discovery_revision();
            self.request_protocol_subscription_refresh();
            self.mark_mobile_push_dirty();
        }
        if policy_changed && !changed {
            self.bump_user_discovery_revision();
        }
        true
    }

    pub(super) fn set_user_blocked(&mut self, target: &str, blocked: bool) {
        let Ok(target) = PublicKey::parse(target.trim()) else {
            return;
        };
        let target = target.to_hex();
        if self.is_owner_blocked(&target) == blocked {
            return;
        }
        if let Err(error) = self.write_private_block(&target, blocked, false) {
            self.push_debug_log("block.save_failed", error.to_string());
            self.state.toast = Some("Could not save blocked user. Try again.".into());
        }
        self.rebuild_persist_and_emit_state();
    }

    fn write_private_block(
        &mut self,
        target: &str,
        blocked: bool,
        legacy: bool,
    ) -> anyhow::Result<()> {
        let login = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No account"))?;
        let owner = login.owner_pubkey;
        anyhow::ensure!(target != owner.to_hex(), "Cannot block yourself");
        let previous = self
            .private_block_event(target)
            .and_then(|event| block_event_state(&event));
        let revision = if legacy {
            0
        } else {
            previous.as_ref().map_or(1, |state| state.revision + 1)
        };
        anyhow::ensure!(revision <= MAX_REVISION, "Block revision exhausted");
        let now = unix_now().get().max(
            self.private_block_event(target)
                .map_or(0, |event| event.created_at.as_secs()),
        );
        let deleted_at = previous
            .as_ref()
            .map_or(0, |state| state.deleted_at)
            .max(if blocked {
                self.threads
                    .get(target)
                    .map_or(now, |thread| now.max(thread.updated_at_secs))
            } else {
                0
            });
        let state = BlockState {
            v: 1,
            owner: owner.to_hex(),
            target: target.into(),
            blocked,
            revision,
            blocked_since: if blocked {
                Some(now)
            } else {
                previous.as_ref().and_then(|state| state.blocked_since)
            },
            deleted_at,
        };
        let event = EventBuilder::new(
            Kind::Custom(BLOCK_CONTROL_KIND as u16),
            serde_json::to_string(&state)?,
        )
        .tag(nostr::Tag::public_key(owner))
        .tag(nostr::Tag::identifier(format!("iris:block:{target}")))
        .allow_self_tagging()
        .custom_created_at(Timestamp::from_secs(now))
        .sign_with_keys(&login.device_keys)?;
        anyhow::ensure!(
            self.apply_private_block_event(event.clone()),
            "Block not persisted"
        );
        // The current authorized sibling's encrypted envelope explicitly attests
        // this private account state. The original signed writer may since have
        // been removed; its signature protects integrity, not current access.
        let content =
            serde_json::json!({"type":"private-block-state","v":1,"event":event}).to_string();
        let unsigned = EventBuilder::new(Kind::Custom(BLOCK_CONTROL_KIND as u16), content)
            .tag(nostr::Tag::public_key(owner))
            .allow_self_tagging()
            .build(owner);
        self.send_protocol_engine_unsigned_event_to_local_siblings(
            owner,
            &owner.to_hex(),
            unsigned,
            "block.self_sync",
        );
        self.broadcast_device_sync_snapshot();
        Ok(())
    }

    pub(super) fn migrate_legacy_blocks(&mut self) {
        for target in self.preferences.blocked_owner_pubkeys.clone() {
            if self.private_block_event(&target).is_none() {
                if let Err(error) = self.write_private_block(&target, true, true) {
                    self.push_debug_log("block.migrate_failed", error.to_string());
                }
            }
        }
    }

    pub(super) fn receive_block_control(
        &mut self,
        owner: PublicKey,
        device: Option<PublicKey>,
        content: &str,
    ) -> bool {
        if content.len() > 8192
            || self
                .logged_in
                .as_ref()
                .is_none_or(|login| login.owner_pubkey != owner)
            || !device.is_some_and(|device| self.device_sync_peer_is_authorized(&device.to_hex()))
        {
            return true;
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Control {
            #[serde(rename = "type")]
            kind: String,
            v: u8,
            event: Event,
        }
        let Ok(control) = serde_json::from_str::<Control>(content) else {
            return true;
        };
        if control.kind != "private-block-state"
            || control.v != 1
            || !self.private_block_event_allowed(&control.event)
        {
            return true;
        }
        if !self.apply_private_block_event(control.event) {
            return false;
        }
        self.rebuild_persist_and_emit_state();
        self.broadcast_device_sync_snapshot();
        true
    }
}

// Shared by actor projection and the read-only pagination/search connection.
// Invalid stored signed state is an error, never silently treated as no blocks.
pub(super) fn blocked_intervals_json(events: Vec<Event>, owner: &str) -> anyhow::Result<String> {
    let mut policies: BTreeMap<String, PrivateBlockPolicy> = BTreeMap::new();
    for event in events {
        let state = block_event_state(&event)
            .ok_or_else(|| anyhow::anyhow!("Invalid private block state"))?;
        anyhow::ensure!(state.owner == owner, "Private block account mismatch");
        policies
            .entry(state.target)
            .or_default()
            .events
            .insert(event.id.to_hex(), event);
    }
    let mut intervals = Vec::new();
    for (author, mut policy) in policies {
        policy.rebuild();
        for (since, until) in policy.intervals {
            intervals.push(serde_json::json!({"author":author,"since":since,"until":until}));
        }
    }
    Ok(serde_json::to_string(&intervals)?)
}
