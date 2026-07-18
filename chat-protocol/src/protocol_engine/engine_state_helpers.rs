fn quarantine_unverified_owner_rosters(
    snapshot: &mut SessionManagerSnapshot,
    verified_app_keys_owners: &BTreeSet<NdrOwnerPubkey>,
) {
    for user in &mut snapshot.users {
        if verified_app_keys_owners.contains(&user.owner_pubkey) {
            continue;
        }

        // A device may always speak for the identity represented by its own
        // key. Preserve only that exact legacy singleton; every O != D binding
        // needs persisted AppKeys provenance.
        let exact_self_owned_singleton = user.roster.as_ref().is_some_and(|roster| {
            roster.devices().len() == 1
                && provisional_owner_from_sender_pubkey(roster.devices()[0].device_pubkey)
                    == user.owner_pubkey
        });
        if !exact_self_owned_singleton {
            user.roster = None;
        }

        for device in &mut user.devices {
            // O = D is authenticated by possession of D's key even when an
            // older persisted session has no synthetic singleton roster.
            device.authorized = provisional_owner_from_sender_pubkey(device.device_pubkey)
                == user.owner_pubkey;
            if !device.authorized {
                device.is_stale = false;
                device.stale_since = None;
            }
        }
    }
}

fn load_or_create_local_invite(
    storage: &dyn StorageAdapter,
    device_pubkey: PublicKey,
    device_id: &str,
    owner_pubkey: PublicKey,
) -> anyhow::Result<Invite> {
    let storage_key = format!("device-invite/{device_id}");
    if let Some(serialized) = storage.get(&storage_key)? {
        if let Ok(invite) = Invite::deserialize(&serialized) {
            return Ok(normalize_local_invite_owner(invite, owner_pubkey));
        }
    }

    let mut invite = Invite::create_new(device_pubkey, Some(device_id.to_string()), None)?;
    invite = normalize_local_invite_owner(invite, owner_pubkey);
    storage.put(&storage_key, invite.serialize()?)?;
    Ok(invite)
}

fn normalize_local_invite_owner(mut invite: Invite, owner_pubkey: PublicKey) -> Invite {
    invite.inviter_owner_pubkey = Some(ndr_owner(owner_pubkey));
    invite.owner_public_key = Some(owner_pubkey);
    invite
}

fn user_record_snapshot(
    snapshot: &SessionManagerSnapshot,
    owner: NdrOwnerPubkey,
) -> Option<&UserRecordSnapshot> {
    snapshot.users.iter().find(|user| user.owner_pubkey == owner)
}

fn roster_device_pubkeys(user: &UserRecordSnapshot) -> Option<Vec<NdrDevicePubkey>> {
    user.roster.as_ref().map(|roster| {
        roster
            .devices()
            .iter()
            .map(|entry| entry.device_pubkey)
            .collect()
    })
}

fn user_can_send_to_device(user: &UserRecordSnapshot, device: NdrDevicePubkey) -> bool {
    user.devices
        .iter()
        .find(|record| record.device_pubkey == device)
        .is_some_and(device_record_can_send)
}

fn device_record_can_send(record: &DeviceRecordSnapshot) -> bool {
    record.authorized
        && !record.is_stale
        && (record.active_session.is_some()
            || !record.inactive_sessions.is_empty()
            || record.public_invite.is_some())
}
