use super::*;

impl AppCore {
    pub(super) fn can_use_chats(&self) -> bool {
        matches!(
            self.logged_in
                .as_ref()
                .map(|logged_in| logged_in.authorization_state),
            Some(LocalAuthorizationState::Authorized)
        )
    }

    pub(super) fn account_protocol_readiness(&self) -> ProtocolReadinessSnapshot {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::AccountMissing,
                "Create or restore a profile first.",
            );
        };
        match logged_in.authorization_state {
            LocalAuthorizationState::AwaitingApproval => {
                return ProtocolReadinessSnapshot::blocked(
                    ProtocolReadinessReason::DeviceAwaitingApproval,
                    "This device is still waiting for approval.",
                );
            }
            LocalAuthorizationState::Revoked => {
                return ProtocolReadinessSnapshot::blocked(
                    ProtocolReadinessReason::DeviceRevoked,
                    "This device has been removed from the profile. Log out to continue.",
                );
            }
            LocalAuthorizationState::Authorized => {}
        }
        if self.protocol_engine.is_none() {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::ProtocolEngineUnavailable,
                "Protocol engine is not ready.",
            );
        }
        ProtocolReadinessSnapshot::ready()
    }

    pub(super) fn chat_protocol_readiness(&self, chat_id: &str) -> ProtocolReadinessSnapshot {
        let account_readiness = self.account_protocol_readiness();
        if !account_readiness.can_send {
            return account_readiness;
        }
        if is_group_chat_id(chat_id) {
            let Some(group_id) = parse_group_id_from_chat_id(chat_id) else {
                return ProtocolReadinessSnapshot::blocked(
                    ProtocolReadinessReason::GroupMetadataMissing,
                    "This group is not ready yet. Waiting for group metadata.",
                );
            };
            return self.group_protocol_readiness(&group_id);
        }
        if self.is_owner_blocked(chat_id) {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::BlockedPeer,
                "User is blocked.",
            );
        }
        let Ok((_owner_hex, peer_pubkey)) = parse_peer_input(chat_id) else {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::PeerAppKeysMissing,
                "This chat is not ready yet. Waiting for the recipient's app keys.",
            );
        };
        let Some(protocol_engine) = self.protocol_engine.as_ref() else {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::ProtocolEngineUnavailable,
                "Protocol engine is not ready.",
            );
        };
        if !protocol_engine.has_roster_for_owner(peer_pubkey) {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::PeerAppKeysMissing,
                "This chat is not ready yet. Waiting for the recipient's app keys.",
            );
        }
        if !protocol_engine.has_direct_send_capability_for_owner(peer_pubkey) {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::PeerSessionMissing,
                "This chat is not ready yet. Waiting for a secure session.",
            );
        }
        ProtocolReadinessSnapshot::ready()
    }

    pub(super) fn group_protocol_readiness(&self, group_id: &str) -> ProtocolReadinessSnapshot {
        let account_readiness = self.account_protocol_readiness();
        if !account_readiness.can_send {
            return account_readiness;
        }
        let Some(group) = self.groups.get(group_id) else {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::GroupMetadataMissing,
                "This group is not ready yet. Waiting for group metadata.",
            );
        };
        let Some(local_owner_hex) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex())
        else {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::AccountMissing,
                "Create or restore a profile first.",
            );
        };
        if !group
            .members
            .iter()
            .any(|owner| owner.to_string() == local_owner_hex)
        {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::GroupNotJoined,
                "You are not a member of this group.",
            );
        }
        let Some(protocol_engine) = self.protocol_engine.as_ref() else {
            return ProtocolReadinessSnapshot::blocked(
                ProtocolReadinessReason::ProtocolEngineUnavailable,
                "Protocol engine is not ready.",
            );
        };
        for member in &group.members {
            let member_hex = member.to_string();
            if member_hex == local_owner_hex {
                continue;
            }
            let Ok(member_pubkey) = PublicKey::parse(&member_hex) else {
                return ProtocolReadinessSnapshot::blocked(
                    ProtocolReadinessReason::GroupMemberAppKeysMissing,
                    "This group is not ready yet. Waiting for member app keys.",
                );
            };
            if !protocol_engine.has_roster_for_owner(member_pubkey) {
                return ProtocolReadinessSnapshot::blocked(
                    ProtocolReadinessReason::GroupMemberAppKeysMissing,
                    "This group is not ready yet. Waiting for member app keys.",
                );
            }
            if !protocol_engine.has_direct_send_capability_for_owner(member_pubkey) {
                return ProtocolReadinessSnapshot::blocked(
                    ProtocolReadinessReason::GroupMemberSessionMissing,
                    "This group is not ready yet. Waiting for secure member sessions.",
                );
            }
        }
        ProtocolReadinessSnapshot::ready()
    }
}
