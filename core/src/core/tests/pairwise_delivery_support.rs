//! Message events name nobody. These helpers read, or supply, the delivery
//! record that travels beside each one, the way every carrier must.

use super::*;

/// The engine's delivery record for a stand-in pairwise envelope. Message
/// events name nobody, so tests supply the target the way the engine does.
pub(crate) fn test_pairwise_carrier() -> ProtocolCarrier {
    ProtocolCarrier::Pairwise {
        device: Keys::generate().public_key(),
    }
}

/// The device the engine prepared a message event for. The event names
/// nobody; only the publish effect's delivery record says.
pub(crate) fn effect_pairwise_device(
    effects: &[ProtocolEffect],
    event: &Event,
) -> Option<PublicKey> {
    effects.iter().find_map(|ProtocolEffect::Publish(publish)| {
        (publish.event.id == event.id)
            .then(|| publish.carrier.pairwise_device())
            .flatten()
    })
}

/// The pairwise envelope a batch of publish effects prepared for `device`.
pub(crate) fn pairwise_event_for_device(
    effects: &[ProtocolEffect],
    device: PublicKey,
) -> Option<&Event> {
    effects.iter().find_map(|ProtocolEffect::Publish(publish)| {
        (publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
            && publish.carrier.pairwise_device() == Some(device))
        .then_some(&publish.event)
    })
}

/// Whether `core` queued this message event for `device`.
pub(crate) fn queued_for_device(core: &AppCore, event: &Event, device: PublicKey) -> bool {
    event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
        && event.tags.public_keys().next().is_none()
        && core.queued_pairwise_device(event) == Some(device)
}

/// Queue a stand-in pairwise envelope for `device` the way a publish effect
/// from the engine does. Nothing else may put a message event in the outbox.
pub(crate) fn publish_pairwise_for_test(
    core: &mut AppCore,
    event: Event,
    device: PublicKey,
    chat_id: &str,
    inner_event_id: Option<String>,
) -> bool {
    core.publish_protocol_event(ProtocolPublish {
        authored_at_secs: None,
        event,
        chat_id: chat_id.to_owned(),
        inner_event_id,
        carrier: ProtocolCarrier::Pairwise { device },
    })
}

/// A publish effect for a stand-in pairwise envelope with a fresh target.
pub(crate) fn pairwise_publish_for_test(
    event: Event,
    chat_id: &str,
    inner_event_id: Option<String>,
) -> ProtocolPublish {
    ProtocolPublish {
        authored_at_secs: None,
        event,
        chat_id: chat_id.to_owned(),
        inner_event_id,
        carrier: test_pairwise_carrier(),
    }
}
