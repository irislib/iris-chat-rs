use super::invites::parse_public_invite_input;
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
        let current_app_keys = current.as_ref().and_then(known_app_keys_to_ndr);
        let current_created_at = current
            .as_ref()
            .map(|known| known.created_at_secs)
            .unwrap_or_default();
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
        let applied = apply_app_keys_snapshot_with_required_device(
            current_app_keys.as_ref(),
            current_created_at,
            incoming_app_keys,
            incoming_created_at,
            required_device,
        );
        let known = known_app_keys_from_ndr(owner, &applied.app_keys, applied.created_at);
        if current.as_ref() == Some(&known) {
            return None;
        }
        self.app_keys.insert(owner_hex, known);
        Some((applied.app_keys, applied.created_at))
    }
}

pub(super) fn parse_link_device_invite_input(
    input: &str,
    owner_pubkey: PublicKey,
) -> anyhow::Result<Invite> {
    let invite = parse_public_invite_input(input)?;
    if invite.purpose.as_deref() != Some("link") {
        return Err(anyhow::anyhow!("Invalid link code."));
    }
    if invite
        .owner_public_key
        .is_some_and(|invite_owner| invite_owner != owner_pubkey)
    {
        return Err(anyhow::anyhow!("This code is for a different profile."));
    }
    Ok(invite)
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
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(truncate_chars(trimmed, 160))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    out
}

pub(super) fn known_app_keys_to_ndr(known: &KnownAppKeys) -> Option<AppKeys> {
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
    Some(app_keys)
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

pub(super) fn preserve_known_app_key_labels(
    known: &mut KnownAppKeys,
    current: Option<&KnownAppKeys>,
) {
    let Some(current) = current else {
        return;
    };
    for device in &mut known.devices {
        let Some(current_device) = current
            .devices
            .iter()
            .find(|candidate| candidate.identity_pubkey_hex == device.identity_pubkey_hex)
        else {
            continue;
        };
        if device.device_label.is_none() {
            device.device_label = current_device.device_label.clone();
        }
        if device.client_label.is_none() {
            device.client_label = current_device.client_label.clone();
        }
        if device.label_updated_at_secs == 0 {
            device.label_updated_at_secs = current_device.label_updated_at_secs;
        }
    }
}
