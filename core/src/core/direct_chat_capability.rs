use super::protocol::fetch_events_for_filters;
use super::*;

const DIRECT_CAPABILITY_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DIRECT_CAPABILITY_EVENTS: usize = 32;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CurrentAppKeysResolution {
    Found {
        event: Box<Event>,
        has_devices: bool,
    },
    Missing,
    Ambiguous,
}

impl AppCore {
    pub(super) fn chat_capability(
        &self,
        chat_id: &str,
        kind: &ChatKind,
    ) -> Option<DirectChatCapabilityState> {
        matches!(kind, ChatKind::Direct).then(|| self.direct_chat_capability_state(chat_id))
    }

    pub(super) fn direct_chat_capability_state(
        &self,
        owner_pubkey_hex: &str,
    ) -> DirectChatCapabilityState {
        let runtime_state = self
            .direct_chat_capability_runtime
            .current
            .as_ref()
            .filter(|check| check.owner_pubkey_hex == owner_pubkey_hex)
            .map(|check| check.state);
        if runtime_state == Some(DirectChatCapabilityCheckState::Checking) {
            return DirectChatCapabilityState::Checking;
        }
        if self
            .app_keys
            .get(owner_pubkey_hex)
            .is_some_and(|known| !known.devices.is_empty())
        {
            return DirectChatCapabilityState::Available;
        }
        match runtime_state {
            Some(DirectChatCapabilityCheckState::CheckFailed) => {
                DirectChatCapabilityState::CheckFailed
            }
            Some(DirectChatCapabilityCheckState::Unavailable) => {
                DirectChatCapabilityState::Unavailable
            }
            Some(DirectChatCapabilityCheckState::Checking) => unreachable!(),
            None if self.app_keys.contains_key(owner_pubkey_hex) => {
                DirectChatCapabilityState::Unavailable
            }
            None => DirectChatCapabilityState::Checking,
        }
    }

    pub(super) fn request_direct_chat_capability_check(
        &mut self,
        owner_pubkey_hex: &str,
        force: bool,
    ) -> bool {
        let Ok(owner) = PublicKey::parse(owner_pubkey_hex) else {
            return false;
        };
        if self
            .app_keys
            .get(owner_pubkey_hex)
            .is_some_and(|known| !known.devices.is_empty())
        {
            self.direct_chat_capability_runtime.current = None;
            return false;
        }
        if !force
            && self
                .direct_chat_capability_runtime
                .current
                .as_ref()
                .filter(|check| check.owner_pubkey_hex == owner_pubkey_hex)
                .is_some_and(|check| {
                    check.state == DirectChatCapabilityCheckState::Checking
                        || check.state == DirectChatCapabilityCheckState::Unavailable
                })
        {
            return false;
        }

        self.direct_chat_capability_runtime.next_token = self
            .direct_chat_capability_runtime
            .next_token
            .wrapping_add(1)
            .max(1);
        let token = self.direct_chat_capability_runtime.next_token;
        let generation = self.direct_chat_capability_runtime.generation;
        self.direct_chat_capability_runtime.current = Some(DirectChatCapabilityCheck {
            token,
            owner_pubkey_hex: owner_pubkey_hex.to_string(),
            state: DirectChatCapabilityCheckState::Checking,
        });

        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .filter(|logged_in| !logged_in.relay_urls.is_empty())
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            if let Some(check) = self.direct_chat_capability_runtime.current.as_mut() {
                check.state = DirectChatCapabilityCheckState::CheckFailed;
            }
            return true;
        };

        let tx = self.core_sender.clone();
        let owner_pubkey_hex = owner_pubkey_hex.to_string();
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, DIRECT_CAPABILITY_FETCH_TIMEOUT).await;
            let filter = Filter::new()
                .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
                .author(owner)
                .limit(MAX_DIRECT_CAPABILITY_EVENTS);
            let result =
                fetch_events_for_filters(&client, vec![filter], DIRECT_CAPABILITY_FETCH_TIMEOUT)
                    .await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::DirectChatCapabilityFetchFinished {
                    generation,
                    token,
                    owner_pubkey_hex,
                    result,
                },
            )));
        });
        true
    }

    pub(super) fn retry_direct_chat_capability(&mut self, chat_id: &str) {
        let Some(chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        if is_group_chat_id(&chat_id) {
            return;
        }
        if self.request_direct_chat_capability_check(&chat_id, true) {
            self.rebuild_state();
            self.emit_state();
        }
    }

    pub(super) fn handle_direct_chat_capability_fetch_finished(
        &mut self,
        generation: u64,
        token: u64,
        owner_pubkey_hex: &str,
        result: Result<Vec<Event>, String>,
    ) {
        let Some(check) = self
            .direct_chat_capability_runtime
            .current
            .as_ref()
            .filter(|check| check.owner_pubkey_hex == owner_pubkey_hex)
        else {
            return;
        };
        if generation != self.direct_chat_capability_runtime.generation || check.token != token {
            return;
        }

        let Ok(owner) = PublicKey::parse(owner_pubkey_hex) else {
            return;
        };
        match result {
            Err(error) => {
                self.push_debug_log(
                    "chat.capability.fetch.error",
                    format!("owner={owner_pubkey_hex} error={error}"),
                );
                if let Some(check) = self.direct_chat_capability_runtime.current.as_mut() {
                    check.state = DirectChatCapabilityCheckState::CheckFailed;
                }
            }
            Ok(events) => match resolve_current_app_keys(events, owner, unix_now().get()) {
                CurrentAppKeysResolution::Found { event, has_devices } => {
                    if let Some(check) = self.direct_chat_capability_runtime.current.as_mut() {
                        check.state = if has_devices {
                            DirectChatCapabilityCheckState::Checking
                        } else {
                            DirectChatCapabilityCheckState::Unavailable
                        };
                    }
                    self.handle_relay_event(*event);
                    if self
                        .app_keys
                        .get(owner_pubkey_hex)
                        .is_some_and(|known| !known.devices.is_empty())
                    {
                        self.direct_chat_capability_runtime.current = None;
                    }
                }
                CurrentAppKeysResolution::Missing | CurrentAppKeysResolution::Ambiguous => {
                    if let Some(check) = self.direct_chat_capability_runtime.current.as_mut() {
                        check.state = DirectChatCapabilityCheckState::Unavailable;
                    }
                }
            },
        }
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn reset_direct_chat_capability_runtime(&mut self) {
        self.direct_chat_capability_runtime.generation = self
            .direct_chat_capability_runtime
            .generation
            .wrapping_add(1);
        self.direct_chat_capability_runtime.current = None;
    }
}

pub(super) fn resolve_current_app_keys(
    events: Vec<Event>,
    owner: PublicKey,
    now_secs: u64,
) -> CurrentAppKeysResolution {
    let mut acceptable = events
        .into_iter()
        .filter_map(|event| {
            app_keys_event_is_acceptable_for_owner(owner, &event, now_secs)
                .then(|| AppKeys::from_event(&event).ok().map(|keys| (event, keys)))
                .flatten()
        })
        .collect::<Vec<_>>();
    let Some(current_created_at) = acceptable.iter().map(|(event, _)| event.created_at).max()
    else {
        return CurrentAppKeysResolution::Missing;
    };
    acceptable.retain(|(event, _)| event.created_at == current_created_at);
    acceptable.sort_by_key(|(event, _)| event.id);

    let roster = |app_keys: &AppKeys| {
        app_keys
            .get_all_devices()
            .into_iter()
            .map(|device| device.identity_pubkey)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let Some((selected, selected_keys)) = acceptable.first() else {
        return CurrentAppKeysResolution::Missing;
    };
    let selected_roster = roster(selected_keys);
    if acceptable
        .iter()
        .skip(1)
        .any(|(_, app_keys)| roster(app_keys) != selected_roster)
    {
        return CurrentAppKeysResolution::Ambiguous;
    }
    CurrentAppKeysResolution::Found {
        event: Box::new(selected.clone()),
        has_devices: !selected_roster.is_empty(),
    }
}
