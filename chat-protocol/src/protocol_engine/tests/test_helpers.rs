use super::*;

pub(super) fn pending_local_group_fanout(
    group_id: &str,
    inner_event_id: Option<&str>,
    payload: &[u8],
    created_at_secs: u64,
) -> ProtocolPendingGroupFanout {
    ProtocolPendingGroupFanout {
        group_id: group_id.to_string(),
        fanout: GroupPendingFanout::LocalSiblings {
            payload: payload.to_vec(),
        }
        .into(),
        remaining_devices: None,
        inner_event_id: inner_event_id.map(str::to_string),
        created_at_secs,
        next_retry_at_secs: created_at_secs,
    }
}

pub(super) fn group_snapshot_for_test(
    group_id: &str,
    name: &str,
    revision: u64,
    admin: &Keys,
    members: &[PublicKey],
) -> GroupSnapshot {
    GroupSnapshot {
        group_id: group_id.to_string(),
        protocol: GroupProtocol::sender_key_v1(),
        name: name.to_string(),
        picture: None,
        about: None,
        created_by: ndr_owner(admin.public_key()),
        members: members.iter().copied().map(ndr_owner).collect(),
        admins: vec![ndr_owner(admin.public_key())],
        revision,
        created_at: NdrUnixSeconds(10),
        updated_at: NdrUnixSeconds(10 + revision),
    }
}

pub(super) fn group_roster_fact_event_for_test(admin: &Keys, snapshot: &GroupSnapshot) -> Event {
    group_roster_unsigned_event(admin.public_key(), snapshot)
        .expect("unsigned group roster fact")
        .sign_with_keys(admin)
        .expect("signed group roster fact")
}

pub(super) fn incoming_sender_key_record(
    group_id: &str,
    owner: PublicKey,
    device: PublicKey,
    author: NdrDevicePubkey,
    state: &nostr_double_ratchet::SenderKeyState,
) -> nostr_double_ratchet::GroupSenderKeyRecordSnapshot {
    nostr_double_ratchet::GroupSenderKeyRecordSnapshot {
        group_id: group_id.into(),
        sender_owner: ndr_owner(owner),
        sender_device: ndr_device(device),
        sender_event_pubkey: author,
        sender_event_secret_key: None,
        latest_key_id: Some(state.key_id()),
        states: vec![state.clone()],
        distribution_history: vec![],
        delivery_state: None,
        distributed_to: vec![],
        repair_snapshots: vec![],
    }
}
