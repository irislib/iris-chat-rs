use super::*;

#[derive(PartialEq, Eq)]
struct BootstrapIdentity {
    owner: PublicKey,
    device: PublicKey,
    has_owner_key: bool,
    app_keys: Option<KnownAppKeys>,
    profile: Option<OwnerProfileRecord>,
    invite: Option<UnsignedEvent>,
    deferred_app_keys: bool,
    device_labels: Option<CurrentDeviceLabels>,
    forwarded: Vec<Event>,
}

pub(in crate::core) struct FipsNearbyBootstrapCache {
    identity: BootstrapIdentity,
    payloads: Vec<Vec<u8>>,
}

impl AppCore {
    pub(in crate::core) fn local_fips_nearby_bootstrap_payloads(&self) -> Vec<Vec<u8>> {
        let Some(login) = self.logged_in.as_ref() else {
            self.fips_nearby_bootstrap.replace(None);
            return Vec::new();
        };
        // Linked devices forward the original owner signatures. Include those
        // records in the cache key so newly learned identity still propagates.
        let forwarded = if login.owner_keys.is_none() {
            [Kind::Metadata, Kind::Custom(APP_KEYS_EVENT_KIND as u16)]
                .into_iter()
                .filter_map(|kind| self.cached_local_fips_identity(kind))
                .collect()
        } else {
            Vec::new()
        };
        let owner_hex = login.owner_pubkey.to_hex();
        let identity = BootstrapIdentity {
            owner: login.owner_pubkey,
            device: login.device_keys.public_key(),
            has_owner_key: login.owner_keys.is_some(),
            app_keys: self.app_keys.get(&owner_hex).cloned(),
            profile: self.owner_profiles.get(&owner_hex).cloned(),
            invite: self.protocol_engine.as_ref().and_then(|engine| {
                nostr_double_ratchet::invite_unsigned_event(&engine.local_invite()?).ok()
            }),
            deferred_app_keys: self.defer_owner_app_keys_publish,
            device_labels: self.current_device_labels.clone(),
            forwarded,
        };
        if let Some(cached) = self.fips_nearby_bootstrap.borrow().as_ref() {
            if cached.identity == identity {
                return cached.payloads.clone();
            }
        }

        // Encryption uses a fresh nonce. Rebuilding an unchanged roster creates
        // a new event ID, which restarts bootstrap on every connected peer; their
        // reply then refreshes our bootstrap again. Reuse the exact signed bytes
        // until local identity changes, while retaining normal reconnect sends.
        let (background, mut durable) = self.build_local_identity_artifacts();
        durable.extend(
            identity
                .forwarded
                .iter()
                .cloned()
                .map(|event| ("linked-identity-nearby", event)),
        );
        if let Some(event) = self.deferred_owner_app_keys_for_fips_nearby() {
            durable.insert(0, ("app-keys-nearby", event));
        }
        let payloads: Vec<_> = durable
            .into_iter()
            .chain(background)
            .map(|(_, event)| event)
            .filter(is_fips_nearby_bootstrap_event)
            .filter_map(|event| encode_fips_nearby_event(&event))
            .collect();
        self.fips_nearby_bootstrap
            .replace(Some(FipsNearbyBootstrapCache {
                identity,
                payloads: payloads.clone(),
            }));
        payloads
    }
}
