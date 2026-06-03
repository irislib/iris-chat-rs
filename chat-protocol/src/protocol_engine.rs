use super::*;

include!("protocol_engine/types.rs");
include!("protocol_engine/engine_core.rs");
include!("protocol_engine/engine_sends.rs");
include!("protocol_engine/engine_incoming_retry.rs");
include!("protocol_engine/engine_resolution.rs");
include!("protocol_engine/engine_sender_key_repair.rs");
include!("protocol_engine/engine_queue_filters.rs");
include!("protocol_engine/free_functions.rs");

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
        let storage = Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new())
            as Arc<dyn StorageAdapter>;
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), device)
            .expect("protocol engine")
    }

    fn pending_outbound(
        message_id: &str,
        chat_id: &str,
        recipient_owner_hex: String,
        send_remote: bool,
        local_sibling_payload: Option<Vec<u8>>,
        probe_local_sibling_roster: bool,
        reason: ProtocolPendingReason,
    ) -> ProtocolPendingOutbound {
        ProtocolPendingOutbound {
            message_id: message_id.to_string(),
            chat_id: chat_id.to_string(),
            recipient_owner_hex,
            send_remote,
            remote_payload: b"remote".to_vec(),
            local_sibling_payload,
            inner_event_id: Some(message_id.to_string()),
            delivered_remote_device_hexes: Vec::new(),
            delivered_local_device_hexes: Vec::new(),
            probe_local_sibling_roster,
            created_at_secs: 1,
            next_retry_at_secs: 1,
            reason,
        }
    }

    #[test]
    fn local_sibling_roster_probe_does_not_block_delivery() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        engine.pending_outbound.push(pending_outbound(
            "message-id",
            "chat-id",
            owner.public_key().to_hex(),
            false,
            Some(b"local".to_vec()),
            true,
            ProtocolPendingReason::PublishRetry,
        ));

        assert!(!engine.has_delivery_blocking_message_work("message-id"));
    }

    #[test]
    fn known_local_sibling_target_blocks_delivery() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let sibling = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        engine
            .session_manager
            .replace_local_roster(DeviceRoster::new(
                NdrUnixSeconds(1),
                vec![
                    AuthorizedDevice::new(ndr_device(device.public_key()), NdrUnixSeconds(1)),
                    AuthorizedDevice::new(ndr_device(sibling.public_key()), NdrUnixSeconds(1)),
                ],
            ));
        engine.pending_outbound.push(pending_outbound(
            "message-id",
            "chat-id",
            owner.public_key().to_hex(),
            false,
            Some(b"local".to_vec()),
            false,
            ProtocolPendingReason::PublishRetry,
        ));

        assert!(engine.has_delivery_blocking_message_work("message-id"));
    }

    #[test]
    fn missing_remote_roster_blocks_delivery() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        engine.pending_outbound.push(pending_outbound(
            "message-id",
            &peer.public_key().to_hex(),
            peer.public_key().to_hex(),
            true,
            None,
            false,
            ProtocolPendingReason::MissingRoster,
        ));

        assert!(engine.has_delivery_blocking_message_work("message-id"));
    }
}
