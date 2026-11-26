use super::*;

#[derive(Default)]
pub(super) struct ThreadPersistCache {
    pub(super) header_hash: Option<u64>,
    pub(super) messages: HashMap<String, u64>,
}

/// Per-slice fingerprints from the last successful save or load. We hash each
/// slice (and each row for maps/threads) and skip writes when nothing has
/// changed, so routine saves only touch the exact rows that mutated. The cache
/// is reset on `clear` and on construction (a fresh `AppStore` will issue a
/// full sync the first time it sees state if `load_state` was not called).
#[derive(Default)]
pub(super) struct PersistCache {
    pub(super) meta: Option<u64>,
    pub(super) preferences: Option<u64>,
    pub(super) owner_profiles: Option<HashMap<String, u64>>,
    pub(super) chat_ttls: Option<HashMap<String, u64>>,
    pub(super) app_keys: Option<HashMap<String, u64>>,
    pub(super) groups: Option<HashMap<String, u64>>,
    /// Event ids currently in the `seen_events` table. Mirrors DB rows so
    /// we can compute an INSERT/DELETE diff per save instead of rewriting
    /// the whole window. Populated from `load_state` and after each save.
    pub(super) seen_events_persisted: HashSet<String>,
    pub(super) seen_events_hash: Option<u64>,
    /// Monotonic next-sequence to assign to a newly inserted seen event.
    /// Sequences are only used for `ORDER BY sequence ASC` on load, so
    /// they need to grow but don't need to be dense.
    pub(super) seen_events_next_seq: i64,
    pub(super) threads: HashMap<String, ThreadPersistCache>,
}

/// View into `AppCore` fields used to drive a single `save_state` call.
pub(crate) struct SaveSnapshot<'a> {
    pub active_chat_id: Option<&'a str>,
    pub next_message_id: u64,
    pub authorization_state: Option<PersistedAuthorizationState>,
    pub preferences: &'a PreferencesSnapshot,
    pub owner_profiles: &'a BTreeMap<String, OwnerProfileRecord>,
    pub chat_message_ttl_seconds: &'a BTreeMap<String, u64>,
    pub app_keys: &'a BTreeMap<String, KnownAppKeys>,
    pub groups: &'a BTreeMap<String, GroupSnapshot>,
    pub group_pictures: &'a BTreeMap<String, String>,
    pub threads: &'a BTreeMap<String, ThreadRecord>,
    pub seen_event_order: &'a VecDeque<String>,
}

/// Diff against the persisted `seen_events` mirror. We only apply rows
/// that actually changed instead of rewriting the whole window every save:
/// the window holds up to MAX_SEEN_EVENT_IDS (2048) entries and the old
/// code did `DELETE FROM seen_events` + `INSERT × N` on every relay event,
/// which was the SQLite hot path that iOS RUNNINGBOARD 0xdead10cc'd while
/// the journal was mid-fsync.
pub(super) struct SeenEventsPlan {
    pub(super) to_insert: Vec<String>,
    pub(super) to_delete: Vec<String>,
    pub(super) first_insert_seq: i64,
    pub(super) next_hash: u64,
    pub(super) next_sequence: i64,
}

pub(super) struct MapDiff {
    pub(super) full_replace: bool,
    pub(super) upserts: Vec<(String, u64)>,
    pub(super) deletes: Vec<String>,
}

pub(super) struct ThreadWritePlan {
    pub(super) header_hash: Option<u64>,
    pub(super) messages: Vec<(usize, String, u64)>,
}

impl ThreadWritePlan {
    pub(super) fn apply_to_cache(self, entry: &mut ThreadPersistCache) {
        if let Some(hash) = self.header_hash {
            entry.header_hash = Some(hash);
        }
        for (_, id, hash) in self.messages {
            entry.messages.insert(id, hash);
        }
    }
}

/// Decision tree for one save tick. Carries the new fingerprints so the
/// cache can be updated atomically with the write.
pub(super) struct SavePlan {
    pub(super) meta: Option<u64>,
    pub(super) preferences: Option<u64>,
    pub(super) owner_profiles: Option<MapDiff>,
    pub(super) chat_ttls: Option<MapDiff>,
    pub(super) app_keys: Option<MapDiff>,
    pub(super) groups: Option<MapDiff>,
    pub(super) seen_events: Option<SeenEventsPlan>,
    /// chat_id -> incremental thread/message writes; only changed threads are listed here.
    pub(super) threads_to_write: HashMap<String, ThreadWritePlan>,
    /// chat_ids cached previously but no longer present in the snapshot.
    pub(super) threads_to_delete: Vec<String>,
}

impl SavePlan {
    pub(super) fn compute(cache: &PersistCache, snapshot: &SaveSnapshot<'_>) -> Self {
        let meta_hash = hash_meta(snapshot);
        let preferences_hash = hash_preferences(snapshot.preferences);
        let owner_profiles = diff_map(
            cache.owner_profiles.as_ref(),
            snapshot.owner_profiles,
            |_, profile| hash_std(profile),
        );
        let chat_ttls = diff_map(
            cache.chat_ttls.as_ref(),
            snapshot.chat_message_ttl_seconds,
            |_, ttl| hash_std(ttl),
        );
        let app_keys = diff_map(cache.app_keys.as_ref(), snapshot.app_keys, |_, entry| {
            hash_std(entry)
        });
        let groups = diff_map(cache.groups.as_ref(), snapshot.groups, |id, group| {
            hash_group_entry(group, snapshot.group_pictures.get(id))
        });
        let seen_events = plan_seen_events_diff(cache, snapshot.seen_event_order);

        let mut threads_to_write = HashMap::new();
        for (chat_id, thread) in snapshot.threads {
            if let Some(plan) = plan_thread_write(cache.threads.get(chat_id), thread) {
                threads_to_write.insert(chat_id.clone(), plan);
            }
        }
        let threads_to_delete: Vec<String> = cache
            .threads
            .keys()
            .filter(|chat_id| !snapshot.threads.contains_key(chat_id.as_str()))
            .cloned()
            .collect();

        Self {
            meta: changed(cache.meta, meta_hash),
            preferences: changed(cache.preferences, preferences_hash),
            owner_profiles,
            chat_ttls,
            app_keys,
            groups,
            seen_events,
            threads_to_write,
            threads_to_delete,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.meta.is_none()
            && self.preferences.is_none()
            && self.owner_profiles.is_none()
            && self.chat_ttls.is_none()
            && self.app_keys.is_none()
            && self.groups.is_none()
            && self.seen_events.is_none()
            && self.threads_to_write.is_empty()
            && self.threads_to_delete.is_empty()
    }

    pub(super) fn apply(
        &self,
        tx: &Transaction,
        snapshot: &SaveSnapshot<'_>,
    ) -> anyhow::Result<()> {
        if self.meta.is_some() {
            write_meta(tx, snapshot)?;
        }
        if self.preferences.is_some() {
            write_preferences(tx, snapshot.preferences)?;
        }
        if let Some(diff) = &self.owner_profiles {
            write_owner_profiles_diff(tx, snapshot.owner_profiles, diff)?;
        }
        if let Some(diff) = &self.chat_ttls {
            write_chat_ttls_diff(tx, snapshot.chat_message_ttl_seconds, diff)?;
        }
        if let Some(diff) = &self.app_keys {
            write_app_keys_diff(tx, snapshot.app_keys, diff)?;
        }
        if let Some(diff) = &self.groups {
            write_groups_diff(tx, snapshot.groups, snapshot.group_pictures, diff)?;
        }
        if let Some(plan) = &self.seen_events {
            apply_seen_events_diff(tx, plan)?;
        }
        for chat_id in &self.threads_to_delete {
            // Cascades to messages.
            tx.prepare_cached("DELETE FROM threads WHERE chat_id = ?1")?
                .execute([chat_id])?;
        }
        for (chat_id, plan) in &self.threads_to_write {
            let Some(thread) = snapshot.threads.get(chat_id) else {
                return Err(anyhow::anyhow!("persist plan missing thread {chat_id}"));
            };
            apply_thread_write_plan(tx, chat_id, thread, plan)?;
        }
        Ok(())
    }

    pub(super) fn update_cache(self, cache: &mut PersistCache) {
        if let Some(hash) = self.meta {
            cache.meta = Some(hash);
        }
        if let Some(hash) = self.preferences {
            cache.preferences = Some(hash);
        }
        if let Some(diff) = self.owner_profiles {
            apply_map_diff_to_cache(&mut cache.owner_profiles, diff);
        }
        if let Some(diff) = self.chat_ttls {
            apply_map_diff_to_cache(&mut cache.chat_ttls, diff);
        }
        if let Some(diff) = self.app_keys {
            apply_map_diff_to_cache(&mut cache.app_keys, diff);
        }
        if let Some(diff) = self.groups {
            apply_map_diff_to_cache(&mut cache.groups, diff);
        }
        if let Some(plan) = self.seen_events {
            for event_id in &plan.to_delete {
                cache.seen_events_persisted.remove(event_id);
            }
            for event_id in plan.to_insert {
                cache.seen_events_persisted.insert(event_id);
            }
            cache.seen_events_hash = Some(plan.next_hash);
            cache.seen_events_next_seq = plan.next_sequence;
        }
        for chat_id in self.threads_to_delete {
            cache.threads.remove(&chat_id);
        }
        for (chat_id, plan) in self.threads_to_write {
            let entry = cache.threads.entry(chat_id).or_default();
            plan.apply_to_cache(entry);
        }
    }
}

fn diff_map<V>(
    cached: Option<&HashMap<String, u64>>,
    current: &BTreeMap<String, V>,
    mut hash_entry: impl FnMut(&str, &V) -> u64,
) -> Option<MapDiff> {
    let Some(cached) = cached else {
        let upserts = current
            .iter()
            .map(|(k, v)| (k.clone(), hash_entry(k, v)))
            .collect();
        return Some(MapDiff {
            full_replace: true,
            upserts,
            deletes: Vec::new(),
        });
    };
    let mut upserts = Vec::new();
    for (k, v) in current {
        let hash = hash_entry(k, v);
        if cached.get(k) != Some(&hash) {
            upserts.push((k.clone(), hash));
        }
    }
    let mut deletes = Vec::new();
    for k in cached.keys() {
        if !current.contains_key(k) {
            deletes.push(k.clone());
        }
    }
    if upserts.is_empty() && deletes.is_empty() {
        None
    } else {
        Some(MapDiff {
            full_replace: false,
            upserts,
            deletes,
        })
    }
}

fn apply_map_diff_to_cache(slot: &mut Option<HashMap<String, u64>>, diff: MapDiff) {
    if diff.full_replace {
        *slot = Some(diff.upserts.into_iter().collect());
        return;
    }
    let map = slot.get_or_insert_with(HashMap::new);
    for key in diff.deletes {
        map.remove(&key);
    }
    for (key, hash) in diff.upserts {
        map.insert(key, hash);
    }
}

fn changed(previous: Option<u64>, current: u64) -> Option<u64> {
    if previous == Some(current) {
        None
    } else {
        Some(current)
    }
}

pub(super) fn hash_std<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

struct HashWriter<'a>(&'a mut DefaultHasher);

impl std::io::Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn hash_meta(snapshot: &SaveSnapshot<'_>) -> u64 {
    hash_meta_fields(
        snapshot.active_chat_id,
        snapshot.next_message_id,
        snapshot.authorization_state.as_ref(),
    )
}

pub(super) fn hash_meta_fields(
    active_chat_id: Option<&str>,
    next_message_id: u64,
    authorization_state: Option<&PersistedAuthorizationState>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    active_chat_id.hash(&mut hasher);
    next_message_id.hash(&mut hasher);
    serialize_authorization_state(authorization_state).hash(&mut hasher);
    hasher.finish()
}

pub(super) fn hash_group_entry(group: &GroupSnapshot, picture: Option<&String>) -> u64 {
    let mut hasher = DefaultHasher::new();
    // GroupSnapshot isn't Hash; stream JSON directly into the hasher without allocating.
    let _ = serde_json::to_writer(HashWriter(&mut hasher), group);
    picture.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn hash_seen_events_iter<'a>(len: usize, iter: impl Iterator<Item = &'a str>) -> u64 {
    let mut hasher = DefaultHasher::new();
    len.hash(&mut hasher);
    for event_id in iter {
        event_id.hash(&mut hasher);
    }
    hasher.finish()
}

/// Build the set of `INSERT` / `DELETE` ops needed to bring the persisted
/// `seen_events` window in line with the in-memory snapshot. Returns
/// `None` when nothing changed so the caller can short-circuit and skip
/// the transaction entirely.
fn plan_seen_events_diff(
    cache: &PersistCache,
    seen_event_order: &VecDeque<String>,
) -> Option<SeenEventsPlan> {
    let current_hash = hash_seen_events_iter(
        seen_event_order.len(),
        seen_event_order.iter().map(String::as_str),
    );
    if cache.seen_events_hash == Some(current_hash)
        && cache.seen_events_persisted.len() == seen_event_order.len()
    {
        return None;
    }

    let mut current_refs: HashSet<&str> = HashSet::with_capacity(seen_event_order.len());
    let mut to_insert: Vec<String> = Vec::new();
    for event_id in seen_event_order {
        if current_refs.insert(event_id.as_str()) && !cache.seen_events_persisted.contains(event_id)
        {
            to_insert.push(event_id.clone());
        }
    }
    let to_delete: Vec<String> = cache
        .seen_events_persisted
        .iter()
        .filter(|event_id| !current_refs.contains(event_id.as_str()))
        .cloned()
        .collect();
    if to_insert.is_empty() && to_delete.is_empty() {
        return None;
    }
    let first_insert_seq = cache.seen_events_next_seq;
    let next_sequence = first_insert_seq.saturating_add(to_insert.len() as i64);
    Some(SeenEventsPlan {
        to_insert,
        to_delete,
        first_insert_seq,
        next_hash: current_hash,
        next_sequence,
    })
}

pub(super) fn hash_thread_header(thread: &ThreadRecord) -> u64 {
    hash_thread_header_fields(thread.unread_count, thread.updated_at_secs, &thread.draft)
}

pub(super) fn hash_thread_header_fields(
    unread_count: u64,
    updated_at_secs: u64,
    draft: &str,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    unread_count.hash(&mut hasher);
    updated_at_secs.hash(&mut hasher);
    draft.hash(&mut hasher);
    hasher.finish()
}

macro_rules! hash_message_fields {
    ($message:expr, $delivery:expr) => {{
        let message = $message;
        let mut hasher = DefaultHasher::new();
        message.id.hash(&mut hasher);
        message.kind.hash(&mut hasher);
        message.author.hash(&mut hasher);
        message.author_owner_pubkey_hex.hash(&mut hasher);
        message.body.hash(&mut hasher);
        message.edit_history.hash(&mut hasher);
        message.deleted_for_everyone.hash(&mut hasher);
        message.is_outgoing.hash(&mut hasher);
        message.created_at_secs.hash(&mut hasher);
        message.expires_at_secs.hash(&mut hasher);
        $delivery.hash(&mut hasher);
        message.attachments.hash(&mut hasher);
        message.reactions.hash(&mut hasher);
        message.reactors.hash(&mut hasher);
        message.source_event_id.hash(&mut hasher);
        message.recipient_deliveries.hash(&mut hasher);
        message.delivery_trace.hash(&mut hasher);
        message.call.hash(&mut hasher);
        message.system_notice_owner_pubkey_hex.hash(&mut hasher);
        hasher.finish()
    }};
}

pub(super) fn hash_message(message: &ChatMessageSnapshot) -> u64 {
    hash_message_fields!(message, &message.delivery)
}

pub(super) fn hash_persisted_message(message: &PersistedMessage) -> u64 {
    let delivery: DeliveryState = message.delivery.clone().into();
    hash_message_fields!(message, &delivery)
}

pub(super) fn plan_thread_write(
    cached: Option<&ThreadPersistCache>,
    thread: &ThreadRecord,
) -> Option<ThreadWritePlan> {
    let header_hash = hash_thread_header(thread);
    let header_changed = cached.and_then(|entry| entry.header_hash) != Some(header_hash);
    let mut messages = Vec::new();
    for (index, message) in thread.messages.iter().enumerate() {
        let message_hash = hash_message(message);
        if cached.and_then(|entry| entry.messages.get(&message.id)) != Some(&message_hash) {
            messages.push((index, message.id.clone(), message_hash));
        }
    }
    if !header_changed && messages.is_empty() {
        return None;
    }
    Some(ThreadWritePlan {
        header_hash: header_changed.then_some(header_hash),
        messages,
    })
}
