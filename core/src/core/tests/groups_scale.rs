// The transport is deterministic; membership, encryption, subscriptions, UI rows,
// retry queues and SQLite writes all use the same paths as the native clients.
struct GroupScaleDevice {
    owner: usize,
    keys: Keys,
    core: AppCore,
    _dir: tempfile::TempDir,
    delivered: HashSet<nostr::EventId>,
    online: bool,
}

struct GroupScaleFarm {
    owners: Vec<Keys>,
    devices: Vec<GroupScaleDevice>,
    history: Vec<Event>,
    published: HashSet<nostr::EventId>,
    deliveries: usize,
    duplicates: usize,
    now: u64,
    publish_counts: BTreeMap<String, usize>,
    transport_time: Duration,
    subscription_time: Duration,
}

impl GroupScaleFarm {
    fn new(device_counts: &[usize]) -> Self {
        let owners = device_counts
            .iter()
            .map(|_| Keys::generate())
            .collect::<Vec<_>>();
        let mut devices = Vec::new();
        let mut history = Vec::new();
        let now = unix_now().get();
        for (owner_index, count) in device_counts.iter().copied().enumerate() {
            let owner = &owners[owner_index];
            let keys = (0..count).map(|_| Keys::generate()).collect::<Vec<_>>();
            history.push(
                AppKeys::new(
                    keys.iter()
                        .map(|key| DeviceEntry::new(key.public_key(), now))
                        .collect(),
                )
                .get_event_at(owner.public_key(), now)
                .sign_with_keys(owner)
                .expect("signed owner device list"),
            );
            for (slot, key) in keys.into_iter().enumerate() {
                let (mut core, _, dir) =
                    logged_in_test_core_with_updates("group-scale", owner, &key);
                // Linked devices have their own credentials, not the account's
                // secret key. Authorization comes from the signed device list.
                if slot > 0 {
                    core.logged_in.as_mut().unwrap().owner_keys = None;
                }
                let invite = core
                    .protocol_engine
                    .as_ref()
                    .unwrap()
                    .local_invite()
                    .unwrap();
                history.push(
                    nostr_double_ratchet::invite_unsigned_event(&invite)
                        .unwrap()
                        .sign_with_keys(&key)
                        .unwrap(),
                );
                devices.push(GroupScaleDevice {
                    owner: owner_index,
                    keys: key,
                    core,
                    _dir: dir,
                    delivered: HashSet::new(),
                    online: true,
                });
            }
        }
        let published = history.iter().map(|event| event.id).collect();
        Self {
            owners,
            devices,
            history,
            published,
            deliveries: 0,
            duplicates: 0,
            now,
            publish_counts: BTreeMap::new(),
            transport_time: Duration::ZERO,
            subscription_time: Duration::ZERO,
        }
    }

    fn pump(&mut self) {
        for round in 0..100 {
            let before = (self.history.len(), self.deliveries);
            if round % 5 == 0 {
                eprintln!(
                    "pump round={round} events={} deliveries={}",
                    self.history.len(),
                    self.deliveries
                );
            }
            let device_count = self.devices.len();
            for (index, device) in self
                .devices
                .iter_mut()
                .enumerate()
                .filter(|(_, device)| device.online)
            {
                if device_count > 100 && index % 40 == 0 {
                    eprintln!(
                        "pump device={index}/{device_count} events={} deliveries={}",
                        self.history.len(),
                        self.deliveries
                    );
                }
                let events = sorted_pending_events_for_test(&device.core);
                device.core.enter_batch();
                for event in events {
                    if self.published.insert(event.id) {
                        let label = device
                            .core
                            .pending_relay_publishes
                            .get(&event.id.to_string())
                            .map(|pending| pending.label.clone())
                            .unwrap_or_default();
                        *self
                            .publish_counts
                            .entry(format!("{}:{label}", event.kind.as_u16()))
                            .or_default() += 1;
                        self.history.push(event.clone());
                    }
                    device.core.handle_relay_publish_finished(
                        event.id.to_string(),
                        true,
                        Vec::new(),
                        "fixture relay accepted".into(),
                    );
                }
                device.core.exit_batch();
                let subscription_start = std::time::Instant::now();
                let filters = device
                    .core
                    .compute_protocol_subscription_plan()
                    .map(|plan| build_protocol_subscription_filters(&plan))
                    .unwrap_or_default();
                self.subscription_time += subscription_start.elapsed();
                let transport_start = std::time::Instant::now();
                let matches = self
                    .history
                    .iter()
                    .filter(|event| {
                        !device.delivered.contains(&event.id)
                            && filters.iter().any(|filter| {
                                filter
                                    .match_event(event, nostr::filter::MatchEventOptions::default())
                            })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                self.transport_time += transport_start.elapsed();
                // The production catch-up loop processes at most 64 events in a batch.
                for events in matches.chunks(super::lifecycle::CATCH_UP_EVENT_PROCESS_CHUNK_SIZE) {
                    device.core.enter_batch();
                    for event in events {
                        device.delivered.insert(event.id);
                        device.core.handle_relay_event(event.clone());
                        self.deliveries += 1;
                        if self.deliveries.is_multiple_of(11) {
                            device.core.handle_relay_event(event.clone());
                            self.duplicates += 1;
                        }
                    }
                    device.core.exit_batch();
                }
            }
            if before == (self.history.len(), self.deliveries) {
                return;
            }
            assert!(
                round < 99,
                "group relay did not settle: events={} deliveries={}",
                self.history.len(),
                self.deliveries
            );
        }
    }

    fn retry(&mut self) {
        self.now = self.now.max(unix_now().get()).saturating_add(60);
        for device in self.devices.iter_mut().filter(|device| device.online) {
            let retry = device
                .core
                .protocol_engine
                .as_mut()
                .unwrap()
                .retry_pending_protocol(NdrUnixSeconds(self.now))
                .expect("retry queued fanout");
            device
                .core
                .process_protocol_engine_retry_batch("group-scale", retry);
        }
        self.pump();
    }

    fn add_members(&mut self, group_id: &str, members: std::ops::Range<usize>) {
        let count = members.len();
        let member_inputs = members
            .map(|index| self.owners[index].public_key().to_hex())
            .collect();
        let action_start = std::time::Instant::now();
        self.devices[0]
            .core
            .handle_action(AppAction::AddGroupMembers {
                group_id: group_id.into(),
                member_inputs,
            });
        eprintln!(
            "admin add {count} members action={:?}",
            action_start.elapsed()
        );
        self.pump();
    }

    fn send(&mut self, device: usize, chat_id: &str, body: &str) -> (String, Duration) {
        let action_start = std::time::Instant::now();
        self.devices[device]
            .core
            .handle_action(AppAction::SendMessage {
                chat_id: chat_id.into(),
                text: body.into(),
            });
        let action_time = action_start.elapsed();
        let core = &self.devices[device].core;
        let id = core
            .threads
            .get(chat_id)
            .and_then(|thread| thread.messages.iter().find(|message| message.body == body))
            .unwrap_or_else(|| {
                panic!(
                    "sender device {device} did not create message: {:?}",
                    core.state.toast
                )
            })
            .id
            .clone();
        (id, action_time)
    }

    fn assert_messages(&self, chat_id: &str, expected: &[(String, String)]) {
        let mut failures = Vec::new();
        for (index, device) in self
            .devices
            .iter()
            .enumerate()
            .filter(|(_, device)| device.online)
        {
            let messages = device
                .core
                .threads
                .get(chat_id)
                .map(|thread| thread.messages.as_slice())
                .unwrap_or_default();
            let actual = messages
                .iter()
                .filter(|message| message.body.starts_with("scale-"))
                .map(|message| (&message.id, &message.body))
                .collect::<Vec<_>>();
            assert_eq!(
                actual.len(),
                expected.len(),
                "unexpected extra logical message on device {index}"
            );
            for (id, body) in expected {
                let matching = messages
                    .iter()
                    .filter(|message| message.id == *id && message.body == *body)
                    .count();
                if matching != 1 {
                    failures.push(format!(
                        "device={index} owner={} body={body} copies={matching}",
                        device.owner
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} missing/duplicate logical messages; first failures: {:?}",
            failures.len(),
            &failures[..failures.len().min(12)]
        );
    }
}

fn group_scale_scenario(device_counts: &[usize]) {
    let started = std::time::Instant::now();
    let mut farm = GroupScaleFarm::new(device_counts);
    farm.pump();
    let offline = farm.devices.len() - 1;
    farm.devices[offline].online = false;
    // One listed linked device has not yet published its invitation. Its owner's
    // other device and all other members must still make progress.
    let late_key = farm.devices[offline].keys.public_key();
    let delayed_invite_index = farm
        .history
        .iter()
        .position(|event| {
            event.pubkey == late_key && event.kind.as_u16() as u32 == INVITE_EVENT_KIND
        })
        .unwrap();
    let delayed_invite = farm.history.remove(delayed_invite_index);
    farm.devices[0].core.handle_action(AppAction::CreateGroup {
        name: "Large multi-device group".into(),
        member_inputs: Vec::new(),
    });
    let group_id = farm.devices[0]
        .core
        .groups
        .keys()
        .next()
        .expect("admin created group")
        .clone();
    let split = (farm.owners.len() - 1) * 9 / 10 + 1;
    farm.add_members(&group_id, 1..split);
    farm.add_members(&group_id, split..farm.owners.len());
    farm.retry();
    let chat_id = group_chat_id(&group_id);
    for (index, device) in farm
        .devices
        .iter_mut()
        .enumerate()
        .filter(|(_, device)| device.online)
    {
        let group = device.core.groups.get(&group_id).unwrap_or_else(|| {
            panic!(
                "device {index} missing group; debug={:?}",
                device
                    .core
                    .protocol_engine
                    .as_ref()
                    .unwrap()
                    .debug_snapshot()
            )
        });
        assert_eq!(
            group
                .members
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            farm.owners
                .iter()
                .map(|owner| ndr_owner_pubkey(owner.public_key()))
                .collect::<std::collections::BTreeSet<_>>(),
            "membership differs on device {index}"
        );
        device
            .core
            .handle_action(AppAction::SetMessageRequestAccepted {
                chat_id: chat_id.clone(),
            });
    }
    let setup_time = started.elapsed();
    eprintln!("group setup complete: {setup_time:?}");
    let first_round = std::time::Instant::now();
    let mut expected = Vec::new();
    let mut first_actions_total = Duration::ZERO;
    let mut first_action_max = Duration::ZERO;
    for owner in 0..farm.owners.len() {
        let index = farm
            .devices
            .iter()
            .position(|device| device.owner == owner && device.online)
            .unwrap();
        let body = format!("scale-member-{owner}");
        let (id, action_time) = farm.send(index, &chat_id, &body);
        first_actions_total += action_time;
        first_action_max = first_action_max.max(action_time);
        if farm.owners.len() > 100 && owner % 20 == 0 {
            eprintln!(
                "first send owner={owner}/{} action={action_time:?}",
                farm.owners.len()
            );
        }
        expected.push((id, body));
    }
    eprintln!("first send actions total={first_actions_total:?} max={first_action_max:?}");
    farm.pump();
    farm.retry();
    farm.assert_messages(&chat_id, &expected);
    let first_round_time = first_round.elapsed();
    eprintln!("first round complete: {first_round_time:?}");
    // These later sends also exercise another linked device's first sender-key
    // stream, so this timing is not a pure warm-session benchmark.
    let steady_start = std::time::Instant::now();
    let initial_events = farm.history.len();
    for round in 0..3 {
        let sender = round % (farm.devices.len() - 1);
        let body = format!("scale-steady-{round}");
        let (id, _) = farm.send(sender, &chat_id, &body);
        expected.push((id, body));
        farm.pump();
    }
    farm.assert_messages(&chat_id, &expected);
    let steady_time = steady_start.elapsed();
    let steady_events = farm.history.len() - initial_events;
    let recovery_start = std::time::Instant::now();
    // Drop/reload the offline recipient's real protocol state from SQLite, then
    // rediscover its invitation and replay retained relay history.
    let device = &mut farm.devices[offline];
    device.core.protocol_engine = None;
    let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        device.core.app_store.shared(),
        farm.owners[device.owner].public_key().to_hex(),
        device.keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    device.core.protocol_engine = Some(
        ProtocolEngine::load_or_create_for_local_device(
            storage,
            farm.owners[device.owner].public_key(),
            &device.keys,
        )
        .expect("restore offline protocol from SQLite"),
    );
    device.online = true;
    farm.history.push(delayed_invite);
    for _ in 0..3 {
        farm.pump();
        farm.retry();
    }
    farm.assert_messages(&chat_id, &expected);
    let after_restart = "scale-after-restart".to_string();
    let (id, _) = farm.send(offline, &chat_id, &after_restart);
    expected.push((id, after_restart));
    farm.pump();
    farm.retry();
    farm.assert_messages(&chat_id, &expected);
    // Verify actual durable rows, not just transient UI projection or totals.
    for (index, device) in farm.devices.iter_mut().enumerate() {
        let mut rows = device
            .core
            .app_store
            .load_thread(&chat_id, 40)
            .unwrap()
            .expect("persisted group thread")
            .messages;
        let mut oldest = rows.first().map(|message| message.id.clone());
        while let Some(before) = oldest {
            let page = device
                .core
                .app_store
                .load_messages_before(&chat_id, &before, 40)
                .unwrap();
            oldest = page.first().map(|message| message.id.clone());
            rows.extend(page);
        }
        assert_eq!(
            rows.iter()
                .filter(|message| message.body.starts_with("scale-"))
                .count(),
            expected.len(),
            "unexpected extra durable logical message on device {index}"
        );
        for (id, body) in &expected {
            assert_eq!(
                rows.iter()
                    .filter(|message| message.id == *id && message.body == *body)
                    .count(),
                1,
                "durable logical message {body} differs on device {index}"
            );
        }
    }
    eprintln!(
        "published kinds/labels: {:?}; fixture filtering={:?}; fixture subscription queries={:?}",
        farm.publish_counts, farm.transport_time, farm.subscription_time
    );
    eprintln!("group scale: owners={} devices={} logical_messages={} delivered_rows={} missing=0 duplicates=0 relay_events={} relay_deliveries={} mirrored_duplicates={} setup={setup_time:?} first_round={first_round_time:?} steady_3={steady_time:?} steady_events={steady_events} recovery={:?} total={:?}",
        farm.owners.len(), farm.devices.len(), expected.len(), farm.devices.len() * expected.len(), farm.history.len(), farm.deliveries, farm.duplicates, recovery_start.elapsed(), started.elapsed());
}

#[test]
fn appcore_group_multi_device_fanout_recovery() {
    group_scale_scenario(&[2, 1, 2, 3, 2]);
}

#[test]
#[ignore = "101 owners/182 SQLite-backed devices; use TOKIO_WORKER_THREADS=1 IRIS_FIPS_WEBSOCKET_SEED_URLS='' and --test-threads=1"]
fn appcore_group_100_members_multi_device_scale() {
    assert_eq!(
        std::env::var("TOKIO_WORKER_THREADS").as_deref(),
        Ok("1"),
        "use TOKIO_WORKER_THREADS=1 for the 182-device process fixture"
    );
    assert!(
        std::env::var("IRIS_FIPS_WEBSOCKET_SEED_URLS").is_ok_and(|seeds| seeds.trim().is_empty()),
        "use IRIS_FIPS_WEBSOCKET_SEED_URLS='' to keep generated profiles off public transports"
    );
    let mut device_counts = vec![2];
    device_counts.extend(std::iter::repeat_n(1, 40));
    device_counts.extend(std::iter::repeat_n(2, 40));
    device_counts.extend(std::iter::repeat_n(3, 20));
    group_scale_scenario(&device_counts);
}

#[test]
fn appcore_group_own_relay_echo_does_not_start_sender_key_repair() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let storage = Arc::new(InMemoryStorage::new());
    let mut engine = test_protocol_engine_with_storage(&owner, &device, storage.clone());
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    let group = engine
        .create_group("echo".into(), Vec::new(), unix_now())
        .unwrap()
        .snapshot
        .unwrap();
    let sent = engine
        .send_group_payload(
            &group.group_id,
            b"already displayed locally".to_vec(),
            Some("echo".into()),
        )
        .unwrap();
    let event =
        sender_key_outer_events_for_engine(&engine, &sent.effects, &sent.event_ids)[0].clone();
    for restored in [false, true] {
        if restored {
            engine = ProtocolEngine::load_or_create_for_local_device(
                storage.clone(),
                owner.public_key(),
                &device,
            )
            .unwrap();
        }
        let received = engine.process_group_outer_event(&event).unwrap();
        assert!(
            received.consumed
                && !received.pending
                && received.events.is_empty()
                && received.effects.is_empty(),
            "our own relay echo must not decrypt, display again, or request key repair"
        );
        let snapshot = engine.debug_snapshot();
        assert_eq!(snapshot.pending_group_sender_key_message_count, 0);
        assert_eq!(snapshot.pending_group_sender_key_repair_count, 0);
    }
}

#[test]
fn appcore_group_multi_device_sender_publishes_one_shared_ciphertext() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling = Keys::generate();
    let mut sender = test_protocol_engine(&owner, &device);
    let mut receiver = test_protocol_engine(&owner, &sibling);
    for engine in [&mut sender, &mut receiver] {
        observe_peer_appkeys_for_test(
            engine,
            &owner,
            &[device.public_key(), sibling.public_key()],
            unix_now().get(),
        );
    }
    let invite = receiver.local_invite().unwrap();
    let event = nostr_double_ratchet::invite_unsigned_event(&invite)
        .unwrap()
        .sign_with_keys(&sibling)
        .unwrap();
    sender.observe_invite_event(&event).unwrap();
    let group = sender
        .create_group("siblings".into(), Vec::new(), unix_now())
        .unwrap();
    deliver_protocol_effects_without_feedback(&mut receiver, &group.effects);
    let group_id = group.snapshot.unwrap().group_id;
    let sent = sender
        .send_group_payload(
            &group_id,
            b"one shared message".to_vec(),
            Some("shared".into()),
        )
        .unwrap();
    assert_eq!(sender_key_outer_count(&sender, &sent.effects, &sent.event_ids), 1,
        "remote and sibling copies use one shared sender-key event, not independently randomized wrappers");
    let received = deliver_protocol_effects_without_feedback(&mut receiver, &sent.effects);
    assert_eq!(received.iter().filter(|event| matches!(event, GroupIncomingEvent::Message(message) if message.body == b"one shared message")).count(), 1);
}

#[test]
fn appcore_group_control_journal_replay_does_not_repeat_sibling_fanout_within_batch() {
    let mut farm = GroupScaleFarm::new(&[1, 2]);
    farm.pump();
    farm.devices[0]
        .core
        .create_group("batch replay", &[farm.owners[1].public_key().to_hex()]);
    let group_id = farm.devices[0].core.groups.keys().next().unwrap().clone();
    farm.pump();
    farm.devices[0]
        .core
        .update_group_name(&group_id, "renamed once");
    let events = sorted_pending_events_for_test(&farm.devices[0].core);
    let receiver = &mut farm.devices[1].core;
    let filters = build_protocol_subscription_filters(
        &receiver.compute_protocol_subscription_plan().unwrap(),
    );
    receiver.enter_batch();
    for event in events.into_iter().filter(|event| {
        filters
            .iter()
            .any(|filter| filter.match_event(event, nostr::filter::MatchEventOptions::default()))
    }) {
        receiver.handle_relay_event(event);
    }
    assert!(
        !receiver.pending_decrypted_delivery_acks.is_empty(),
        "application awaits durable batch commit"
    );
    let pending = receiver.pending_relay_publishes.len();
    let replay = receiver
        .protocol_engine
        .as_mut()
        .unwrap()
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
        .unwrap();
    assert!(
        !replay.direct_messages.is_empty(),
        "uncommitted delivery journal is still replayable"
    );
    receiver.process_protocol_engine_retry_batch("batch replay regression", replay);
    assert_eq!(
        receiver.pending_relay_publishes.len(),
        pending,
        "a delivered control awaiting commit must not be applied and forwarded again"
    );
    receiver.exit_batch();
    assert!(receiver.pending_decrypted_delivery_acks.is_empty());
    assert_eq!(
        receiver
            .protocol_engine
            .as_ref()
            .unwrap()
            .pending_decrypted_deliveries_len_for_test(),
        0
    );
    assert_eq!(receiver.groups[&group_id].name, "renamed once");
}

#[test]
fn appcore_group_foreign_recipient_never_enters_group_fallback() {
    let mut farm = GroupScaleFarm::new(&[1, 2]);
    farm.pump();
    farm.devices[0].core.create_group(
        "addressed distribution",
        &[farm.owners[1].public_key().to_hex()],
    );
    farm.pump();
    let peer = farm.owners[1].public_key();
    let sent = farm.devices[0]
        .core
        .protocol_engine
        .as_mut()
        .unwrap()
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "for both linked devices",
            None,
            UnixSeconds(unix_now().get()),
        )
        .unwrap();
    let target = farm.devices[2].keys.public_key();
    let event = sent
        .effects
        .into_iter()
        .map(|ProtocolEffect::Publish(publish)| publish.event)
        .find(|event| {
            event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
                && event
                    .tags
                    .public_keys()
                    .any(|recipient| *recipient == target)
        })
        .expect("signed delivery for the other linked device");
    let event_id = event.id.to_string();
    let receiver = &mut farm.devices[1].core;
    receiver.handle_relay_event(event);
    assert!(
        !receiver.event_transport_channels.contains_key(&event_id),
        "foreign recipient is discarded before group/direct dispatch"
    );
    let snapshot = receiver.protocol_engine.as_ref().unwrap().debug_snapshot();
    assert_eq!(snapshot.pending_inbound_count, 0);
    assert_eq!(snapshot.pending_group_sender_key_message_count, 0);
}
