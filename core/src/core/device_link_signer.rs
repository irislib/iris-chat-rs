use super::*;
use nostr::EventId;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

mod repair;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeviceLinkInfo {
    pub v: u8,
    pub link_id: String,
    pub approver: String,
    pub device: String,
    pub link_at: u64,
}

pub(super) struct PendingDeviceLinkSigner {
    pub(super) token: String,
    owner: PublicKey,
    client: PublicKey,
    include_history: bool,
    completed: bool,
    deadline: Instant,
    signed: Option<(Event, DeviceLinkInfo, Option<EventId>)>,
    repair: Option<(std::collections::BTreeSet<EventId>, Event)>,
    cancel: oneshot::Sender<()>,
}

impl AppCore {
    pub(super) fn start_device_link_signer(&mut self, input: &str, include_history: bool) {
        let result = (|| -> anyhow::Result<_> {
            let connection = super::remote_signer_uri::parse_device_link_connection(input)?;
            let logged = self
                .logged_in
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Sign in first."))?;
            anyhow::ensure!(
                logged.owner_keys.is_some()
                    && logged.authorization_state == LocalAuthorizationState::Authorized,
                "Only the primary device can manage devices."
            );
            let mut relays = logged.relay_urls.clone();
            for relay in &connection.relays {
                if !relays.contains(relay) {
                    relays.push(relay.clone());
                }
            }
            Ok((connection, logged.owner_pubkey, relays))
        })();
        let (connection, owner, relays) = match result {
            Ok(value) => value,
            Err(_) => {
                self.state.toast = Some("That device link is not valid.".into());
                self.emit_state();
                return;
            }
        };
        self.stop_device_link_signer();
        let token = uuid::Uuid::new_v4().to_string();
        let (cancel, cancelled) = oneshot::channel();
        self.pending_device_link_signer = Some(PendingDeviceLinkSigner {
            token: token.clone(),
            owner,
            client: connection.signer,
            include_history,
            completed: false,
            deadline: Instant::now() + Duration::from_secs(180),
            signed: None,
            repair: None,
            cancel,
        });
        self.state.busy.updating_roster = true;
        self.state.toast = None;
        self.emit_state();
        self.runtime.spawn(super::device_link_signer_transport::run(
            connection,
            owner,
            relays,
            token,
            self.core_sender.clone(),
            cancelled,
        ));
    }

    pub(super) fn stop_device_link_signer(&mut self) {
        if let Some(pending) = self.pending_device_link_signer.take() {
            let _ = pending.cancel.send(());
            self.state.busy.updating_roster = false;
        }
    }

    pub(super) fn finish_device_link_signer(&mut self, token: &str, success: bool) {
        if self
            .pending_device_link_signer
            .as_ref()
            .is_none_or(|pending| pending.token != token)
        {
            return;
        }
        if success && !self.device_link_is_authorized() {
            // Returning a signature is not proof that the new device saved and
            // published it. Keep progress visible until its authorization arrives.
            return;
        }
        let Some(pending) = self.pending_device_link_signer.as_mut() else {
            return;
        };
        if pending.completed && !success {
            return;
        }
        pending.completed |= success;
        self.state.busy.updating_roster = false;
        self.state.toast = Some(
            if success {
                "Device added"
            } else {
                "Could not link device. Try again."
            }
            .into(),
        );
        self.emit_state();
    }

    fn device_link_is_authorized(&self) -> bool {
        self.pending_device_link_signer
            .as_ref()
            .is_some_and(|pending| {
                pending.signed.as_ref().is_some_and(|(signed, info, _)| {
                    self.app_keys
                        .get(&pending.owner.to_hex())
                        .is_some_and(|known| {
                            known.created_at_secs >= signed.created_at.as_secs()
                                && known.devices.iter().any(|device| {
                                    device.identity_pubkey_hex == info.device
                                        && device.created_at_secs == info.link_at
                                })
                        })
                })
            })
    }

    pub(super) fn complete_authorized_device_link(&mut self) {
        if self.device_link_is_authorized() {
            if let Some(pending) = self
                .pending_device_link_signer
                .as_ref()
                .filter(|p| !p.completed)
            {
                let token = pending.token.clone();
                self.finish_device_link_signer(&token, true);
            }
        }
    }

    pub(super) fn sign_device_link_request(
        &mut self,
        token: &str,
        unsigned_json: &str,
        previous: Option<&Event>,
    ) -> anyhow::Result<(Event, DeviceLinkInfo)> {
        let pending = self
            .pending_device_link_signer
            .as_ref()
            .filter(|pending| pending.token == token && pending.deadline > Instant::now())
            .ok_or_else(|| anyhow::anyhow!("Device approval expired."))?;
        let logged = self
            .logged_in
            .as_ref()
            .filter(|logged| {
                logged.owner_pubkey == pending.owner
                    && logged.authorization_state == LocalAuthorizationState::Authorized
            })
            .ok_or_else(|| anyhow::anyhow!("Device approval cancelled."))?;
        let owner_keys = logged
            .owner_keys
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Primary device required."))?;
        anyhow::ensure!(
            unsigned_json.len() <= 32 * 1024,
            "Invalid device authorization."
        );
        let mut value: serde_json::Value = serde_json::from_str(unsigned_json)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid device authorization."))?;
        object
            .entry("pubkey")
            .or_insert_with(|| serde_json::json!(pending.owner.to_hex()));
        let mut draft: UnsignedEvent = serde_json::from_value(value)?;
        let supplied_id = draft.id;
        draft.id = None;
        draft.ensure_id();
        anyhow::ensure!(
            supplied_id.is_none_or(|id| Some(id) == draft.id),
            "Invalid device authorization."
        );
        if let Some((signed, info, baseline_id)) = &pending.signed {
            anyhow::ensure!(
                *baseline_id == previous.map(|event| event.id),
                "Device list changed. Try again."
            );
            anyhow::ensure!(
                Some(signed.id) == draft.id,
                "This link already approved a device."
            );
            return Ok((signed.clone(), info.clone()));
        }
        let (device, link_at) =
            validate_device_link_draft(pending.owner, &draft, previous, unix_now().get())?;
        let baseline = previous
            .map(AppKeys::from_event)
            .transpose()?
            .unwrap_or_else(|| AppKeys::new(Vec::new()));
        anyhow::ensure!(
            baseline
                .get_device(&logged.device_keys.public_key())
                .is_some(),
            "Approving device is not authorized."
        );
        if let Some(known) = self.app_keys.get(&pending.owner.to_hex()) {
            anyhow::ensure!(
                known.created_at_secs <= previous.map_or(0, |event| event.created_at.as_secs()),
                "Device list changed. Try again."
            );
            for known in &known.devices {
                let key = PublicKey::from_hex(&known.identity_pubkey_hex)?;
                anyhow::ensure!(
                    baseline
                        .get_device(&key)
                        .is_some_and(|device| device.created_at == known.created_at_secs),
                    "Device list changed. Try again."
                );
            }
        }
        let signed = draft.sign_with_keys(owner_keys)?;
        let info = DeviceLinkInfo {
            v: 1,
            link_id: pending.client.to_hex(),
            approver: logged.device_keys.public_key().to_hex(),
            device: device.to_hex(),
            link_at,
        };
        let include_history = pending.include_history;
        // Save the private pair before returning the signature. It becomes usable only when
        // the client publishes its authorization and the normal verified roster admits it.
        let previous_known = self.app_keys.insert(
            pending.owner.to_hex(),
            known_app_keys_from_ndr(
                pending.owner,
                &AppKeys::from_event(&signed)?,
                signed.created_at.as_secs(),
            ),
        );
        let stored =
            self.create_device_history_transfer(device, include_history, info.link_id.clone());
        let owner_hex = signed.pubkey.to_hex();
        if let Some(previous_known) = previous_known {
            self.app_keys.insert(owner_hex, previous_known);
        } else {
            self.app_keys.remove(&owner_hex);
        }
        stored?;
        if let Some(pending) = self.pending_device_link_signer.as_mut() {
            pending.signed = Some((signed.clone(), info.clone(), previous.map(|event| event.id)));
        }
        Ok((signed, info))
    }
}

pub(super) fn validate_device_link_draft(
    owner: PublicKey,
    draft: &UnsignedEvent,
    previous: Option<&Event>,
    now: u64,
) -> anyhow::Result<(PublicKey, u64)> {
    anyhow::ensure!(
        draft.pubkey == owner
            && draft.kind.as_u16() == APP_KEYS_EVENT_KIND as u16
            && draft.content.is_empty()
            && draft.created_at.as_secs() <= now.saturating_add(300)
            && draft.created_at.as_secs() > previous.map_or(0, |event| event.created_at.as_secs()),
        "Invalid device authorization."
    );
    if let Some(previous) = previous {
        anyhow::ensure!(previous.pubkey == owner, "Invalid previous device list.");
    }
    let mut baseline = previous
        .map(AppKeys::from_event)
        .transpose()?
        .unwrap_or_else(|| AppKeys::new(Vec::new()));
    let mut added = None;
    for tag in draft
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|name| name == "device"))
    {
        let values = tag.as_slice();
        let [_, key, joined] = values else {
            anyhow::bail!("Invalid device authorization.");
        };
        let key = PublicKey::from_hex(key)?;
        let joined: u64 = joined.parse()?;
        if baseline.get_device(&key).is_none() {
            anyhow::ensure!(
                added.is_none()
                    && joined >= now.saturating_sub(300)
                    && joined <= now.saturating_add(300)
                    && joined <= draft.created_at.as_secs(),
                "Invalid new device."
            );
            added = Some((key, joined));
        }
    }
    let (device, joined) =
        added.ok_or_else(|| anyhow::anyhow!("A device link must add one device."))?;
    baseline.add_device(DeviceEntry::new(device, joined));
    anyhow::ensure!(
        baseline.get_all_devices().len() <= 64,
        "Too many linked devices."
    );
    let profile_tags = draft
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|name| name == "d"))
        .collect::<Vec<_>>();
    let [profile_tag] = profile_tags.as_slice() else {
        anyhow::bail!("Invalid device profile.");
    };
    let [_, profile] = profile_tag.as_slice() else {
        anyhow::bail!("Invalid device profile.");
    };
    uuid::Uuid::parse_str(profile)?;
    // AppKeys generates a fresh subject UUID for every snapshot. Preserve only the
    // validated draft UUID when recomputing; all other tags must match exactly.
    let expected = super::account_signer::canonical_signer_roster(
        &baseline,
        owner,
        draft.created_at.as_secs(),
    );
    let expected_tags = expected
        .tags
        .iter()
        .map(|tag| {
            let mut values = tag.as_slice().to_vec();
            if values
                .first()
                .is_some_and(|name| name == "d" || name == "i")
            {
                if let Some(value) = values.get_mut(1) {
                    *value = profile.clone();
                }
            }
            values
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        draft
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect::<Vec<_>>()
            == expected_tags,
        "Device link cannot change existing devices."
    );
    Ok((device, joined))
}
