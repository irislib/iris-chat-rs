use super::*;

pub(in crate::core) fn is_private_transport_event(event: &Event) -> bool {
    matches!(
        event.kind.as_u16() as u32,
        MESSAGE_EVENT_KIND | INVITE_RESPONSE_KIND | 21_111
    )
}

fn recipient_device_target(event: &Event) -> BTreeSet<String> {
    let recipients = event.tags.public_keys().copied().collect::<Vec<_>>();
    let [recipient] = recipients.as_slice() else {
        return BTreeSet::new();
    };
    fips_peer_from_hex(&recipient.to_hex())
        .map(|peer| BTreeSet::from([peer.npub()]))
        .unwrap_or_default()
}

#[cfg(test)]
pub(super) fn encoded_private_targets(payload: &[u8]) -> Option<BTreeSet<String>> {
    let FipsNearbyPacket::Event { event_json, .. } = FipsNearbyPacket::decode(payload)? else {
        return None;
    };
    let event = serde_json::from_str::<Event>(&event_json).ok()?;
    if !is_private_transport_event(&event) {
        return None;
    }
    // Neither an invite response nor a message event names a live endpoint.
    // Only local delivery metadata may supply that mapping; a call wake names
    // the device it rings.
    Some(if event.kind.as_u16() == 21_111 {
        recipient_device_target(&event)
    } else {
        BTreeSet::new()
    })
}

impl AppCore {
    pub(in crate::core) fn accepts_private_nearby_event(&self, event: &Event) -> bool {
        if !is_private_transport_event(event) {
            return true;
        }
        let recipients = event.tags.public_keys().copied().collect::<Vec<_>>();
        if event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND && recipients.is_empty() {
            // Message events name nobody. Whether this device can read one is
            // decided by its ratchet sessions after signature checks, and an
            // unknown sender only reaches the bounded pending queue.
            return self.logged_in.is_some()
                && crate::core::publishing::message_event_names_nobody(event);
        }
        // Older clients still name the receiving device on message events.
        let [recipient] = recipients.as_slice() else {
            return false;
        };
        if event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND {
            let Some(local) = &self.logged_in else {
                return false;
            };
            if *recipient == local.owner_pubkey || *recipient == local.device_keys.public_key() {
                return false;
            }
            return self
                .protocol_engine
                .as_ref()
                .and_then(ProtocolEngine::local_invite_response_pubkey)
                == Some(*recipient)
                || self.private_chat_invites.values().any(|invite| {
                    invite.metadata_privacy_version
                        == nostr_double_ratchet::METADATA_PRIVACY_VERSION
                        && invite.inviter_ephemeral_private_key.is_some()
                        && invite.inviter_device_pubkey.to_nostr().ok()
                            == Some(local.device_keys.public_key())
                        && invite.inviter_ephemeral_public_key.to_nostr().ok() == Some(*recipient)
                });
        }
        self.logged_in
            .as_ref()
            .is_some_and(|local| *recipient == local.device_keys.public_key())
    }

    pub(in crate::core) fn private_nearby_targets(
        &self,
        event: &Event,
    ) -> Option<BTreeSet<String>> {
        if !is_private_transport_event(event) {
            return None;
        }
        match event.kind.as_u16() as u32 {
            // The target of a message event exists only in the local queue.
            // Without that record there is nobody to send it to.
            MESSAGE_EVENT_KIND => {
                return Some(
                    self.queued_pairwise_device(event)
                        .and_then(|device| fips_peer_from_hex(&device.to_hex()))
                        .map(|peer| BTreeSet::from([peer.npub()]))
                        .unwrap_or_default(),
                );
            }
            INVITE_RESPONSE_KIND => {}
            _ => return Some(recipient_device_target(event)),
        }
        let recipients = event.tags.public_keys().collect::<Vec<_>>();
        let [recipient] = recipients.as_slice() else {
            return Some(BTreeSet::new());
        };
        let ephemeral = recipient.to_hex();
        let targets = self
            .protocol_engine
            .as_ref()
            .map(ProtocolEngine::session_manager_snapshot)
            .into_iter()
            .flat_map(|snapshot| snapshot.users)
            .flat_map(|user| user.devices)
            .filter(|device| {
                device.public_invite.as_ref().is_some_and(|invite| {
                    invite.inviter_ephemeral_public_key.to_string() == ephemeral
                })
            })
            .filter_map(|device| fips_peer_from_hex(&device.device_pubkey.to_string()))
            .map(|device| device.npub())
            .collect();
        Some(targets)
    }
}
