fn filter_has_kind(filters: &[Filter], kind: u32) -> bool {
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|value| value.as_u64() == Some(kind as u64))
                })
        })
}

#[test]
fn device_roster_pending_invite_fetch_skips_message_history() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let message_author = Keys::generate();
    let group_author = Keys::generate();
    let relay = crate::local_relay::TestRelay::start();
    let mut core = logged_in_test_core("device-roster-fetch-metadata-only", &owner, &device);

    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    let mut plan = protocol_plan_for_test(
        vec![message_author.public_key()],
        vec![group_author.public_key()],
    );
    plan.roster_authors = vec![owner.public_key().to_hex()];
    core.protocol_subscription_runtime.desired_plan = Some(plan);
    core.debug_log.clear();

    let filters = core.recent_protocol_metadata_filters(UnixSeconds(1_777_159_500));
    assert!(
        !filter_has_kind(&filters, MESSAGE_EVENT_KIND),
        "Devices refresh must not backfill direct messages"
    );
    assert!(
        !filter_has_kind(&filters, GROUP_SENDER_KEY_MESSAGE_KIND),
        "Devices refresh must not backfill group sender-key messages"
    );

    assert!(
        core.fetch_pending_device_invites_for_local_owner(),
        "Devices refresh should still fetch roster/invite metadata"
    );
    assert!(
        core.debug_log.iter().any(|entry| {
            entry.category == "protocol.catch_up.fetch" && entry.detail.contains("messages=false")
        }),
        "Devices refresh must use the metadata-only catch-up path"
    );
}
