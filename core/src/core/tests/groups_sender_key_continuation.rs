// Protocol receive work yields after a bounded key-search slice. Synchronous
// matrix fixtures drive the same continuation turns the app actor schedules.
fn finish_group_search_for_test(
    engine: &mut ProtocolEngine,
    mut result: ProtocolGroupIncomingResult,
) -> ProtocolGroupIncomingResult {
    for _ in 0..128 {
        if !engine.has_ready_group_sender_key_retry_work() {
            return result;
        }
        let retry = engine
            .retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
            .unwrap();
        result.events.extend(retry.group_result.events);
        result.effects.extend(retry.group_result.effects);
        result.effects.extend(retry.effects);
    }
    assert!(
        !engine.has_ready_group_sender_key_retry_work(),
        "Bounded group search did not finish"
    );
    result
}

fn deliver_protocol_effects_with_ready_work(
    engine: &mut ProtocolEngine,
    effects: &[ProtocolEffect],
) -> Vec<GroupIncomingEvent> {
    let mut events = deliver_protocol_effects_to_engine(engine, effects);
    let continued = finish_group_search_for_test(engine, ProtocolGroupIncomingResult::default());
    events.extend(continued.events);
    apply_protocol_events_to_engine(
        engine,
        &ordered_protocol_events(&continued.effects),
        &mut events,
    );
    events
}

fn deliver_protocol_effects_with_ready_work_once(
    engine: &mut ProtocolEngine,
    effects: &[ProtocolEffect],
) -> (Vec<GroupIncomingEvent>, Vec<ProtocolEffect>) {
    let (mut events, mut effects) = deliver_protocol_effects_to_engine_once(engine, effects);
    let continued = finish_group_search_for_test(engine, ProtocolGroupIncomingResult::default());
    events.extend(continued.events);
    effects.extend(continued.effects);
    (events, effects)
}
