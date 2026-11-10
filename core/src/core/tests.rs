use super::protocol::build_protocol_subscription_filters;
use super::*;

mod app_keys_device_labels;
mod app_keys_invites_requests;
mod app_keys_publishing;
mod app_keys_roster;
mod attachment_completion;
mod block_preserves_history;
mod call_history_multi_device;
mod call_media_congestion;
mod call_quality;
mod call_wake_v2;
mod calls;
mod chat_deletion_sync;
mod chat_page_order;
mod chat_read_negentropy;
mod chat_read_receipts;
mod chat_read_sync;
mod contact_details;
mod contact_identity;
mod device_approval;
mod device_connections;
mod device_link_signer;
mod device_roster_fetch;
mod device_sync;
mod device_sync_authorization;
mod device_sync_helpers;
mod device_sync_history;
mod device_sync_history_restart;
mod device_sync_mutation_privacy;
mod device_sync_provider_refresh;
mod device_sync_records;
mod direct_chat_capability;
mod direct_files;
mod direct_group_sender_key_ack;
mod direct_message_request_acceptance;
mod direct_messages_blocking;
mod direct_messages_group_requests;
mod direct_messages_runtime_regressions;
mod direct_messages_typing;
mod explicit_direct_invite_acceptance;
mod fips_provider_privacy;
mod first_contact_receiver;
mod foreground_discovery;
mod forwarding;
mod full_invites;
mod group_message_ids;
mod group_metadata_recovery;
mod group_private_transport;
mod group_reactions;
mod group_removal;
mod groups_message_expiration;
mod groups_mutation_delivery;
mod groups_persistence_helpers;
mod groups_persistence_more;
mod groups_scale;
mod groups_sender_key;
mod groups_sender_key_continuation;
mod groups_sender_key_retry;
mod handshake_owner_proof;
mod image_proxy_preferences;
mod invite_notifications;
mod issued_invites;
mod local_message_deletion;
mod mentions;
mod mesh_chat;
mod mesh_group_replay;
mod message_expiry;
mod message_mutation_cleanup;
mod message_mutation_live_privacy;
mod message_mutations;
mod message_request_policy;
mod metadata_privacy;
mod mobile_push_controls;
mod mobile_push_read_sync;
mod nearby_publish_burst;
mod nhash_previews;
mod open_chat_finalize;
mod pending_device_link;
mod pending_publish_replay;
#[cfg(target_os = "linux")]
mod populated_discovery_idle;
mod populated_retry_idle;
mod private_block_carriers;
mod private_block_controls;
mod private_block_ingress;
mod private_block_outbox;
mod private_block_push_atomic;
mod private_block_sync;
mod private_contact_migration;
mod private_contacts;
mod private_control_interop;
mod private_group_wake;
mod private_invite_join_navigation;
mod private_invite_owner_crash;
mod private_invite_owner_verification;
mod private_settings;
mod profile_metadata_restart;
mod protocol_discovery_demand;
mod protocol_filters_push;
mod protocol_idle_replay;
mod protocol_ready_retry;
mod protocol_runtime;
mod protocol_runtime_direct_queue;
mod protocol_runtime_replay;
mod protocol_startup_guards;
mod public_follow;
mod publish_drain_progress;
mod receipt_batch_latency;
mod recipient_receipt_sync;
mod registration_recovery;
mod remote_signer;
mod remote_signer_fixture;
mod request_acceptance_sync;
mod retry_publish_ordering;
mod send_display;
mod signer_login;
mod signer_login_lookup;
mod startup_restore;
mod synthetic_history_scale;
mod synthetic_protocol_checkpoint;
mod synthetic_vm_profile;
mod timed_mute;

use app_keys_invites_requests::*;
use app_keys_roster::*;
use block_preserves_history::*;
use call_wake_v2::*;
use calls::*;
use chat_deletion_sync::*;
use chat_read_negentropy::*;
use chat_read_receipts::*;
use chat_read_sync::*;
use contact_details::*;
use device_approval::*;
pub(crate) use device_sync_helpers::*;
use device_sync_history::*;
use direct_chat_capability::*;
use direct_files::*;
use direct_messages_group_requests::*;
use direct_messages_typing::*;
use group_private_transport::*;
use group_reactions::*;
use groups_persistence_more::*;
use groups_scale::*;
use groups_sender_key_continuation::*;
use issued_invites::*;
use message_mutations::*;
pub(super) use message_request_policy::*;
use pending_publish_replay::*;
use private_block_sync::*;
use private_contact_migration::*;
use private_contacts::*;
use private_group_wake::*;
use private_invite_join_navigation::*;
use private_invite_owner_verification::*;
use profile_metadata_restart::*;
pub(crate) use protocol_runtime::*;
use recipient_receipt_sync::*;
use registration_recovery::*;
use remote_signer_fixture::*;
use signer_login::*;
use synthetic_history_scale::*;

const TEST_PROTOCOL_ENGINE_STATE_KEY: &str = "appcore/protocol-engine-state-v1";
const TEST_DEVICE_APPROVAL_REQUEST_SECRET: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";

fn seed_protocol_storage_for_test(
    storage: &dyn StorageAdapter,
    seed_session_manager: SessionManagerSnapshot,
    seed_group_manager: GroupManagerSnapshot,
) -> anyhow::Result<()> {
    let verified_app_keys_owners = seed_session_manager
        .verified_peer_app_keys_events
        .iter()
        .map(|event| ndr_owner_pubkey(event.pubkey))
        .collect::<std::collections::BTreeSet<_>>();
    let state = serde_json::json!({
        "version": 1,
        "session_manager": seed_session_manager,
        "group_manager": seed_group_manager,
        "verified_app_keys_owners": verified_app_keys_owners,
        "app_keys_provenance_version": 1,
        "invite_owner_app_keys_evidence": {},
        "pending_outbound": [],
        "pending_inbound": [],
        "pending_group_fanouts": [],
        "pending_group_pairwise_payloads": [],
        "pending_group_sender_key_messages": [],
        "pending_group_sender_key_repairs": [],
        "pending_decrypted_deliveries": [],
        "subscription_generation": 0,
    });
    storage.put(TEST_PROTOCOL_ENGINE_STATE_KEY, state.to_string())?;
    Ok(())
}

fn seed_protocol_storage_if_missing_for_test(
    storage: &dyn StorageAdapter,
    seed_session_manager: SessionManagerSnapshot,
    seed_group_manager: GroupManagerSnapshot,
) -> anyhow::Result<()> {
    if storage.get(TEST_PROTOCOL_ENGINE_STATE_KEY)?.is_none() {
        seed_protocol_storage_for_test(storage, seed_session_manager, seed_group_manager)?;
    }
    Ok(())
}

pub(super) fn protocol_publish_events(effects: &[ProtocolEffect]) -> Vec<&Event> {
    effects
        .iter()
        .map(|effect| match effect {
            ProtocolEffect::Publish(publish) => &publish.event,
        })
        .collect()
}

fn protocol_publish_events_with_kind(effects: &[ProtocolEffect], kind: u32) -> Vec<Event> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::Publish(publish) if publish.event.kind.as_u16() as u32 == kind => {
                Some(publish.event.clone())
            }
            _ => None,
        })
        .collect()
}

pub(super) fn protocol_publish_events_for_target(
    effects: &[ProtocolEffect],
    _owner_pubkey_hex: &str,
    _device_id: &str,
) -> Vec<Event> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::Publish(publish)
                if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND =>
            {
                Some(publish.event.clone())
            }
            _ => None,
        })
        .collect()
}

fn protocol_has_publish_target(
    effects: &[ProtocolEffect],
    _owner_pubkey_hex: &str,
    _device_id: &str,
) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            ProtocolEffect::Publish(publish)
                if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
        )
    })
}

fn protocol_targeted_payload_count(effects: &[ProtocolEffect], _owner_pubkey_hex: &str) -> usize {
    effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                ProtocolEffect::Publish(publish)
                    if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
            )
        })
        .count()
}
