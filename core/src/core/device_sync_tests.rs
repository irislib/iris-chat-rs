use super::*;

#[test]
fn chunks_are_bounded_additive_camel_case_snapshots() {
    let snapshot = DeviceSyncSnapshot {
        roster_at: 42,
        chats: vec![DeviceSyncChat {
            id: "a".repeat(64),
            updated_at: 41,
        }],
        app_keys: vec![DeviceSyncAppKeys {
            owner_pubkey: "a".repeat(64),
            created_at: 42,
            devices: vec![DeviceSyncAppKeyDevice {
                identity_pubkey: "b".repeat(64),
                created_at: 40,
            }],
        }],
        groups: vec![DeviceSyncGroup {
            id: "group".to_string(),
            name: "Group".to_string(),
            description: None,
            picture: None,
            created_by: "b".repeat(64),
            members: vec!["b".repeat(64)],
            admins: vec!["b".repeat(64)],
            protocol: None,
            revision: 0,
            created_at: 1,
            updated_at: 2,
            accepted: Some(true),
        }],
        messages: (0..200)
            .map(|index| DeviceSyncMessage {
                chat_id: "a".repeat(64),
                id: format!("message-{index}"),
                body: "x".repeat(1024),
                author: "b".repeat(64),
                created_at: 43 + index,
                expires_at: None,
            })
            .collect(),
    };
    let packets = encode_device_sync_chunks(snapshot);
    assert!(packets.len() > 1);
    assert!(packets
        .iter()
        .all(|packet| packet.len() <= DEVICE_SYNC_MAX_PACKET_BYTES));
    let json = String::from_utf8(packets[0].clone()).unwrap();
    assert!(json.contains("\"rosterAt\":42"));
    assert!(json.contains("\"appKeys\""));
    assert!(json.contains("\"ownerPubkey\""));
    assert!(json.contains("\"identityPubkey\""));
    assert!(json.contains("\"createdBy\""));
    assert!(json.contains("\"updatedAt\""));
    let app_keys_count = packets
        .iter()
        .map(|packet| serde_json::from_slice::<DeviceSyncPacket>(packet).unwrap())
        .map(|packet| match packet {
            DeviceSyncPacket::Snapshot { app_keys, .. } => app_keys.len(),
            DeviceSyncPacket::Request { .. } => 0,
        })
        .sum::<usize>();
    assert_eq!(app_keys_count, 1);
    let message_count = packets
        .iter()
        .map(|packet| serde_json::from_slice::<DeviceSyncPacket>(packet).unwrap())
        .map(|packet| match packet {
            DeviceSyncPacket::Snapshot { messages, .. } => messages.len(),
            DeviceSyncPacket::Request { .. } => 0,
        })
        .sum::<usize>();
    assert_eq!(message_count, 200);
}
