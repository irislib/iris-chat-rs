impl ProtocolEngine {
    pub fn has_nostr_identity_roster_history_for_owner(&self, owner_pubkey: PublicKey) -> bool {
        self.nostr_identity_roster_histories
            .values()
            .any(|history| {
                history.owner_pubkey == Some(owner_pubkey) && !history.events.is_empty()
            })
    }

    pub fn latest_nostr_identity_roster_created_at_for_owner(
        &self,
        owner_pubkey: PublicKey,
    ) -> Option<u64> {
        self.nostr_identity_roster_histories
            .values()
            .filter(|history| history.owner_pubkey == Some(owner_pubkey))
            .flat_map(|history| history.events.iter())
            .map(|event| event.created_at.as_secs())
            .max()
    }

    pub fn has_device_roster_entry_for_owner(
        &self,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
    ) -> bool {
        let owner = ndr_owner(owner_pubkey);
        let device = ndr_device(device_pubkey);
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == owner)
            .and_then(|user| user.roster)
            .is_some_and(|roster| {
                roster
                    .devices()
                    .iter()
                    .any(|entry| entry.device_pubkey == device)
            })
    }

    pub fn nostr_identity_roster_parent_ids_for_owner(
        &self,
        owner_pubkey: PublicKey,
    ) -> Vec<String> {
        let signed_ops = self
            .nostr_identity_roster_histories
            .values()
            .filter(|history| history.owner_pubkey == Some(owner_pubkey))
            .flat_map(|history| history.events.iter())
            .filter_map(|event| nostr_identity::parse_nostr_identity_roster_op_event(event).ok())
            .collect::<Vec<_>>();
        nostr_identity::nostr_identity_roster_parent_ids(&signed_ops)
    }
}

fn should_replace_provisional_local_roster(
    snapshot: &SessionManagerSnapshot,
    owner_pubkey: PublicKey,
    local_device_pubkey: NdrDevicePubkey,
    incoming_roster: &DeviceRoster,
) -> bool {
    let incoming_devices = incoming_roster.devices();
    if incoming_devices.len() <= 1
        || !incoming_devices
            .iter()
            .any(|entry| entry.device_pubkey == local_device_pubkey)
    {
        return false;
    }

    let Some(current_roster) = snapshot
        .users
        .iter()
        .find(|user| user.owner_pubkey == ndr_owner(owner_pubkey))
        .and_then(|user| user.roster.as_ref())
    else {
        return false;
    };
    let current_devices = current_roster.devices();
    current_devices.len() == 1
        && current_devices[0].device_pubkey == local_device_pubkey
        && current_roster.created_at > incoming_roster.created_at
}

fn nostr_identity_roster_profile_id(event: &Event) -> Option<String> {
    event
        .tags
        .iter()
        .find_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(|value| value.as_str()) == Some("i")
                && values.get(2).map(|value| value.as_str()) == Some("subject"))
            .then(|| values.get(1).map(|value| value.trim().to_lowercase()))
            .flatten()
        })
        .filter(|profile_id| is_uuid_like(profile_id))
}

fn is_uuid_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
}

fn app_keys_from_nostr_identity_roster_events<'a, I>(
    profile_id: &str,
    owner_pubkey: PublicKey,
    events: I,
) -> anyhow::Result<AppKeys>
where
    I: IntoIterator<Item = &'a Event>,
{
    let profile_id = profile_id.parse::<nostr_identity::NostrIdentityId>()?;
    let signed_ops = events
        .into_iter()
        .filter_map(|event| nostr_identity::parse_nostr_identity_roster_op_event(event).ok())
        .collect::<Vec<_>>();
    let projection = nostr_identity::project_nostr_identity_roster(profile_id, signed_ops);
    let owner_pubkey_hex = owner_pubkey.to_hex();
    let mut devices = projection
        .active_facets
        .values()
        .filter(|facet| facet.is_app_key() && facet.capabilities.can_write_roots)
        .filter(|facet| !(facet.pubkey == owner_pubkey_hex && facet.capabilities.can_admin_profile))
        .filter_map(|facet| {
            Some(DeviceEntry::new(
                PublicKey::parse(&facet.pubkey).ok()?,
                u64::try_from(facet.added_at).ok()?,
            ))
        })
        .collect::<Vec<_>>();
    devices.sort_by(|left, right| {
        left.created_at.cmp(&right.created_at).then_with(|| {
            left.identity_pubkey
                .to_hex()
                .cmp(&right.identity_pubkey.to_hex())
        })
    });
    Ok(AppKeys::new(devices))
}
