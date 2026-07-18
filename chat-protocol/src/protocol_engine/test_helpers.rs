fn pending_local_group_fanout(
    group_id: &str,
    inner_event_id: Option<&str>,
    payload: &[u8],
    created_at_secs: u64,
) -> ProtocolPendingGroupFanout {
    ProtocolPendingGroupFanout {
        group_id: group_id.to_string(),
        fanout: GroupPendingFanout::LocalSiblings {
            payload: payload.to_vec(),
        },
        inner_event_id: inner_event_id.map(str::to_string),
        created_at_secs,
        next_retry_at_secs: created_at_secs,
    }
}

fn group_snapshot_for_test(
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

fn group_roster_fact_event_for_test(admin: &Keys, snapshot: &GroupSnapshot) -> Event {
    group_roster_unsigned_event(admin.public_key(), snapshot)
        .expect("unsigned group roster fact")
        .sign_with_keys(admin)
        .expect("signed group roster fact")
}
