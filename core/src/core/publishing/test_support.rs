use super::*;

impl AppCore {
    /// Queue a stand-in pairwise envelope under any label. Production code
    /// reaches the outbox with a message event only through a publish effect.
    pub(in crate::core) fn publish_pairwise_runtime_event_for_test(
        &mut self,
        event: Event,
        label: &'static str,
        completion: Option<(String, String)>,
        device: PublicKey,
    ) -> bool {
        let (inner_event_id, chat_id) = completion
            .map(|(inner_event_id, chat_id)| (Some(inner_event_id), Some(chat_id)))
            .unwrap_or((None, None));
        self.publish_runtime_event_with_metadata(
            event,
            label,
            chat_id,
            inner_event_id,
            None,
            Some(device),
        )
    }
}
