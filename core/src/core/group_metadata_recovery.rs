use super::*;
use nostr_double_ratchet::{GroupPairwiseCommand, GroupPayloadCodec, JsonGroupPayloadCodecV1};
use rusqlite::OptionalExtension;

pub(super) const GROUP_CONTEXT_REQUEST_KIND: u32 = 21_114;
const RECOVERY_PREFIX: &str = "group-context-recovery:";
const RECOVERY_INTERVAL_SECS: u64 = 60;
const MAX_RECOVERY_SLOTS: i64 = 256;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextRequest {
    v: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author: Option<PublicKey>,
    // A member may ask an admin to reply directly to another current member.
    // This identity is carried only inside the private ratchet payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    requester: Option<PublicKey>,
}

impl AppCore {
    // Reserve before sending, so duplicate deliveries and restarts cannot flood
    // peers. The durable input queues drive subsequent bounded retries.
    fn reserve_group_recovery(&self, key: &str) -> anyhow::Result<Option<u64>> {
        let shared = self.app_store.shared();
        let mut conn = shared
            .lock()
            .map_err(|_| anyhow::anyhow!("storage lock poisoned"))?;
        let tx = conn.transaction()?;
        let key = format!("{RECOVERY_PREFIX}{key}");
        let now = unix_now().get();
        let previous: Option<String> = tx
            .query_row("SELECT value FROM app_meta WHERE key=?1", [&key], |row| {
                row.get(0)
            })
            .optional()?;
        let previous = previous
            .map(|value| serde_json::from_str::<(u64, u64)>(&value))
            .transpose()?;
        if previous.is_some_and(|(last, _)| now < last.saturating_add(RECOVERY_INTERVAL_SECS)) {
            return Ok(None);
        }
        tx.execute(
            "DELETE FROM app_meta WHERE key >= 'group-context-recovery:' AND key < 'group-context-recovery;'
             AND json_extract(value, '$[0]') < ?1",
            [now.saturating_sub(86_400)],
        )?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM app_meta WHERE key >= 'group-context-recovery:' AND key < 'group-context-recovery;'",
            [],
            |row| row.get(0),
        )?;
        if previous.is_none() && count >= MAX_RECOVERY_SLOTS {
            return Ok(None);
        }
        let attempt = previous.map_or(0, |(_, attempt)| attempt.saturating_add(1));
        tx.execute(
            "INSERT INTO app_meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![key, serde_json::to_string(&(now, attempt))?],
        )?;
        tx.commit()?;
        Ok(Some(attempt))
    }

    pub(super) fn request_missing_group_context(
        &mut self,
        sender: PublicKey,
        group_id: Option<String>,
        author: Option<PublicKey>,
    ) {
        if self.is_owner_blocked(&sender.to_hex())
            || self
                .logged_in
                .as_ref()
                .is_some_and(|login| login.owner_pubkey == sender)
            || group_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return;
        }
        let selector = author.map(|key| key.to_hex()).or_else(|| group_id.clone());
        let Some(selector) = selector else { return };
        if !matches!(
            self.reserve_group_recovery(&format!("ask:{sender}:{selector}")),
            Ok(Some(_))
        ) {
            return;
        }
        self.send_group_context_request(
            sender,
            &ContextRequest {
                v: 1,
                group_id,
                author,
                requester: None,
            },
        );
    }

    pub(super) fn retry_missing_group_metadata(&mut self) {
        if !self
            .protocol_engine
            .as_ref()
            .is_some_and(ProtocolEngine::has_pending_group_pairwise_payloads)
        {
            return;
        }
        if !matches!(self.reserve_group_recovery("scan"), Ok(Some(_))) {
            return;
        }
        let sources = self
            .protocol_engine
            .as_ref()
            .map(ProtocolEngine::missing_group_metadata_sources)
            .unwrap_or_default();
        for (sender, group_id, author) in sources {
            self.request_missing_group_context(sender, Some(group_id), Some(author));
        }
    }

    fn send_group_context_request(&mut self, peer: PublicKey, request: &ContextRequest) -> bool {
        let Some(local) = self.logged_in.as_ref().map(|login| login.owner_pubkey) else {
            return false;
        };
        let Ok(content) = serde_json::to_string(request) else {
            return false;
        };
        let rumor = UnsignedEvent::new(
            local,
            Timestamp::now(),
            Kind::Custom(GROUP_CONTEXT_REQUEST_KIND as u16),
            // A fresh request replaces expired attempts; stale recipient devices
            // must not retain a new permanent queued send on every retry.
            [nostr::Tag::expiration(Timestamp::from(
                unix_now().get().saturating_add(2 * RECOVERY_INTERVAL_SECS),
            ))],
            content,
        );
        self.send_protocol_engine_unsigned_event_to_peer_only(
            peer,
            &peer.to_hex(),
            rumor,
            "group.context.request",
        )
    }

    pub(super) fn request_context_for_pending_group_payload(
        &mut self,
        sender: PublicKey,
        device: Option<PublicKey>,
        payload: &[u8],
    ) {
        let Some(engine) = self.protocol_engine.as_ref() else {
            return;
        };
        if !device.is_some_and(|device| engine.invite_owner_device_is_authorized(sender, device)) {
            return;
        }
        let Ok(Some(GroupPairwiseCommand::SenderKeyDistribution { distribution })) =
            JsonGroupPayloadCodecV1.decode_pairwise_command(payload)
        else {
            return;
        };
        if engine.group_snapshot(&distribution.group_id).is_none() {
            let author = PublicKey::from_slice(&distribution.sender_event_pubkey.to_bytes()).ok();
            self.request_missing_group_context(sender, Some(distribution.group_id), author);
        }
    }

    pub(super) fn receive_group_context_request(
        &mut self,
        sender: PublicKey,
        content: &str,
    ) -> bool {
        if content.len() > 1024 || self.is_owner_blocked(&sender.to_hex()) {
            return true;
        }
        let Ok(mut request) = serde_json::from_str::<ContextRequest>(content) else {
            return true;
        };
        if request.v != 1
            || request
                .group_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return true;
        }
        let Some(engine) = self.protocol_engine.as_ref() else {
            return false;
        };
        let mapped = request
            .author
            .and_then(|author| engine.group_id_for_sender_event_author(author));
        if let (Some(id), Some(mapped)) = (&request.group_id, &mapped) {
            if id != mapped {
                return true;
            }
        }
        let Some(group_id) = request.group_id.clone().or(mapped) else {
            return true;
        };
        let Some(group) = engine.group_snapshot(&group_id) else {
            return true;
        };
        let Some(local) = self.logged_in.as_ref().map(|login| login.owner_pubkey) else {
            return true;
        };
        let requester = request.requester.unwrap_or(sender);
        let member = |key: PublicKey| {
            group
                .members
                .iter()
                .any(|member| member.to_bytes() == key.to_bytes())
        };
        if requester == local
            || !member(local)
            || !member(sender)
            || !member(requester)
            || self.is_owner_blocked(&requester.to_hex())
        {
            return true;
        }
        let is_admin = group
            .admins
            .iter()
            .any(|admin| admin.to_bytes() == local.to_bytes());
        // Forward at most once. Only an admin handles a member's request on
        // behalf of someone else, and both identities must be current members.
        if request.requester.is_some() && !is_admin {
            return true;
        }
        let attempt = match self.reserve_group_recovery(&format!("reply:{requester}:{group_id}")) {
            Ok(Some(attempt)) => attempt,
            Ok(None) => return true,
            Err(_) => return false,
        };
        let Some(engine) = self.protocol_engine.as_mut() else {
            return false;
        };
        let result = engine.resend_private_group_context(&group_id, requester, request.author);
        match result {
            Ok(effects) => {
                self.process_protocol_engine_effects(effects);
            }
            Err(error) => {
                self.push_debug_log("group.context.reply", error.to_string());
                return false;
            }
        }
        if !is_admin {
            let admins = group
                .admins
                .iter()
                .filter_map(|admin| PublicKey::from_slice(&admin.to_bytes()).ok())
                .filter(|admin| {
                    *admin != requester && member(*admin) && !self.is_owner_blocked(&admin.to_hex())
                })
                .collect::<Vec<_>>();
            // Rotate on bounded retries instead of broadcasting to every admin.
            let next = admins.get(attempt as usize % admins.len().max(1)).copied();
            if let Some(admin) = next {
                request.group_id = Some(group_id);
                request.requester = Some(requester);
                self.send_group_context_request(admin, &request);
            }
        }
        self.schedule_fast_protocol_retry_if_pending();
        true
    }
}
