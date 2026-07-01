impl ProtocolEngine {
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
