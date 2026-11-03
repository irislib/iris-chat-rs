use super::*;

#[test]
fn delete_for_me_inventory_requires_an_opted_in_private_state_reader() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _, _dir) = crate::core::tests::logged_in_test_core_with_updates(
        "private-delete-inventory",
        &owner,
        &device,
    );
    let chat = Keys::generate().public_key().to_hex();
    core.delete_local_message(&chat, "hidden");
    core.delete_local_message(&chat, "42");
    // Older releases used the same local marker for expiry and explicit removal.
    core.app_store
        .delete_message_locally(&chat, "legacy-or-expired", None)
        .unwrap();
    for (private_events, message_deletions) in [(false, true), (true, false), (true, true)] {
        let inventory = HistorySession::snapshot(
            &core,
            Filter { since: 0, until: 0 },
            false,
            MAX_RECORDS,
            RecordScope::State,
            "",
            HistoryRecordAccess {
                supported: true,
                recipient_receipts: false,
                receipt_link_at: None,
                private_events,
                message_deletions,
                read_states: false,
                name_approvals: false,
                private_facts: false,
                target_since: 0,
            },
        )
        .unwrap();
        let events: Vec<_> = inventory
            .records
            .values()
            .filter_map(|r| match r.load(&core)? {
                DeviceSyncRecord::PrivateMessageDeletion { event } => {
                    core.message_deletion_target(&event)
                }
                _ => None,
            })
            .collect();
        if private_events && message_deletions {
            assert_eq!(events, [(chat.clone(), "hidden".to_string())]);
        } else {
            assert!(events.is_empty());
        }
    }
}
