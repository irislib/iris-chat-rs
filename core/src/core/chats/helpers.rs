use super::*;

pub(super) fn is_supported_group_pairwise_payload(payload: &[u8]) -> bool {
    let codec = JsonGroupPayloadCodecV1;
    let Ok(Some(command)) =
        nostr_double_ratchet::GroupPayloadCodec::decode_pairwise_command(&codec, payload)
    else {
        return false;
    };
    match command {
        nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot { snapshot } => {
            snapshot.protocol.is_sender_key_v1()
        }
        nostr_double_ratchet::GroupPairwiseCommand::SenderKeyDistribution { .. }
        | nostr_double_ratchet::GroupPairwiseCommand::SenderKeyRepairRequest { .. } => true,
        _ => false,
    }
}

pub(super) fn summarize_group_send_effect_targets(effects: &[ProtocolEffect]) -> String {
    let mut targets = Vec::new();
    for effect in effects {
        let ProtocolEffect::Publish(publish) = effect;
        let stage = if publish.inner_event_id.is_some() {
            "delivery"
        } else {
            "control"
        };
        targets.push(format!("{stage}:{}:{}", publish.chat_id, publish.event.id));
    }
    targets.join("|")
}

pub(super) fn delivery_trace_for_source_event(
    source_event_id: Option<&str>,
) -> MessageDeliveryTraceSnapshot {
    let mut trace = MessageDeliveryTraceSnapshot::default();
    if let Some(source_event_id) = source_event_id {
        trace.outer_event_ids.push(source_event_id.to_string());
    }
    trace
}

pub(super) fn push_unique(values: &mut Vec<String>, value: &str) {
    if values.iter().any(|existing| existing == value) {
        return;
    }
    values.push(value.to_string());
}
