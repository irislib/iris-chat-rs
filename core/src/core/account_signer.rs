use super::account_signer_relay::{fetch_signer_roster, publish_signer_authorization};
use super::*;
use nostr_double_ratchet::APP_KEYS_ENCRYPTED_DEVICE_LABELS_FACT;

const SIGNER_LOGIN_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_SIGNER_EVENT_BYTES: usize = 64 * 1024;

pub(super) struct PendingSignerLogin {
    request_id: String,
    owner: PublicKey,
    device_keys: Keys,
    relay_urls: Vec<RelayUrl>,
    previous_event: Option<Event>,
    unsigned_event: Option<UnsignedEvent>,
    publishing: bool,
    deadline: Instant,
}

impl AppCore {
    pub(super) fn begin_signer_login(&mut self, owner_pubkey_hex: &str) {
        if self.logged_in.is_some() {
            return;
        }
        self.pending_signer_login = None;
        let Ok(owner) = PublicKey::parse(owner_pubkey_hex.trim()) else {
            self.fail_signer_login("Invalid user ID.");
            return;
        };
        let relay_urls = relay_urls_from_strings(&self.preferences.nostr_relay_urls);
        if relay_urls.is_empty() {
            self.fail_signer_login("Add a message server before signing in.");
            return;
        }
        self.stop_pending_linked_device();
        let request_id = uuid::Uuid::new_v4().to_string();
        self.pending_signer_login = Some(PendingSignerLogin {
            request_id: request_id.clone(),
            owner,
            device_keys: Keys::generate(),
            relay_urls: relay_urls.clone(),
            previous_event: None,
            unsigned_event: None,
            publishing: false,
            deadline: Instant::now() + SIGNER_LOGIN_TIMEOUT,
        });
        self.state.busy.restoring_session = true;
        self.emit_state();
        let tx = self.core_sender.clone();
        let timeout_tx = tx.clone();
        let timeout_id = request_id.clone();
        self.runtime.spawn(async move {
            let result = fetch_signer_roster(owner, &relay_urls).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::SignerLoginFetched { request_id, result },
            )));
        });
        self.runtime.spawn(async move {
            sleep(SIGNER_LOGIN_TIMEOUT).await;
            let _ = timeout_tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::SignerLoginTimedOut {
                    request_id: timeout_id,
                },
            )));
        });
    }

    pub(super) fn handle_signer_login_fetched(
        &mut self,
        request_id: &str,
        result: Result<Option<Event>, String>,
    ) {
        let Some(pending) = self
            .pending_signer_login
            .as_mut()
            .filter(|pending| pending.request_id == request_id && pending.unsigned_event.is_none())
        else {
            return;
        };
        if pending.deadline <= Instant::now() {
            self.fail_signer_login("Sign-in timed out. Try again.");
            return;
        }
        let prepared = result.and_then(|event| {
            let unsigned = prepare_signer_authorization(
                pending.owner,
                pending.device_keys.public_key(),
                event.as_ref(),
                unix_now().get(),
            )
            .map_err(|error| error.to_string())?;
            pending.previous_event = event;
            pending.unsigned_event = Some(unsigned.clone());
            Ok(unsigned)
        });
        match prepared {
            Ok(unsigned) => {
                let _ = self.update_tx.send(AppUpdate::SignerLoginSignEvent {
                    request_id: request_id.to_string(),
                    owner_pubkey_hex: pending.owner.to_hex(),
                    unsigned_event_json: serde_json::to_string(&unsigned)
                        .expect("serialize unsigned authorization"),
                });
            }
            Err(error) => self.fail_signer_login(&error),
        }
    }

    pub(super) fn complete_signer_login(&mut self, request_id: &str, signed_event_json: &str) {
        let Some(pending) = self
            .pending_signer_login
            .as_mut()
            .filter(|pending| pending.request_id == request_id && !pending.publishing)
        else {
            return;
        };
        if pending.deadline <= Instant::now() {
            self.fail_signer_login("Sign-in timed out. Try again.");
            return;
        }
        let result = (|| -> anyhow::Result<Event> {
            anyhow::ensure!(
                signed_event_json.len() <= MAX_SIGNER_EVENT_BYTES,
                "Invalid signer response."
            );
            let expected = pending
                .unsigned_event
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Unexpected signer response."))?;
            let event: Event = serde_json::from_str(signed_event_json)
                .map_err(|_| anyhow::anyhow!("Invalid signer response."))?;
            validate_signer_authorization(expected, &event)?;
            Ok(event)
        })();
        let event = match result {
            Ok(event) => event,
            Err(error) => {
                self.fail_signer_login(&error.to_string());
                return;
            }
        };
        pending.publishing = true;
        let owner = pending.owner;
        let relay_urls = pending.relay_urls.clone();
        let previous_event = pending.previous_event.clone();
        let request_id = request_id.to_string();
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            let result =
                publish_signer_authorization(owner, relay_urls, previous_event.as_ref(), event)
                    .await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::SignerLoginPublished { request_id, result },
            )));
        });
    }

    pub(super) fn handle_signer_login_published(
        &mut self,
        request_id: &str,
        result: Result<Event, String>,
    ) {
        if !self
            .pending_signer_login
            .as_ref()
            .is_some_and(|pending| pending.request_id == request_id && pending.publishing)
        {
            return;
        }
        let pending = self
            .pending_signer_login
            .take()
            .expect("matched pending signer");
        self.enter_batch();
        let result = result.map_err(anyhow::Error::msg).and_then(|event| {
            self.start_session_inner(
                pending.owner,
                None,
                pending.device_keys.clone(),
                false,
                false,
                false,
            )?;
            self.apply_app_keys_event(&event)?;
            anyhow::ensure!(
                self.logged_in
                    .as_ref()
                    .is_some_and(|logged_in| logged_in.authorization_state
                        == LocalAuthorizationState::Authorized),
                "Device authorization failed."
            );
            // Exact signed proof is persisted by protocol ingestion before storing
            // the account's device secret. No identity secret enters this account.
            self.persist_best_effort_inner();
            anyhow::ensure!(
                self.app_store.has_app_key_device(
                    &pending.owner.to_hex(),
                    &pending.device_keys.public_key().to_hex()
                )?,
                "Could not save device authorization."
            );
            self.emit_account_bundle_update(None, &pending.device_keys);
            self.publish_runtime_event(event, "app-keys", None);
            self.fetch_missing_profile_metadata(&pending.owner.to_hex(), "signer_login");
            self.request_protocol_subscription_refresh_forced();
            self.fetch_recent_protocol_state();
            self.reconcile_device_sync();
            Ok(())
        });
        if let Err(error) = result {
            if self.logged_in.is_some() {
                self.logout();
            }
            self.state.toast = Some(error.to_string());
        }
        self.state.busy.restoring_session = false;
        self.rebuild_state();
        self.emit_state();
        self.exit_batch();
    }

    pub(super) fn cancel_signer_login(&mut self, request_id: &str) {
        if self.pending_signer_login.as_ref().is_some_and(|pending| {
            !pending.publishing && (request_id.is_empty() || pending.request_id == request_id)
        }) {
            self.pending_signer_login = None;
            self.state.busy.restoring_session = false;
            self.emit_state();
        }
    }

    pub(super) fn handle_signer_login_timeout(&mut self, request_id: &str) {
        if self
            .pending_signer_login
            .as_ref()
            .is_some_and(|pending| pending.request_id == request_id && !pending.publishing)
        {
            self.fail_signer_login("Sign-in timed out. Try again.");
        }
    }

    pub(super) fn expire_pending_signer_login(&mut self) {
        if self
            .pending_signer_login
            .as_ref()
            .is_some_and(|pending| !pending.publishing && pending.deadline <= Instant::now())
        {
            self.fail_signer_login("Sign-in timed out. Try again.");
        }
    }

    fn fail_signer_login(&mut self, message: &str) {
        self.pending_signer_login = None;
        self.state.busy.restoring_session = false;
        self.state.toast = Some(message.to_string());
        self.emit_state();
    }

    pub(super) fn signed_local_device_authorization(
        &self,
        owner: PublicKey,
        device: PublicKey,
    ) -> Option<bool> {
        if self.logged_in.as_ref().is_some_and(|logged_in| {
            logged_in.owner_pubkey != owner || logged_in.device_keys.public_key() != device
        }) {
            return None;
        }
        self.protocol_engine
            .as_ref()?
            .signed_local_device_authorization()
    }
}

pub(super) fn prepare_signer_authorization(
    owner: PublicKey,
    device: PublicKey,
    previous: Option<&Event>,
    now: u64,
) -> anyhow::Result<UnsignedEvent> {
    let mut app_keys = previous
        .map(AppKeys::from_event)
        .transpose()?
        .unwrap_or_else(|| AppKeys::new(Vec::new()));
    let created_at = super::account_app_keys::next_app_keys_created_at(
        now,
        previous.map_or(0, |event| event.created_at.as_secs()),
    );
    anyhow::ensure!(
        created_at <= now.saturating_add(300),
        "Device list is dated too far ahead. Try again later."
    );
    app_keys.add_device(DeviceEntry::new(device, now));
    anyhow::ensure!(
        app_keys.get_all_devices().len() <= 64,
        "Too many linked devices."
    );
    let mut unsigned = app_keys.get_event_at(owner, created_at);
    if let Some(previous) = previous {
        // Labels are identity-encrypted. Carry them unchanged so a one-time
        // signing approval never erases existing names or needs decryption.
        unsigned.tags.extend(
            previous
                .tags
                .iter()
                .filter(|tag| {
                    tag.as_slice()
                        .first()
                        .is_some_and(|name| name == APP_KEYS_ENCRYPTED_DEVICE_LABELS_FACT)
                })
                .cloned(),
        );
    }
    unsigned.id = None;
    unsigned.ensure_id();
    anyhow::ensure!(
        serde_json::to_vec(&unsigned)?.len() < MAX_SIGNER_EVENT_BYTES - 256,
        "Device list is too large."
    );
    Ok(unsigned)
}

pub(super) fn validate_signer_authorization(
    expected: &UnsignedEvent,
    event: &Event,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        Some(event.id) == expected.id
            && event.pubkey == expected.pubkey
            && event.kind == expected.kind
            && event.created_at == expected.created_at
            && event.tags == expected.tags
            && event.content == expected.content,
        "Signer changed the device authorization. Try again."
    );
    event
        .verify()
        .map_err(|_| anyhow::anyhow!("Invalid signer signature."))?;
    AppKeys::from_event(event).map_err(|_| anyhow::anyhow!("Invalid device authorization."))?;
    Ok(())
}
