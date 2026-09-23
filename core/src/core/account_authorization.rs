use super::*;

impl AppCore {
    pub(super) fn refresh_local_authorization_state(&mut self) -> bool {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        let previous = logged_in.authorization_state;
        let next = self.local_authorization_state(
            logged_in.owner_keys.as_ref(),
            logged_in.owner_pubkey,
            logged_in.device_keys.public_key(),
            Some(previous),
        );
        if next == previous {
            return false;
        }

        let owner_hex = logged_in.owner_pubkey.to_hex();
        let device_hex = logged_in.device_keys.public_key().to_hex();
        if let Some(logged_in) = self.logged_in.as_mut() {
            logged_in.authorization_state = next;
        }
        self.push_debug_log(
            "session.authorization",
            format!("state={next:?} owner={owner_hex} device={device_hex}"),
        );
        true
    }

    pub(super) fn restored_local_authorization_state(
        &self,
        owner_keys: Option<&Keys>,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
        previous: Option<LocalAuthorizationState>,
    ) -> LocalAuthorizationState {
        self.local_authorization_state_inner(
            owner_keys,
            owner_pubkey,
            device_pubkey,
            previous,
            false,
        )
    }

    pub(super) fn local_authorization_state(
        &self,
        owner_keys: Option<&Keys>,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
        previous: Option<LocalAuthorizationState>,
    ) -> LocalAuthorizationState {
        self.local_authorization_state_inner(
            owner_keys,
            owner_pubkey,
            device_pubkey,
            previous,
            true,
        )
    }

    fn local_authorization_state_inner(
        &self,
        owner_keys: Option<&Keys>,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
        previous: Option<LocalAuthorizationState>,
        allow_revoke: bool,
    ) -> LocalAuthorizationState {
        if owner_keys.is_some() {
            return LocalAuthorizationState::Authorized;
        }

        if let Some(authorized) =
            self.signed_local_device_authorization(owner_pubkey, device_pubkey)
        {
            return if authorized {
                LocalAuthorizationState::Authorized
            } else if matches!(
                previous,
                Some(LocalAuthorizationState::Authorized | LocalAuthorizationState::Revoked)
            ) {
                LocalAuthorizationState::Revoked
            } else {
                LocalAuthorizationState::AwaitingApproval
            };
        }

        let owner_hex = owner_pubkey.to_hex();
        let device_hex = device_pubkey.to_hex();
        let Some(app_keys) = self.app_keys.get(&owner_hex) else {
            return previous.unwrap_or(LocalAuthorizationState::AwaitingApproval);
        };

        let registered = app_keys
            .devices
            .iter()
            .any(|device| device.identity_pubkey_hex.eq_ignore_ascii_case(&device_hex));
        if registered {
            let has_local_session = self.protocol_engine.as_ref().is_some_and(|engine| {
                // The owner-signed AppKeys snapshot above already proves
                // that this local device is authorized. Here a session is
                // only an approval-handshake readiness signal; its remote
                // device must not be promoted to the owner for messaging
                // unless that separate O -> D binding is verified.
                ProtocolEngine::active_session_count_for_owner_with_snapshot(
                    &engine.session_manager_snapshot(),
                    owner_pubkey,
                ) > 0
            });
            if has_local_session {
                return LocalAuthorizationState::Authorized;
            }
            return LocalAuthorizationState::AwaitingApproval;
        }

        if !allow_revoke && previous == Some(LocalAuthorizationState::Authorized) {
            return LocalAuthorizationState::Authorized;
        }

        match previous {
            Some(LocalAuthorizationState::Authorized) | Some(LocalAuthorizationState::Revoked) => {
                LocalAuthorizationState::Revoked
            }
            _ => LocalAuthorizationState::AwaitingApproval,
        }
    }
    pub(super) fn signed_local_device_authorization(
        &self,
        owner: PublicKey,
        device: PublicKey,
    ) -> Option<bool> {
        if self.logged_in.as_ref().is_some_and(|logged_in| {
            logged_in.owner_pubkey != owner || logged_in.device_keys.public_key() != device
        }) {
            return None;
        }
        self.protocol_engine
            .as_ref()?
            .signed_local_device_authorization()
    }
}
