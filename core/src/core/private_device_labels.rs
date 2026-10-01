use super::*;

pub(super) const DEVICE_LABEL_CONTROL_KIND: u32 = 10453;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PrivateDeviceLabel {
    #[serde(rename = "type")]
    kind: String,
    v: u8,
    owner: String,
    device: String,
    #[serde(deserialize_with = "required_label")]
    device_label: Option<String>,
    #[serde(deserialize_with = "required_label")]
    client_label: Option<String>,
    updated_at_secs: u64,
}

fn required_label<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

fn label_is_valid(label: &Option<String>) -> bool {
    label
        .as_ref()
        .is_none_or(|label| label.chars().count() <= 128 && !label.chars().any(char::is_control))
}

impl AppCore {
    pub(super) fn private_device_label_snapshot(&self) -> Vec<PrivateDeviceLabel> {
        let Some(owner) = self
            .logged_in
            .as_ref()
            .map(|login| login.owner_pubkey.to_hex())
        else {
            return Vec::new();
        };
        self.app_keys
            .get(&owner)
            .into_iter()
            .flat_map(|roster| &roster.devices)
            .filter(|device| device.label_updated_at_secs > 0)
            .map(|device| PrivateDeviceLabel {
                kind: "device-labels".into(),
                v: 2,
                owner: owner.clone(),
                device: device.identity_pubkey_hex.clone(),
                device_label: device
                    .device_label
                    .as_ref()
                    .map(|value| value.chars().take(128).collect()),
                client_label: device
                    .client_label
                    .as_ref()
                    .map(|value| value.chars().take(128).collect()),
                updated_at_secs: device.label_updated_at_secs,
            })
            .collect()
    }

    pub(super) fn broadcast_private_device_labels(&mut self) {
        let Some(owner) = self.logged_in.as_ref().map(|login| login.owner_pubkey) else {
            return;
        };
        for label in self.private_device_label_snapshot() {
            if self.private_contacts.sent_device_labels.contains(&label) {
                continue;
            }
            let Ok(content) = serde_json::to_string(&label) else {
                continue;
            };
            let unsigned = EventBuilder::new(Kind::from(DEVICE_LABEL_CONTROL_KIND as u16), content)
                .tag(nostr::Tag::public_key(owner))
                .build(owner);
            if self.send_protocol_engine_unsigned_event_to_local_siblings(
                owner,
                &owner.to_hex(),
                unsigned,
                "device_labels.self_sync_v2",
            ) {
                self.private_contacts
                    .sent_device_labels
                    .retain(|old| old.device != label.device);
                self.private_contacts.sent_device_labels.push(label);
            }
        }
    }

    pub(super) fn replay_private_device_labels(&mut self) {
        self.private_contacts.sent_device_labels.clear();
        self.broadcast_private_device_labels();
    }

    pub(super) fn merge_private_device_label(&mut self, label: PrivateDeviceLabel) -> bool {
        let Some(owner) = self
            .logged_in
            .as_ref()
            .map(|login| login.owner_pubkey.to_hex())
        else {
            return false;
        };
        if label.kind != "device-labels"
            || label.v != 2
            || label.owner != owner
            || PublicKey::from_hex(&label.device).is_err()
            || label.updated_at_secs == 0
            || label.updated_at_secs > unix_now().get().saturating_add(300)
            || !label_is_valid(&label.device_label)
            || !label_is_valid(&label.client_label)
        {
            return false;
        }
        let Some(device) = self.app_keys.get_mut(&owner).and_then(|roster| {
            roster
                .devices
                .iter_mut()
                .find(|device| device.identity_pubkey_hex == label.device)
        }) else {
            return false;
        };
        if (
            label.updated_at_secs,
            &label.device_label,
            &label.client_label,
        ) <= (
            device.label_updated_at_secs,
            &device.device_label,
            &device.client_label,
        ) {
            return false;
        }
        device.device_label = label.device_label;
        device.client_label = label.client_label;
        device.label_updated_at_secs = label.updated_at_secs;
        true
    }

    pub(super) fn receive_private_device_label_control(
        &mut self,
        owner: PublicKey,
        device: Option<PublicKey>,
        content: &str,
    ) -> bool {
        if content.len() > 2048
            || self
                .logged_in
                .as_ref()
                .is_none_or(|login| login.owner_pubkey != owner)
            || !device.is_some_and(|device| self.device_sync_peer_is_authorized(&device.to_hex()))
        {
            return true;
        }
        let Ok(label) = serde_json::from_str::<PrivateDeviceLabel>(content) else {
            return true;
        };
        if self.merge_private_device_label(label) {
            self.rebuild_persist_and_emit_state();
        }
        true
    }

    pub(super) fn replay_private_chat_settings(&mut self) {
        let Some(owner) = self.logged_in.as_ref().map(|login| login.owner_pubkey) else {
            return;
        };
        let controls = self
            .chat_mute_snapshot()
            .into_iter()
            .map(|mute| {
                (
                    chat_mute_sync::CHAT_MUTE_KIND,
                    serde_json::json!({"type":"chat-mute","v":1,"mute":mute}),
                )
            })
            .chain(self.chat_pin_snapshot().into_iter().map(|pin| {
                (
                    chat_pin_sync::CHAT_PIN_KIND,
                    serde_json::json!({"type":"chat-pin","v":1,"pin":pin}),
                )
            }))
            .collect::<Vec<_>>();
        for (kind, content) in controls {
            let unsigned = EventBuilder::new(Kind::from(kind as u16), content.to_string())
                .tag(nostr::Tag::public_key(owner))
                .build(owner);
            self.send_protocol_engine_unsigned_event_to_local_siblings(
                owner,
                &owner.to_hex(),
                unsigned,
                "private_settings.replay",
            );
        }
    }
}

// Preserve an old signed authorization intent locally until its exact public
// membership replacement is acknowledged, but never retransmit private labels.
pub(super) fn obsolete_private_app_keys_event(event: &Event) -> bool {
    is_app_keys_event(event)
        && event.tags.iter().any(|tag| {
            tag.as_slice().first().is_some_and(|name| {
                name == nostr_double_ratchet::APP_KEYS_ENCRYPTED_DEVICE_LABELS_FACT
            })
        })
}

impl AppCore {
    pub(super) fn retire_private_label_publications_after_ack(&mut self, replacement: &Event) {
        if !is_app_keys_event(replacement) || obsolete_private_app_keys_event(replacement) {
            return;
        }
        let members = |event: &Event| {
            AppKeys::from_event(event).ok().map(|keys| {
                let mut entries = keys
                    .get_all_devices()
                    .into_iter()
                    .map(|device| (device.identity_pubkey.to_hex(), device.created_at))
                    .collect::<Vec<_>>();
                entries.sort();
                entries
            })
        };
        let Some(expected) = members(replacement) else {
            return;
        };
        let Ok(pending) = self
            .app_store
            .load_pending_relay_publishes(&replacement.pubkey.to_hex())
        else {
            return;
        };
        for item in pending {
            let Ok(event) = serde_json::from_str::<Event>(&item.event_json) else {
                continue;
            };
            if obsolete_private_app_keys_event(&event)
                && event.pubkey == replacement.pubkey
                && event.created_at <= replacement.created_at
                && members(&event).as_ref() == Some(&expected)
            {
                self.forget_pending_relay_publish(&item.event_id);
            }
        }
    }
}
