//! Encrypted, short-lived call wakeups through the existing notification service.
//! Answers, cancellation, liveness and media use the authenticated FIPS session.
use super::*;
use nostr::{nips::nip44, Tag};

pub(in crate::core) const CALL_WAKE_KIND: u16 = 21_111;
const WAKE_LIFETIME: u64 = 40;

fn wake_event(keys: &Keys, target: PublicKey, signal: &Signal) -> Option<Event> {
    let content = nip44::encrypt(
        keys.secret_key(),
        &target,
        serde_json::to_vec(signal).ok()?,
        nip44::Version::V2,
    )
    .ok()?;
    EventBuilder::new(Kind::from(CALL_WAKE_KIND), content)
        .tags([Tag::public_key(target)])
        .sign_with_keys(keys)
        .ok()
}

fn wake_signal(event: &Event, keys: &Keys) -> Option<Signal> {
    let now = unix_now().get();
    if event.kind.as_u16() != CALL_WAKE_KIND
        || event.verify().is_err()
        || event.created_at.as_secs() > now.saturating_add(5)
        || now.saturating_sub(event.created_at.as_secs()) > WAKE_LIFETIME
        || event.tags.public_keys().copied().collect::<Vec<_>>() != vec![keys.public_key()]
    {
        return None;
    }
    let content = nip44::decrypt(keys.secret_key(), &event.pubkey, &event.content).ok()?;
    let signal = Signal::decode(content.as_bytes())?;
    (signal.kind == "offer").then_some(signal)
}

impl AppCore {
    pub(super) fn send_call_push_wakeups(&self) {
        #[cfg(test)]
        if self.preferences.mobile_push_server_url.is_empty() {
            return;
        }
        let (Some(logged), Some(active)) = (&self.logged_in, &self.calls.active) else {
            return;
        };
        let signal = Signal::new("offer", &active.id, active.offered_video, false);
        let events: Vec<_> = active
            .targets
            .iter()
            .filter_map(|target| {
                wake_event(
                    &logged.device_keys,
                    PublicKey::from_hex(target).ok()?,
                    &signal,
                )
            })
            .collect();
        // The configured service already handles message pushes. Offline FIPS
        // calls don't wait for this best-effort HTTP side effect.
        let server = super::super::mobile_push::resolve_mobile_push_server_url(
            String::new(),
            true,
            Some(self.preferences.mobile_push_server_url.clone()),
        );
        let relay_client = logged.client.clone();
        self.runtime.spawn(async move {
            let Ok(client) = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
            else {
                return;
            };
            for event in events {
                let relay = relay_client.clone();
                let relay_event = event.clone();
                tokio::spawn(async move {
                    let _ = relay.send_event(&relay_event).await;
                });
                let client = client.clone();
                let endpoint = format!("{}/events", server.trim_end_matches('/'));
                tokio::spawn(async move {
                    let _ = client.post(endpoint).json(&event).send().await;
                });
            }
        });
    }

    pub(in crate::core) fn receive_call_push(&mut self, event: &Event) {
        let Some(logged) = &self.logged_in else {
            return;
        };
        let Some(signal) = wake_signal(event, &logged.device_keys) else {
            return;
        };
        let source = event.pubkey.to_hex();
        let Some(owner) = self.call_owner(&source) else {
            return;
        };
        if !self.call_contact_allowed(&owner) {
            return;
        }
        // Reuse the live offer policy, deduplication, history and timeout logic.
        self.handle_call_packet(
            &source,
            PORT,
            &serde_json::to_vec(&signal).unwrap_or_default(),
        );
        if self
            .calls
            .active
            .as_ref()
            .is_some_and(|call| call.id == signal.call_id)
        {
            self.handle_app_foregrounded();
        }
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
    let signal = wake_signal(&event, &Keys::parse(&device_nsec).ok()?)?;
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
    let prefs = &persisted.preferences;
    if prefs.blocked_owner_pubkeys.contains(&owner)
        || !(prefs.accepted_owner_pubkeys.contains(&owner)
            || persisted.threads.iter().any(|thread| {
                thread.chat_id == owner && thread.messages.iter().any(|message| message.is_outgoing)
            }))
    {
        return None;
    }
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
