//! Encrypted, short-lived call wakeups through the existing notification service.
//! Answers, cancellation, liveness and media use the authenticated FIPS session.
use super::*;
use nostr::{EventId, JsonUtil, Tag};

pub(in crate::core) const CALL_WAKE_KIND: u16 = 21_111;
pub(in crate::core) const CALL_OFFER_KIND: u32 = 21_112;
const WAKE_LIFETIME: u64 = 40;
const MAX_WAKE_BYTES: usize = 4096;
mod bootstrap;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallWake {
    #[serde(rename = "type")]
    kind: String,
    v: u8,
    events: Vec<Event>,
    #[serde(
        rename = "bootstrapEventId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    bootstrap_event_id: Option<String>,
}

fn wake_event(keys: &Keys, encrypted: &Event, bootstrap: Option<&Event>) -> Option<Event> {
    let targets = encrypted.tags.public_keys().copied().collect::<Vec<_>>();
    if encrypted.kind.as_u16() as u32 != MESSAGE_EVENT_KIND || targets.len() != 1 {
        return None;
    }
    let target = targets.first().copied()?;
    let sign = |events, bootstrap_event_id| {
        let content = serde_json::to_string(&CallWake {
            kind: "call-wake".into(),
            v: 2,
            events,
            bootstrap_event_id,
        })
        .ok()?;
        let event = EventBuilder::new(Kind::from(CALL_WAKE_KIND), content)
            .tags([Tag::public_key(target)])
            .sign_with_keys(keys)
            .ok()?;
        (event.as_json().len() <= MAX_WAKE_BYTES).then_some(event)
    };
    sign(
        bootstrap
            .into_iter()
            .cloned()
            .chain([encrypted.clone()])
            .collect(),
        None,
    )
    .or_else(|| sign(vec![encrypted.clone()], Some(bootstrap?.id.to_hex())))
}

fn wake_messages(event: &Event, device: PublicKey) -> Option<CallWake> {
    let now = unix_now().get();
    if event.kind.as_u16() != CALL_WAKE_KIND
        || event.verify().is_err()
        || event.as_json().len() > MAX_WAKE_BYTES
        || event.created_at.as_secs() > now.saturating_add(5)
        || now.saturating_sub(event.created_at.as_secs()) > WAKE_LIFETIME
        || event.tags.public_keys().copied().collect::<Vec<_>>() != vec![device]
    {
        return None;
    }
    let wake: CallWake = serde_json::from_str(&event.content).ok()?;
    if wake.kind != "call-wake" || wake.v != 2 || !(1..=2).contains(&wake.events.len()) {
        return None;
    }
    if let Some(id) = &wake.bootstrap_event_id {
        if wake.events.len() != 1 || id.len() != 64 || EventId::from_hex(id).ok()?.to_hex() != *id {
            return None;
        }
    }
    for (index, message) in wake.events.iter().enumerate() {
        let is_message = index + 1 == wake.events.len();
        if message.verify().is_err()
            || (is_message
                && (message.kind.as_u16() as u32 != MESSAGE_EVENT_KIND
                    || message.tags.public_keys().copied().collect::<Vec<_>>() != vec![device]))
            || (!is_message && message.kind.as_u16() as u32 != INVITE_RESPONSE_KIND)
            || message.created_at.as_secs() > now.saturating_add(5)
            || now.saturating_sub(message.created_at.as_secs())
                > if is_message {
                    WAKE_LIFETIME
                } else {
                    2 * 24 * 60 * 60 + WAKE_LIFETIME
                }
        {
            return None;
        }
    }
    Some(wake)
}

fn decrypted_offer(message: &ProtocolDecryptedMessage) -> Option<Signal> {
    let rumor = parse_runtime_rumor(&message.content)?;
    if rumor.kind != CALL_OFFER_KIND
        || (rumor.pubkey != message.sender && Some(rumor.pubkey) != message.sender_device)
        || rumor.created_at_secs > unix_now().get().saturating_add(5)
        || unix_now().get().saturating_sub(rumor.created_at_secs) > WAKE_LIFETIME
    {
        return None;
    }
    let signal = Signal::decode(rumor.content.as_bytes())?;
    (signal.kind == "offer").then_some(signal)
}

impl AppCore {
    pub(super) fn send_call_push_wakeups(&mut self) {
        let (Some(logged), Some(active)) = (&self.logged_in, &self.calls.active) else {
            return;
        };
        let Ok(peer) = PublicKey::from_hex(&active.owner) else {
            return;
        };
        let chat_id = active.owner.clone();
        let signal = Signal::new("offer", &active.id, active.offered_video, false);
        let mut rumor = EventBuilder::new(
            Kind::from(CALL_OFFER_KIND as u16),
            serde_json::to_string(&signal).unwrap_or_default(),
        )
        .tags([
            Tag::public_key(peer),
            Tag::expiration(Timestamp::from(unix_now().get() + WAKE_LIFETIME)),
        ])
        .build(logged.owner_pubkey);
        rumor.ensure_id();
        if let Some(active) = self.calls.active.as_mut() {
            active.push_inner_event_id = rumor.id.map(|id| id.to_hex());
        }
        // The durable protocol outbox also handles a session that becomes ready
        // after this call starts. Its later Publish effect creates the wake.
        self.send_protocol_engine_unsigned_event_to_peer_only(peer, &chat_id, rumor, "call.offer");
        self.schedule_fast_protocol_retry_if_pending();
    }

    pub(in crate::core) fn publish_call_push_for_protocol(&mut self, publish: &ProtocolPublish) {
        let Some(active) = &self.calls.active else {
            return;
        };
        if publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND && active.outgoing {
            let recipient = publish.event.tags.public_keys().next().copied();
            let device = self
                .protocol_engine
                .as_ref()
                .and_then(|engine| {
                    engine
                        .session_manager_snapshot()
                        .users
                        .into_iter()
                        .find(|user| user.owner_pubkey.to_string() == active.owner)
                })
                .and_then(|user| {
                    user.devices.into_iter().find(|device| {
                        device.public_invite.as_ref().is_some_and(|invite| {
                            Some(invite.inviter_ephemeral_public_key.to_string())
                                == recipient.map(|key| key.to_hex())
                        })
                    })
                })
                .map(|device| device.device_pubkey.to_string());
            if let (Some(device), Some(active)) = (device, self.calls.active.as_mut()) {
                active.push_bootstrap.insert(device, publish.event.clone());
            }
            return;
        }
        if !active.outgoing
            || active.started.elapsed().as_secs() > WAKE_LIFETIME
            || active.push_inner_event_id.is_none()
            || active.push_inner_event_id != publish.inner_event_id
        {
            return;
        }
        let Some(target) = publish
            .event
            .tags
            .public_keys()
            .next()
            .map(|key| key.to_hex())
        else {
            return;
        };
        let encrypted = publish.event.clone();
        if let Some(active) = self.calls.active.as_mut() {
            active.push_pending_wakes.insert(target, encrypted);
        }
        self.flush_call_push_wakeups();
    }

    pub(in crate::core) fn call_push_bootstrap_accepted(&mut self, id: &str) {
        if let Some(active) = self.calls.active.as_mut() {
            if active
                .push_bootstrap
                .values()
                .any(|event| event.id.to_hex() == id)
            {
                active.push_bootstrap_acked.insert(id.to_owned());
            }
        }
        self.flush_call_push_wakeups();
    }

    fn flush_call_push_wakeups(&mut self) {
        let (Some(logged), Some(active)) = (&self.logged_in, &mut self.calls.active) else {
            return;
        };
        if !active.outgoing || active.started.elapsed().as_secs() > WAKE_LIFETIME {
            return;
        }
        let mut ready = Vec::new();
        active.push_pending_wakes.retain(|target, encrypted| {
            let Some(event) = wake_event(
                &logged.device_keys,
                encrypted,
                active.push_bootstrap.get(target),
            ) else {
                return false;
            };
            let Ok(wake) = serde_json::from_str::<CallWake>(&event.content) else {
                return false;
            };
            if wake
                .bootstrap_event_id
                .as_ref()
                .is_some_and(|id| !active.push_bootstrap_acked.contains(id))
            {
                return true;
            }
            ready.push(event);
            false
        });
        for event in ready {
            self.publish_call_wake(event);
        }
    }

    fn publish_call_wake(&self, event: Event) {
        let Some(logged) = &self.logged_in else {
            return;
        };
        let server = super::super::mobile_push::resolve_mobile_push_server_url(
            String::new(),
            true,
            Some(self.preferences.mobile_push_server_url.clone()),
        );
        #[cfg(test)]
        if self.preferences.mobile_push_server_url.is_empty() {
            return;
        }
        let relay = logged.client.clone();
        self.runtime.spawn(async move {
            let relay_event = event.clone();
            tokio::spawn(async move {
                let _ = relay.send_event(&relay_event).await;
            });
            if let Ok(client) = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
            {
                let _ = client
                    .post(format!("{}/events", server.trim_end_matches('/')))
                    .json(&event)
                    .send()
                    .await;
            }
        });
    }

    pub(in crate::core) fn receive_call_push(&mut self, event: &Event) {
        let Some(logged) = &self.logged_in else {
            return;
        };
        let local_owner = logged.owner_pubkey;
        let local_keys = logged.device_keys.clone();
        let Some(wake) = wake_messages(event, local_keys.public_key()) else {
            return;
        };
        let Some(owner) = self.call_owner(&event.pubkey.to_hex()) else {
            return;
        };
        if !self.call_contact_allowed(&owner) {
            return;
        }
        let preview = super::super::mobile_push::preview_direct_messages(
            &self.data_dir.to_string_lossy(),
            local_owner,
            &local_keys,
            &wake.events,
        );
        if let Some(message) = preview {
            if message.sender_device != Some(event.pubkey)
                || message.sender.to_hex() != owner
                || decrypted_offer(&message).is_none()
            {
                return;
            }
        }
        // Preserve a valid ciphertext while its normal authenticated handshake is
        // still arriving. Only the decrypted inner offer can ring the phone.
        for encrypted in wake.events {
            self.handle_relay_event(encrypted);
        }
        if let Some(id) = wake
            .bootstrap_event_id
            .and_then(|id| EventId::from_hex(&id).ok())
        {
            let urls = self.preferences.nostr_relay_urls.clone();
            let tx = self.core_sender.clone();
            self.runtime.spawn(async move {
                if let Some(bootstrap) = bootstrap::fetch(id, urls).await {
                    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::RelayEvent(
                        bootstrap,
                    ))));
                }
            });
        }
        self.handle_app_foregrounded();
    }

    pub(in crate::core) fn receive_ratcheted_call_offer(
        &mut self,
        owner: PublicKey,
        device: Option<PublicKey>,
        content: &str,
        created_at: u64,
    ) -> bool {
        let now = unix_now().get();
        if created_at > now.saturating_add(5) || now.saturating_sub(created_at) > WAKE_LIFETIME {
            return true;
        }
        let Some(device) = device else {
            return true;
        };
        if self.call_owner(&device.to_hex()).as_deref() != Some(&owner.to_hex())
            || !self.call_contact_allowed(&owner.to_hex())
        {
            return true;
        }
        let Some(signal) =
            Signal::decode(content.as_bytes()).filter(|signal| signal.kind == "offer")
        else {
            return true;
        };
        self.handle_call_packet(
            &device.to_hex(),
            PORT,
            &serde_json::to_vec(&signal).unwrap_or_default(),
        );
        true
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_call_push_subscription_request(
    owner_nsec: String,
    device_pubkey_hex: String,
    author_pubkeys: Vec<String>,
    subscription_id: Option<String>,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<crate::state::MobilePushSubscriptionRequest> {
    let device = PublicKey::from_hex(&device_pubkey_hex).ok()?;
    let mut authors: Vec<_> = author_pubkeys
        .into_iter()
        .filter_map(|key| PublicKey::from_hex(&key).ok().map(|key| key.to_hex()))
        .collect();
    authors.sort();
    authors.dedup();
    if authors.is_empty() || push_token.trim().is_empty() {
        return None;
    }
    let ios = platform_key == "ios";
    if !ios && platform_key != "android" {
        return None;
    }
    let filter =
        serde_json::json!({"kinds": [CALL_WAKE_KIND], "authors": authors, "#p": [device.to_hex()]});
    let mut body = serde_json::json!({
        "filter": filter, "filters": [],
        "apns_tokens": if ios { vec![push_token.clone()] } else { vec![] },
        "fcm_tokens": if ios { vec![] } else { vec![push_token] },
    });
    if ios {
        let topic = apns_topic.filter(|topic| !topic.trim().is_empty())?;
        let fields = body.as_object_mut()?;
        fields.insert(
            "apns_topic".into(),
            format!("{}.voip", topic.trim_end_matches(".voip")).into(),
        );
        fields.insert(
            "apns_environment".into(),
            if is_release {
                "production"
            } else {
                "development"
            }
            .into(),
        );
    }
    let path = match subscription_id {
        Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => {
            format!("/subscriptions/{id}")
        }
        Some(_) => return None,
        None => "/subscriptions".into(),
    };
    super::super::mobile_push::subscriptions::build_mobile_push_subscription_request(
        owner_nsec,
        "POST",
        &path,
        Some(body.to_string()),
        platform_key,
        is_release,
        server_url_override,
    )
}

pub(crate) fn resolve_call_push_invite(
    data_dir: String,
    device_nsec: String,
    payload_json: String,
) -> Option<CallSnapshot> {
    let event = super::super::mobile_push::mobile_push_event_from_payload(&payload_json)?;
    let keys = Keys::parse(&device_nsec).ok()?;
    let wake = wake_messages(&event, keys.public_key())?;
    // PushKit needs a prompt report, including while the core is starting.
    // Read existing state without running migrations or acquiring a write lock.
    let connection = super::super::mobile_push::open_lookup_connection(&data_dir)?;
    connection.busy_timeout(Duration::from_millis(200)).ok()?;
    let persisted = super::super::storage::AppStore::new(std::sync::Arc::new(
        std::sync::Mutex::new(connection),
    ))
    .load_state()
    .ok()??;
    if matches!(
        persisted.authorization_state,
        Some(PersistedAuthorizationState::AwaitingApproval | PersistedAuthorizationState::Revoked)
    ) {
        return None;
    }
    let owner = persisted
        .app_keys
        .iter()
        .find(|keys| {
            keys.created_at_secs > 0
                && keys
                    .devices
                    .iter()
                    .any(|device| device.identity_pubkey_hex == event.pubkey.to_hex())
        })?
        .owner_pubkey_hex
        .clone();
    let local_owner = persisted
        .app_keys
        .iter()
        .find(|roster| {
            roster
                .devices
                .iter()
                .any(|device| device.identity_pubkey_hex == keys.public_key().to_hex())
        })?
        .owner_pubkey_hex
        .parse::<PublicKey>()
        .ok()?;
    let prefs = &persisted.preferences;
    if prefs.blocked_owner_pubkeys.contains(&owner)
        || !(prefs.accepted_owner_pubkeys.contains(&owner)
            || persisted.threads.iter().any(|thread| {
                thread.chat_id == owner && thread.messages.iter().any(|message| message.is_outgoing)
            }))
    {
        return None;
    }
    if !prefs.video_calls_enabled && !prefs.voice_calls_enabled {
        return None;
    }
    let preview = |messages: &[Event]| {
        super::super::mobile_push::preview_direct_messages(&data_dir, local_owner, &keys, messages)
    };
    let message = preview(&wake.events).or_else(|| {
        let id = EventId::from_hex(wake.bootstrap_event_id.as_ref()?).ok()?;
        let response =
            bootstrap::fetch_blocking(id, persisted.preferences.nostr_relay_urls.clone())?;
        let messages = [vec![response], wake.events.clone()].concat();
        preview(&messages)
    })?;
    if message.sender_device != Some(event.pubkey) || message.sender.to_hex() != owner {
        return None;
    }
    let signal = decrypted_offer(&message)?;
    let video = signal.video.unwrap_or(false) && prefs.video_calls_enabled;
    if !video && !prefs.voice_calls_enabled {
        return None;
    }
    let profile = persisted.owner_profiles.get(&owner);
    let name = profile
        .and_then(|p| {
            p.nickname
                .as_ref()
                .or(p.display_name.as_ref())
                .or(p.name.as_ref())
        })
        .cloned()
        .unwrap_or_else(|| "Iris contact".into());
    Some(CallSnapshot {
        outgoing: false,
        target_bitrate_bps: 1_200_000,
        key_frame_generation: 0,
        media_connected: false,
        max_bitrate_bps: 2_000_000,
        call_id: signal.call_id,
        chat_id: owner,
        peer_name: name,
        phase: "incoming".into(),
        video,
        video_capable: video,
        muted: false,
        remote_video: video,
        remote_muted: false,
        started_at_secs: event.created_at.as_secs(),
        connected_at_secs: None,
        end_reason: None,
    })
}

#[cfg(test)]
mod tests;
