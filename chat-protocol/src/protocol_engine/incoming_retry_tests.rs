#[cfg(test)]
mod incoming_retry_tests {
    use super::*;
    use crate::InMemoryStorage;
    use nostr_double_ratchet::message_event;

    fn test_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
        ProtocolEngine::load_or_create_for_local_device(
            Arc::new(InMemoryStorage::new()),
            owner.public_key(),
            device,
        )
        .expect("test protocol engine")
    }

    fn direct_message_before_receiver_observes_response(
        receiver: &ProtocolEngine,
        sender_owner: &Keys,
        body: &str,
        created_at_secs: u64,
    ) -> (Event, Event) {
        let invite = receiver.local_invite().expect("receiver invite");
        let (mut sender_session, response) = invite
            .accept_with_owner(
                sender_owner.public_key(),
                sender_owner.secret_key().to_secret_bytes(),
                Some(sender_owner.public_key().to_hex()),
                Some(sender_owner.public_key()),
            )
            .expect("sender accepts receiver invite");
        let response_event = invite_response_event(&response).expect("invite response event");
        let plan = sender_session
            .plan_send(body.as_bytes(), NdrUnixSeconds(created_at_secs))
            .expect("sender plans direct message");
        let sent = sender_session.apply_send(plan);
        let message_event = message_event(&sent.envelope).expect("direct message event");
        (message_event, response_event)
    }

    #[test]
    fn invalid_pending_ciphertext_does_not_starve_other_senders() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let mut receiver = test_engine(&owner, &device);
        let sender = Keys::generate();
        let invite = receiver.local_invite().unwrap();
        let (mut session, response) = invite
            .accept_with_owner(
                sender.public_key(),
                sender.secret_key().to_secret_bytes(),
                Some(sender.public_key().to_hex()),
                Some(sender.public_key()),
            )
            .unwrap();
        let plan = session
            .plan_send(b"valid retry", NdrUnixSeconds(100))
            .unwrap();
        let sent = session.apply_send(plan);
        let valid_event = message_event(&sent.envelope).unwrap();
        let mut damaged = sent.envelope;
        damaged.ciphertext = "invalid ciphertext".to_owned();
        let bad_event = message_event(&damaged).unwrap();
        bad_event.verify().unwrap();
        let (healthy_event, healthy_response) = direct_message_before_receiver_observes_response(
            &receiver,
            &Keys::generate(),
            "other sender still arrives",
            100,
        );
        for event in [&bad_event, &healthy_event] {
            assert!(receiver
                .process_direct_message_event(event)
                .unwrap()
                .is_none());
        }
        let pending = std::mem::take(&mut receiver.pending_inbound);
        receiver
            .observe_invite_response_event(&invite_response_event(&response).unwrap())
            .unwrap();
        receiver
            .observe_invite_response_event(&healthy_response)
            .unwrap();
        receiver.pending_inbound = pending;

        let batch = receiver
            .retry_pending_protocol(NdrUnixSeconds(200))
            .unwrap();
        assert_eq!(batch.direct_messages.len(), 1);
        assert_eq!(
            batch.direct_messages[0].content,
            "other sender still arrives"
        );
        assert_eq!(receiver.pending_inbound.len(), 1);
        assert_eq!(receiver.pending_inbound[0].event.id, bad_event.id);
        assert!(receiver.pending_inbound[0].next_retry_at_secs > 200);
        assert_eq!(
            receiver
                .process_direct_message_event(&valid_event)
                .unwrap()
                .unwrap()
                .content,
            "valid retry",
            "failed ciphertext must not advance the sender's session"
        );
    }

    #[test]
    fn pending_receive_save_failure_preserves_entire_queue_and_delivery_journal() {
        struct FailingStorage {
            inner: InMemoryStorage,
            fail: std::sync::atomic::AtomicBool,
        }
        impl StorageAdapter for FailingStorage {
            fn get(&self, key: &str) -> crate::StorageResult<Option<String>> {
                self.inner.get(key)
            }
            fn put(&self, key: &str, value: String) -> crate::StorageResult<()> {
                if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(crate::StorageError::new("injected write failure"));
                }
                self.inner.put(key, value)
            }
            fn del(&self, key: &str) -> crate::StorageResult<()> {
                self.inner.del(key)
            }
            fn list(&self, prefix: &str) -> crate::StorageResult<Vec<String>> {
                self.inner.list(prefix)
            }
        }
        let owner = Keys::generate();
        let device = Keys::generate();
        let mut receiver = test_engine(&owner, &device);
        let mut responses = Vec::new();
        for body in ["first waiting message", "second waiting message"] {
            let sender = Keys::generate();
            let (event, response) =
                direct_message_before_receiver_observes_response(&receiver, &sender, body, 100);
            assert!(receiver
                .process_direct_message_event(&event)
                .unwrap()
                .is_none());
            responses.push(response);
        }
        // Install the sessions without consuming the queued messages yet.
        let pending = std::mem::take(&mut receiver.pending_inbound);
        for response in responses {
            receiver.observe_invite_response_event(&response).unwrap();
        }
        receiver.pending_inbound = pending;
        let storage = Arc::new(FailingStorage {
            inner: InMemoryStorage::new(),
            fail: std::sync::atomic::AtomicBool::new(false),
        });
        receiver.storage = storage.clone();
        receiver.persist().unwrap();
        storage
            .fail
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(receiver
            .retry_pending_protocol(NdrUnixSeconds(200))
            .is_err());
        assert_eq!(
            receiver.pending_inbound.len(),
            2,
            "a failed first receive must retain later events"
        );
        assert_eq!(
            receiver.pending_decrypted_deliveries.len(),
            1,
            "already advanced ratchet retains plaintext"
        );
        storage
            .fail
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let batch = receiver
            .retry_pending_protocol(NdrUnixSeconds(200))
            .unwrap();
        assert_eq!(
            batch.direct_messages.len(),
            2,
            "each pending message is offered exactly once per batch"
        );
        assert!(receiver.pending_inbound.is_empty());
        let mut restored =
            ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device)
                .unwrap();
        assert_eq!(
            restored
                .retry_pending_protocol(NdrUnixSeconds(201))
                .unwrap()
                .direct_messages
                .len(),
            2,
            "both deliveries survive restart before the app acknowledges them"
        );
        restored.ack_pending_decrypted_deliveries().unwrap();
        assert!(restored
            .retry_pending_protocol(NdrUnixSeconds(202))
            .unwrap()
            .direct_messages
            .is_empty());
    }

    #[test]
    fn direct_message_retry_clears_header_sender_key_candidate() {
        let bob_owner = Keys::generate();
        let bob_device = Keys::generate();
        let alice_owner = Keys::generate();
        let mut bob = test_engine(&bob_owner, &bob_device);
        let (message_event, response_event) = direct_message_before_receiver_observes_response(
            &bob,
            &alice_owner,
            "hello after backfill",
            100,
        );

        assert!(
            bob.process_direct_message_event(&message_event)
                .expect("unknown direct message queues")
                .is_none(),
            "receiver should not decrypt before it observes the session response"
        );
        assert_eq!(bob.pending_inbound.len(), 1);
        assert_eq!(bob.pending_group_sender_key_messages.len(), 1);
        assert!(
            bob.has_pending_retry_work(),
            "queued direct/group candidate should keep liveness retry work active"
        );

        let mut direct_messages = bob
            .observe_invite_response_event(&response_event)
            .expect("receiver observes session response");
        // The caller durably applies the first batch before asking for more.
        bob.ack_pending_decrypted_deliveries()
            .expect("ack applied batch");
        let retry = bob
            .retry_pending_protocol(NdrUnixSeconds(103))
            .expect("retry pending direct message");
        direct_messages
            .direct_messages
            .extend(retry.direct_messages);

        assert_eq!(direct_messages.direct_messages.len(), 1);
        assert_eq!(
            direct_messages.direct_messages[0].content,
            "hello after backfill"
        );
        bob.ack_pending_decrypted_deliveries()
            .expect("ack applied retry");
        assert!(bob.pending_inbound.is_empty());
        assert!(
            bob.pending_group_sender_key_messages.is_empty(),
            "the same event must not remain queued as a sender-key repair candidate after direct decrypt succeeds"
        );
        assert!(
            bob.pending_group_sender_key_repairs.is_empty(),
            "direct decrypt success should not leave sender-key repair bookkeeping behind"
        );
        assert!(
            !bob.has_pending_retry_work(),
            "all retry work should be clear after the pending direct message applies"
        );
    }

    #[test]
    fn unknown_group_sender_key_candidate_alone_does_not_keep_retry_work_alive() {
        let bob_owner = Keys::generate();
        let bob_device = Keys::generate();
        let alice_owner = Keys::generate();
        let mut bob = test_engine(&bob_owner, &bob_device);
        let (message_event, _response_event) = direct_message_before_receiver_observes_response(
            &bob,
            &alice_owner,
            "hello before metadata",
            100,
        );

        assert!(bob
            .process_direct_message_event(&message_event)
            .expect("unknown direct message queues")
            .is_none());
        assert_eq!(bob.pending_group_sender_key_messages.len(), 1);
        bob.pending_inbound.clear();

        assert!(
            !bob.has_pending_retry_work(),
            "a header-shaped direct event with no known group sender key must not keep liveness hot by itself"
        );
    }

    #[test]
    fn known_direct_message_author_prunes_unmapped_sender_key_candidate_without_decrypt() {
        let bob_owner = Keys::generate();
        let bob_device = Keys::generate();
        let alice_owner = Keys::generate();
        let mut bob = test_engine(&bob_owner, &bob_device);
        let (message_event, response_event) = direct_message_before_receiver_observes_response(
            &bob,
            &alice_owner,
            "hello from another target",
            100,
        );

        assert!(bob
            .process_direct_message_event(&message_event)
            .expect("unknown direct message queues")
            .is_none());
        assert_eq!(bob.pending_inbound.len(), 1);
        assert_eq!(bob.pending_group_sender_key_messages.len(), 1);

        bob.pending_inbound.clear();
        let retry = bob
            .observe_invite_response_event(&response_event)
            .expect("receiver observes session response");

        assert!(
            retry.direct_messages.is_empty(),
            "test setup removed the pending direct decrypt path"
        );
        assert!(bob.is_known_message_author(message_event.pubkey));
        assert!(
            bob.pending_group_sender_key_messages.is_empty(),
            "once the event pubkey is known to be a direct-message author, an unmapped group sender-key candidate should be pruned"
        );
        assert_eq!(
            bob.debug_snapshot().pending_group_sender_key_unmapped_count,
            0
        );
        assert!(
            !bob.has_pending_retry_work(),
            "pruning the stale candidate should leave no background retry work"
        );
    }
}
