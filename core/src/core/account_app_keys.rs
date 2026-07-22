use super::*;

impl AppCore {
    #[cfg(test)]
    pub(super) fn apply_known_app_keys_snapshot(
        &mut self,
        owner: PublicKey,
        incoming_app_keys: &AppKeys,
        incoming_created_at: u64,
    ) -> Option<(AppKeys, u64)> {
        let owner_hex = owner.to_hex();
        let current = self.app_keys.get(&owner_hex).cloned();
        let required_device = self
            .logged_in
            .as_ref()
            .filter(|logged_in| {
                self.defer_owner_app_keys_publish
                    && logged_in.owner_keys.is_some()
                    && logged_in.owner_pubkey == owner
            })
            .map(|logged_in| {
                DeviceEntry::new(logged_in.device_keys.public_key(), unix_now().get())
            });
        let (app_keys, known) = canonical_known_app_keys_snapshot(
            current.as_ref(),
            owner,
            incoming_app_keys,
            incoming_created_at,
            required_device,
        );
        if current.as_ref() == Some(&known) {
            return None;
        }
        let created_at = known.created_at_secs;
        self.app_keys.insert(owner_hex, known);
        Some((app_keys, created_at))
    }
}

pub(super) fn canonical_known_app_keys_snapshot(
    current: Option<&KnownAppKeys>,
    owner: PublicKey,
    incoming: &AppKeys,
    incoming_created_at: u64,
    required_device: Option<DeviceEntry>,
) -> (AppKeys, KnownAppKeys) {
    let current_app_keys = current.map(known_app_keys_to_ndr);
    let applied = apply_app_keys_snapshot_with_required_device(
        current_app_keys.as_ref(),
        current.map_or(0, |known| known.created_at_secs),
        incoming,
        incoming_created_at,
        required_device,
    );
    let known = known_app_keys_from_ndr(owner, &applied.app_keys, applied.created_at);
    (applied.app_keys, known)
}

pub(super) fn preserve_known_app_key_labels(
    current: Option<&KnownAppKeys>,
    incoming: &mut AppKeys,
) {
    let Some(current) = current else {
        return;
    };
    for device in &current.devices {
        if device.device_label.is_none() && device.client_label.is_none() {
            continue;
        }
        let Ok(pubkey) = PublicKey::parse(&device.identity_pubkey_hex) else {
            continue;
        };
        if incoming.get_device(&pubkey).is_some() {
            incoming.set_device_labels(
                pubkey,
                device.device_label.clone(),
                device.client_label.clone(),
                Some(device.label_updated_at_secs),
            );
        }
    }
}

pub(super) fn next_app_keys_created_at(now: u64, current: u64) -> u64 {
    if now <= current {
        current.saturating_add(1)
    } else {
        now
    }
}

pub(super) fn next_removed_app_keys_created_at(now: u64, current: u64, latest_device: u64) -> u64 {
    now.max(current).max(latest_device).saturating_add(2)
}

pub(super) fn normalize_device_label(label: &str) -> Option<String> {
    let normalized = label
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return None;
    }
    Some(normalized.chars().take(160).collect())
}

pub(super) fn known_app_keys_to_ndr(known: &KnownAppKeys) -> AppKeys {
    let mut app_keys = AppKeys::new(
        known
            .devices
            .iter()
            .filter_map(|device| {
                PublicKey::parse(&device.identity_pubkey_hex)
                    .ok()
                    .map(|pubkey| DeviceEntry::new(pubkey, device.created_at_secs))
            })
            .collect(),
    );
    for device in &known.devices {
        if device.device_label.is_none() && device.client_label.is_none() {
            continue;
        }
        let Ok(pubkey) = PublicKey::parse(&device.identity_pubkey_hex) else {
            continue;
        };
        app_keys.set_device_labels(
            pubkey,
            device.device_label.clone(),
            device.client_label.clone(),
            Some(device.label_updated_at_secs),
        );
    }
    app_keys
}

pub(super) fn known_app_keys_from_ndr(
    owner: PublicKey,
    app_keys: &AppKeys,
    created_at_secs: u64,
) -> KnownAppKeys {
    let mut devices = app_keys
        .get_all_devices()
        .into_iter()
        .map(|device| KnownAppKeyDevice {
            identity_pubkey_hex: device.identity_pubkey.to_hex(),
            created_at_secs: device.created_at,
            device_label: app_keys
                .get_device_labels(&device.identity_pubkey)
                .and_then(|labels| labels.device_label.clone()),
            client_label: app_keys
                .get_device_labels(&device.identity_pubkey)
                .and_then(|labels| labels.client_label.clone()),
            label_updated_at_secs: app_keys
                .get_device_labels(&device.identity_pubkey)
                .map(|labels| labels.updated_at)
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    devices.sort_by(|left, right| left.identity_pubkey_hex.cmp(&right.identity_pubkey_hex));
    KnownAppKeys {
        owner_pubkey_hex: owner.to_hex(),
        created_at_secs,
        devices,
    }
}
