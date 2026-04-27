use super::*;

const GROUP_OUTER_SUBSCRIPTION_ID: &str = "ndr-group-outer";

impl AppCore {
    pub(super) fn remember_recent_handshake_peer(
        &mut self,
        owner_hex: String,
        device_hex: String,
        now_secs: u64,
    ) {
        if self.threads.contains_key(&owner_hex) {
            self.recent_handshake_peers
                .retain(|_, peer| peer.owner_hex != owner_hex);
            return;
        }
        self.recent_handshake_peers.insert(
            device_hex.clone(),
            RecentHandshakePeer {
                owner_hex,
                device_hex,
                observed_at_secs: now_secs,
            },
        );
    }

    pub(super) fn tracked_peer_owner_hexes(&self) -> HashSet<String> {
        let mut owners = self
            .threads
            .keys()
            .filter(|chat_id| !is_group_chat_id(chat_id))
            .cloned()
            .collect::<HashSet<_>>();
        if let Some(chat_id) = self.active_chat_id.as_ref() {
            if !is_group_chat_id(chat_id) {
                owners.insert(chat_id.clone());
            }
        }
        if let Some(logged_in) = self.logged_in.as_ref() {
            let local_owner_hex = logged_in.owner_pubkey.to_hex();
            for group in self.groups.values() {
                for member in &group.members {
                    if member != &local_owner_hex {
                        owners.insert(member.clone());
                    }
                }
            }
        }
        owners
    }

    pub(super) fn protocol_owner_hexes(&self) -> HashSet<String> {
        let mut owners = self.tracked_peer_owner_hexes();
        owners.extend(
            self.recent_handshake_peers
                .values()
                .map(|peer| peer.owner_hex.clone()),
        );
        owners.extend(self.app_keys.keys().cloned());
        if let Some(logged_in) = self.logged_in.as_ref() {
            owners.insert(logged_in.owner_pubkey.to_hex());
        }
        owners
    }

    pub(super) fn schedule_tracked_peer_catch_up(&self, after: Duration) {
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(after).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::FetchTrackedPeerCatchUp,
            )));
        });
    }

    pub(super) fn schedule_pending_device_invite_poll(&mut self, after: Duration) {
        if !self.can_poll_pending_device_invites() {
            return;
        }
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        let token = self.device_invite_poll_token;
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(after).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::PollPendingDeviceInvites { token },
            )));
        });
    }

    pub(super) fn fetch_recent_messages_for_author(
        &self,
        author_pubkey: PublicKey,
        now: UnixSeconds,
        lookback_secs: u64,
    ) {
        let Some(client) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.client.clone())
        else {
            return;
        };
        let filter = build_direct_message_backfill_filter(
            [author_pubkey],
            now.get().saturating_sub(lookback_secs),
            DEVICE_INVITE_DISCOVERY_LIMIT,
        );
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            client.connect_with_timeout(Duration::from_secs(5)).await;
            if let Ok(events) = client
                .fetch_events(vec![filter], Some(Duration::from_secs(5)))
                .await
            {
                let collected = events.into_iter().collect::<Vec<_>>();
                if !collected.is_empty() {
                    let _ = tx.send(CoreMsg::Internal(Box::new(
                        InternalEvent::FetchCatchUpEvents(collected),
                    )));
                }
            }
        });
    }

    pub(super) fn fetch_recent_messages_for_tracked_peers(&self, now: UnixSeconds) {
        let direct_authors = self.direct_message_subscriptions.tracked_authors();
        let group_authors = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.ndr_runtime.group_known_sender_event_pubkeys())
            .unwrap_or_default();
        for author in direct_authors.into_iter().chain(group_authors) {
            self.fetch_recent_messages_for_author(author, now, CATCH_UP_LOOKBACK_SECS);
        }
    }

    pub(super) fn fetch_recent_protocol_state(&mut self) {
        let Some(client) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.client.clone())
        else {
            return;
        };
        let now = unix_now();
        let owners = self
            .protocol_owner_hexes()
            .into_iter()
            .filter_map(|hex| PublicKey::parse(&hex).ok())
            .collect::<Vec<_>>();
        let message_authors = self
            .direct_message_subscriptions
            .tracked_authors()
            .into_iter()
            .chain(
                self.logged_in
                    .as_ref()
                    .map(|logged_in| logged_in.ndr_runtime.group_known_sender_event_pubkeys())
                    .unwrap_or_default(),
            )
            .collect::<Vec<_>>();
        let filters = recent_protocol_filters(owners, message_authors, now);
        if filters.is_empty() {
            return;
        }
        self.push_debug_log(
            "protocol.catch_up.fetch",
            format!("filters={}", filters.len()),
        );

        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            client.connect_with_timeout(Duration::from_secs(5)).await;
            match client
                .fetch_events(filters, Some(Duration::from_secs(5)))
                .await
            {
                Ok(events) => {
                    let collected = events.into_iter().collect::<Vec<_>>();
                    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                        category: "protocol.catch_up.result".to_string(),
                        detail: format!("events={}", collected.len()),
                    })));
                    if !collected.is_empty() {
                        let _ = tx.send(CoreMsg::Internal(Box::new(
                            InternalEvent::FetchCatchUpEvents(collected),
                        )));
                    }
                }
                Err(error) => {
                    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                        category: "protocol.catch_up.error".to_string(),
                        detail: error.to_string(),
                    })));
                }
            }
        });
    }

    pub(super) fn fetch_pending_device_invites_for_local_owner(&mut self) {
        self.fetch_recent_protocol_state();
    }

    pub(super) fn start_notifications_loop(&self, client: Client) {
        let mut notifications = client.notifications();
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            loop {
                match notifications.recv().await {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::RelayEvent(
                            (*event).clone(),
                        ))));
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub(super) fn schedule_session_connect(&self) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        let client = logged_in.client.clone();
        let relay_urls = logged_in.relay_urls.clone();
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &relay_urls).await;
            client
                .connect_with_timeout(Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS))
                .await;
        });
    }

    pub(super) fn request_protocol_subscription_refresh(&mut self) {
        self.request_protocol_subscription_refresh_inner(false);
    }

    pub(super) fn request_protocol_subscription_refresh_forced(&mut self) {
        self.request_protocol_subscription_refresh_inner(true);
    }

    pub(super) fn request_protocol_subscription_refresh_inner(&mut self, _force: bool) {
        let Some((client, owners, group_authors)) = self.logged_in.as_ref().map(|logged_in| {
            (
                logged_in.client.clone(),
                self.protocol_owner_hexes()
                    .into_iter()
                    .filter_map(|hex| PublicKey::parse(&hex).ok())
                    .collect::<Vec<_>>(),
                logged_in
                    .ndr_runtime
                    .group_outer_subscription_plan()
                    .authors,
            )
        }) else {
            self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
            return;
        };

        // setup_user is idempotent on the NDR side, but each call still walks
        // every user record + JSON-serialises every session state to compute
        // the DM subscription author set. Calling it for N owners on every
        // chat tap was a 14 × 337 ms = 4.7 s hit on Android debug. Skip
        // owners we've already initialised — once an owner is set up it
        // stays set up for the lifetime of the AppCore.
        for owner in &owners {
            if !self.setup_user_done.insert(owner.to_hex()) {
                continue;
            }
            if let Some(logged_in) = self.logged_in.as_ref() {
                let _ = logged_in.ndr_runtime.setup_user(*owner);
            }
        }
        self.process_runtime_events();

        if !group_authors.is_empty() {
            let filter = Filter::new()
                .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
                .authors(group_authors.clone());
            let subid = GROUP_OUTER_SUBSCRIPTION_ID.to_string();
            self.protocol_subscription_runtime
                .active_subscriptions
                .insert(subid.clone());
            for author in group_authors {
                self.fetch_recent_messages_for_author(author, unix_now(), CATCH_UP_LOOKBACK_SECS);
            }
            self.runtime.spawn(async move {
                let _ = client
                    .subscribe_with_id(SubscriptionId::new(subid), vec![filter], None)
                    .await;
            });
        }

        // Only bump the refresh token + emit a debug log entry when the
        // computed plan has actually changed since the last emission.
        // Otherwise the log fills up with identical lines on every chat
        // action even though nothing on the relay side moved.
        let plan_summary =
            summarize_protocol_plan(self.compute_protocol_subscription_plan().as_ref());
        let plan_changed = self
            .protocol_subscription_runtime
            .last_emitted_plan_summary
            .as_ref()
            != Some(&plan_summary);
        if plan_changed {
            self.protocol_subscription_runtime.refresh_token = self
                .protocol_subscription_runtime
                .refresh_token
                .wrapping_add(1);
            self.protocol_subscription_runtime.last_emitted_plan_summary =
                Some(plan_summary.clone());
            self.push_debug_log("protocol.subscription.refresh", plan_summary);
        }
    }

    pub(super) fn compute_protocol_subscription_plan(&self) -> Option<ProtocolSubscriptionPlan> {
        let runtime_subscriptions = self
            .protocol_subscription_runtime
            .active_subscriptions
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let runtime_subscriptions = sorted_hexes(runtime_subscriptions);
        (!runtime_subscriptions.is_empty()).then_some(ProtocolSubscriptionPlan {
            runtime_subscriptions,
        })
    }

    pub(super) fn can_poll_pending_device_invites(&self) -> bool {
        self.logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_keys.is_some())
            .unwrap_or(false)
    }

    pub(super) fn known_message_author_hexes(&self) -> HashSet<String> {
        self.direct_message_subscriptions
            .tracked_authors()
            .into_iter()
            .chain(
                self.logged_in
                    .as_ref()
                    .map(|logged_in| logged_in.ndr_runtime.group_known_sender_event_pubkeys())
                    .unwrap_or_default(),
            )
            .map(|pubkey| pubkey.to_hex())
            .collect()
    }

    pub(super) fn has_seen_event(&self, event_id: &str) -> bool {
        self.seen_event_ids.contains(event_id)
    }

    pub(super) fn remember_event(&mut self, event_id: String) {
        if !self.seen_event_ids.insert(event_id.clone()) {
            return;
        }

        self.seen_event_order.push_back(event_id);
        while self.seen_event_order.len() > MAX_SEEN_EVENT_IDS {
            if let Some(expired) = self.seen_event_order.pop_front() {
                self.seen_event_ids.remove(&expired);
            }
        }
    }
}
