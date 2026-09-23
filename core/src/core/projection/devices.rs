use super::*;

impl AppCore {
    pub(in crate::core) fn build_device_roster_snapshot(&self) -> Option<DeviceRosterSnapshot> {
        let logged_in = self.logged_in.as_ref()?;
        let account = self.build_account_snapshot()?;
        let current_device_pubkey_hex = account.device_public_key_hex.clone();
        let current_device_npub = account.device_npub.clone();
        let mut entries = BTreeMap::<String, DeviceEntrySnapshot>::new();

        if let Some(app_keys) = self.app_keys.get(&logged_in.owner_pubkey.to_hex()) {
            for device in &app_keys.devices {
                let device_pubkey_hex = device.identity_pubkey_hex.clone();
                entries.insert(
                    device_pubkey_hex.clone(),
                    DeviceEntrySnapshot {
                        device_pubkey_hex: device_pubkey_hex.clone(),
                        device_npub: device_npub(&device_pubkey_hex)
                            .unwrap_or_else(|| device_pubkey_hex.clone()),
                        is_current_device: device_pubkey_hex == current_device_pubkey_hex,
                        is_connected: device_pubkey_hex != current_device_pubkey_hex
                            && self.fips_nearby_links.iter().any(|link| {
                                link.device_pubkey_hex
                                    .eq_ignore_ascii_case(&device_pubkey_hex)
                            }),
                        is_authorized: true,
                        is_stale: false,
                        added_at_secs: Some(device.created_at_secs),
                        device_label: device.device_label.clone(),
                        client_label: device.client_label.clone(),
                    },
                );
            }
        }

        let current_labels = self.current_device_labels.as_ref();
        if let Some(entry) = entries.get_mut(&current_device_pubkey_hex) {
            if let Some(labels) = current_labels {
                if labels.device_label.is_some() {
                    entry.device_label = labels.device_label.clone();
                }
                if labels.client_label.is_some() {
                    entry.client_label = labels.client_label.clone();
                }
            }
        }
        entries
            .entry(current_device_pubkey_hex.clone())
            .or_insert(DeviceEntrySnapshot {
                device_pubkey_hex: current_device_pubkey_hex.clone(),
                device_npub: current_device_npub.clone(),
                is_current_device: true,
                is_connected: false,
                is_authorized: matches!(
                    logged_in.authorization_state,
                    LocalAuthorizationState::Authorized
                ),
                is_stale: matches!(
                    logged_in.authorization_state,
                    LocalAuthorizationState::Revoked
                ),
                added_at_secs: None,
                device_label: current_labels.and_then(|labels| labels.device_label.clone()),
                client_label: current_labels.and_then(|labels| labels.client_label.clone()),
            });

        let mut devices = entries.into_values().collect::<Vec<_>>();
        devices.sort_by(|left, right| {
            right
                .is_current_device
                .cmp(&left.is_current_device)
                .then_with(|| right.added_at_secs.cmp(&left.added_at_secs))
                .then_with(|| left.device_pubkey_hex.cmp(&right.device_pubkey_hex))
        });

        Some(DeviceRosterSnapshot {
            owner_public_key_hex: account.public_key_hex,
            owner_npub: account.npub,
            current_device_public_key_hex: current_device_pubkey_hex,
            current_device_npub,
            can_manage_devices: logged_in.owner_keys.is_some(),
            authorization_state: public_authorization_state(logged_in.authorization_state),
            devices,
        })
    }
}
