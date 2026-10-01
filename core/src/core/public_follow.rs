use super::*;
use futures_util::{stream, StreamExt};
use nostr::{JsonUtil, Tag};
use nostr_sdk::prelude::ReqExitPolicy;

impl AppCore {
    pub(super) fn set_public_follow(&mut self, target: &str, following: bool) {
        if self.pending_follow.is_some() || PublicKey::from_hex(target).is_err() {
            return;
        }
        let Some(login) = self.logged_in.as_ref() else {
            return;
        };
        if login.owner_keys.is_none() {
            self.state.toast = Some("Use your main device to change public follows.".into());
            self.emit_state();
            return;
        }
        if login.relay_urls.is_empty() || login.owner_pubkey.to_hex() == target {
            return;
        }
        let owner = login.owner_pubkey;
        let client = login.client.clone();
        let urls = login.relay_urls.clone();
        let request_id = uuid::Uuid::new_v4().to_string();
        self.pending_follow = Some((
            request_id.clone(),
            owner.to_hex(),
            target.to_string(),
            following,
        ));
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(5)).await;
            let relays = client.relays().await.into_values().collect::<Vec<_>>();
            let replies = stream::iter(relays)
                .map(|relay| async move {
                    relay
                        .fetch_events(
                            Filter::new().author(owner).kind(Kind::ContactList),
                            Duration::from_secs(8),
                            ReqExitPolicy::ExitOnEOSE,
                        )
                        .await
                })
                .buffer_unordered(8)
                .collect::<Vec<_>>()
                .await;
            let mut complete = false;
            let mut events = Vec::new();
            for reply in replies {
                if let Ok(reply) = reply {
                    complete = true;
                    events.extend(reply.iter().cloned());
                }
            }
            let result = if complete {
                Ok(events)
            } else {
                Err("Couldn’t load your follow list. Try again.".into())
            };
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::FollowUpdateReady { request_id, result },
            )));
        });
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn finish_follow_update(
        &mut self,
        request_id: &str,
        result: Result<Vec<Event>, String>,
    ) {
        let Some((id, owner, target, following)) = self.pending_follow.clone() else {
            return;
        };
        if id != request_id {
            return;
        }
        self.pending_follow = None;
        let Some(keys) = self
            .logged_in
            .as_ref()
            .filter(|login| login.owner_pubkey.to_hex() == owner)
            .and_then(|login| login.owner_keys.clone())
        else {
            return;
        };
        let result = result.and_then(|mut events| {
            if let Some(event) = self
                .user_discovery
                .follow_event_json
                .as_deref()
                .and_then(|json| Event::from_json(json).ok())
            {
                events.push(event);
            }
            let latest =
                super::user_discovery::newest_verified_events_by_author(events, Kind::ContactList)
                    .into_iter()
                    .find(|event| event.pubkey == keys.public_key());
            if self.user_discovery.follow_event_id.is_some()
                && latest.as_ref().is_none_or(|event| {
                    event.created_at.as_secs() < self.user_discovery.follow_created_at_secs
                        || (event.created_at.as_secs()
                            == self.user_discovery.follow_created_at_secs
                            && self
                                .user_discovery
                                .follow_event_id
                                .as_ref()
                                .is_some_and(|id| event.id.to_hex() > *id))
                })
            {
                return Err("Couldn’t load your latest follow list. Try again.".into());
            }
            build_follow_update(&keys, latest.as_ref(), &target, following, unix_now().get())
                .map_err(|error| error.to_string())
        });
        match result {
            Ok(event) => {
                if self.publish_runtime_event(event.clone(), "public-follow", None) {
                    let mut cache = self.user_discovery.clone();
                    cache.owner_pubkey_hex = Some(owner);
                    cache.follow_event_id = Some(event.id.to_hex());
                    cache.follow_event_json = Some(event.as_json());
                    cache.follow_created_at_secs = event.created_at.as_secs();
                    cache.social_graph = super::user_discovery_graph::update_people_graph(
                        keys.public_key(),
                        Some(&event),
                        &[],
                        &[],
                        Vec::new(),
                        cache.social_graph.as_deref(),
                        unix_now().get(),
                    );
                    cache.users.clear();
                    for (position, tag) in event.tags.iter().enumerate() {
                        let values = tag.as_slice();
                        if values.first().map(String::as_str) != Some("p") {
                            continue;
                        }
                        let Some(peer) = values
                            .get(1)
                            .and_then(|value| PublicKey::from_hex(value).ok())
                        else {
                            continue;
                        };
                        cache.users.insert(
                            peer.to_hex(),
                            DiscoveredUserRecord {
                                owner_pubkey_hex: peer.to_hex(),
                                follow_position: position as u32,
                                petname: values.get(3).cloned().filter(|s| !s.trim().is_empty()),
                            },
                        );
                    }
                    let cache_saved = match self.app_store.replace_user_discovery(&cache) {
                        Ok(()) => true,
                        Err(error) => {
                            self.push_debug_log("follow.persist.error", error.to_string());
                            false
                        }
                    };
                    // The signed event is already in the durable outbox. Reflect it in
                    // memory even if this derived cache write fails, and report the partial save.
                    self.user_discovery = cache;
                    self.refresh_social_graph();
                    // An older in-flight discovery must not replace the intentional edit.
                    self.user_discovery_runtime.token =
                        self.user_discovery_runtime.token.wrapping_add(1);
                    self.user_discovery_runtime.in_flight = false;
                    self.refresh_people_syncing();
                    self.bump_user_discovery_revision();
                    self.state.toast = Some(
                        if !cache_saved {
                            "Follow change queued, but couldn’t save the local profile. Try refreshing."
                        } else if following {
                            "Public follow saved"
                        } else {
                            "Public follow removed"
                        }
                        .into(),
                    );
                } else {
                    self.state.toast = Some("Couldn’t save your follow list. Try again.".into());
                }
            }
            Err(error) => self.state.toast = Some(error),
        }
        self.rebuild_state();
        self.emit_state();
    }
}

fn build_follow_update(
    keys: &Keys,
    previous: Option<&Event>,
    target: &str,
    following: bool,
    now: u64,
) -> anyhow::Result<Event> {
    let target_key = PublicKey::from_hex(target)?;
    if let Some(previous) = previous {
        anyhow::ensure!(
            previous.pubkey == keys.public_key()
                && previous.kind == Kind::ContactList
                && previous.verify().is_ok(),
            "Invalid follow list"
        );
        anyhow::ensure!(
            previous.created_at.as_secs() <= now.saturating_add(600),
            "Follow list has a future date"
        );
    }
    let mut tags = previous
        .map(|event| event.tags.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let is_target = |tag: &Tag| {
        let values = tag.as_slice();
        values.first().map(String::as_str) == Some("p")
            && values
                .get(1)
                .and_then(|value| PublicKey::from_hex(value).ok())
                == Some(target_key)
    };
    if following {
        if !tags.iter().any(is_target) {
            tags.push(Tag::public_key(target_key));
        }
    } else {
        tags.retain(|tag| !is_target(tag));
    }
    Ok(EventBuilder::new(
        Kind::ContactList,
        previous
            .map(|event| event.content.as_str())
            .unwrap_or_default(),
    )
    .tags(tags)
    .custom_created_at(Timestamp::from_secs(
        now.max(
            previous
                .map(|event| event.created_at.as_secs().saturating_add(1))
                .unwrap_or_default(),
        ),
    ))
    .sign_with_keys(keys)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_follow_preserves_other_contacts_petnames_and_content() {
        let keys = Keys::generate();
        let alice = Keys::generate().public_key();
        let bob = Keys::generate().public_key();
        let contact = Tag::parse(["p", &alice.to_hex(), "wss://example.com", "Alice"]).unwrap();
        let extra = Tag::parse(["alt", "My contacts"]).unwrap();
        let original = EventBuilder::new(Kind::ContactList, "legacy relay preferences")
            .tags([contact.clone(), extra.clone()])
            .custom_created_at(Timestamp::from_secs(10))
            .sign_with_keys(&keys)
            .unwrap();
        let followed =
            build_follow_update(&keys, Some(&original), &bob.to_hex(), true, 10).unwrap();
        assert!(followed.verify().is_ok());
        assert_eq!(followed.created_at.as_secs(), 11);
        assert_eq!(followed.content, original.content);
        assert_eq!(followed.tags.len(), 3);
        assert!(followed.tags.iter().any(|tag| tag == &contact));
        let repeated =
            build_follow_update(&keys, Some(&followed), &bob.to_hex(), true, 12).unwrap();
        assert_eq!(repeated.tags.len(), 3);
        let unfollowed =
            build_follow_update(&keys, Some(&repeated), &bob.to_hex(), false, 13).unwrap();
        assert_eq!(
            unfollowed.tags.iter().cloned().collect::<Vec<_>>(),
            vec![contact, extra]
        );
    }
}
