use super::*;
use nostr::JsonUtil;
use std::collections::VecDeque;

const MAX_PENDING_EVENTS: usize = 256;
const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;

/// iOS grants execution for a Bluetooth callback, not an always-running core.
/// The native bridge holds a background task until the matching storage barrier.
#[derive(Default)]
pub(super) struct BleBackground {
    enabled: bool,
    generation: u64,
    active: Option<(u64, Instant)>,
    pending: VecDeque<InternalEvent>,
    pending_bytes: usize,
}

fn event_size(event: &InternalEvent) -> Option<usize> {
    Some(match event {
        InternalEvent::MeshEvent(event) => event.as_json().len(),
        InternalEvent::FipsNearbyPacket { data, .. }
        | InternalEvent::DeviceSyncPacket { data, .. } => data.len() + 128,
        InternalEvent::FipsNearbyPeersChanged { peers, .. } => peers.len() * 256,
        InternalEvent::FlushPendingDeliveredReceipts { .. }
        | InternalEvent::RetryReadyProtocolWork { .. } => 64,
        _ => return None,
    })
}

impl AppCore {
    fn owns_background_ble(&self, ownership: &HostBleOwnership) -> bool {
        !ownership.is_cancelled()
            && self
                .host_ble_ownership
                .as_ref()
                .is_some_and(|current| current.matches(ownership))
    }

    pub(super) fn keep_background_ble(&self) -> bool {
        self.ble_background.enabled
            && self.preferences.nearby_enabled
            && self.preferences.nearby_bluetooth_enabled
            && self
                .host_ble_ownership
                .as_ref()
                .is_some_and(|owner| !owner.is_cancelled())
    }

    pub(super) fn set_host_ble_background(
        &mut self,
        ownership: &HostBleOwnership,
        enabled: bool,
    ) -> bool {
        if !self.owns_background_ble(ownership) {
            return false;
        }
        self.ble_background.enabled = enabled;
        if !enabled {
            self.reset_background_ble();
        }
        true
    }

    pub(super) fn set_host_ble_activity(
        &mut self,
        ownership: &HostBleOwnership,
        generation: u64,
        active: bool,
    ) -> bool {
        if !self.owns_background_ble(ownership) || !self.keep_background_ble() {
            return false;
        }
        if active {
            if generation <= self.ble_background.generation {
                return false;
            }
            self.ble_background.generation = generation;
            // A missed native completion must never leave the storage gate open.
            self.ble_background.active =
                Some((generation, Instant::now() + Duration::from_secs(8)));
            self.resume_background_ble_events();
        } else {
            if self.ble_background.active.map(|(id, _)| id) != Some(generation) {
                return false;
            }
            self.ble_background.active = None;
            if self.suspended {
                self.persist_best_effort();
                if let Err(error) = self.app_store.prepare_for_suspend() {
                    self.push_debug_log("storage.ble_suspend.error", error.to_string());
                }
            }
        }
        true
    }

    pub(super) fn admit_background_ble_event(&self, event: &InternalEvent) -> bool {
        self.keep_background_ble()
            && self
                .ble_background
                .active
                .is_some_and(|(_, until)| Instant::now() < until)
            && event_size(event).is_some()
    }

    pub(super) fn defer_background_ble_event(&mut self, event: InternalEvent) {
        if self.keep_background_ble() {
            if let Some(size) = event_size(&event) {
                if self.ble_background.pending.len() < MAX_PENDING_EVENTS
                    && self.ble_background.pending_bytes.saturating_add(size) <= MAX_PENDING_BYTES
                {
                    self.ble_background.pending_bytes += size;
                    self.ble_background.pending.push_back(event);
                }
                return;
            }
        }
        self.defer_remote_signer_event(event);
    }

    pub(super) fn resume_background_ble_events(&mut self) {
        let pending = std::mem::take(&mut self.ble_background.pending);
        self.ble_background.pending_bytes = 0;
        for event in pending {
            self.handle_internal(event);
        }
    }

    pub(super) fn reset_background_ble(&mut self) {
        self.ble_background = BleBackground::default();
    }

    pub(super) fn foreground_background_ble(&mut self) {
        self.ble_background.active = None;
        self.resume_background_ble_events();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fips_core::transport::ble::host::HostBleIo;

    fn fixture() -> (AppCore, HostBleOwnership, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            dir.path().to_string_lossy().into(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        core.preferences.nostr_relay_urls.clear();
        core.start_primary_session(Keys::generate(), Keys::generate(), false, false)
            .unwrap();
        core.preferences.nearby_enabled = true;
        core.preferences.nearby_bluetooth_enabled = true;
        let (io, _) = core
            .runtime
            .block_on(async { HostBleIo::channel("mobile", "wake-test", 8) })
            .unwrap();
        let attachment = super::super::HostBleAttachment::new(io).with_background(true);
        let ownership = attachment.ownership();
        core.attach_host_ble(attachment).unwrap();
        core.handle_action(AppAction::AppBackgrounded);
        core.prepare_for_suspend();
        (core, ownership, dir)
    }

    fn note(body: &str) -> Event {
        EventBuilder::new(Kind::TextNote, body)
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn ble_wake_bounds_processing_and_drops_relay_work_until_foreground() {
        let (mut core, owner, _dir) = fixture();
        assert!(core.suspended);
        assert!(!core.app_in_foreground);
        let queued = note("between wakes");
        core.handle_internal(InternalEvent::MeshEvent(queued.clone()));
        assert!(!core.has_seen_event(&queued.id.to_hex()));
        assert!(core.set_host_ble_activity(&owner, 1, true));
        assert!(core.has_seen_event(&queued.id.to_hex()));
        let unrelated = note("relay");
        core.handle_internal(InternalEvent::RelayEvent(unrelated.clone()));
        assert!(!core.has_seen_event(&unrelated.id.to_hex()));
        let (reply_tx, reply_rx) = flume::bounded(1);
        core.handle_messages(vec![CoreMsg::HostBleActivity {
            ownership: owner.clone(),
            generation: 1,
            active: false,
            reply_tx,
        }]);
        assert!(reply_rx.recv().unwrap());
        assert!(core.suspended);
        let later = note("next wake");
        core.handle_internal(InternalEvent::MeshEvent(later.clone()));
        assert!(!core.has_seen_event(&later.id.to_hex()));
        assert!(
            !core.set_host_ble_activity(&owner, 1, true),
            "replayed begin cannot reopen storage"
        );
        assert!(core.set_host_ble_activity(&owner, 2, true));
        assert!(core.has_seen_event(&later.id.to_hex()));
        assert!(
            !core.set_host_ble_activity(&owner, 1, false),
            "late completion cannot close a newer wake"
        );
        assert_eq!(core.ble_background.active.unwrap().0, 2);
        core.handle_action(AppAction::AppForegrounded);
        assert!(!core.set_host_ble_activity(&owner, 2, false));
        assert!(
            !core.suspended,
            "late completion cannot re-suspend the visible app"
        );
    }

    #[test]
    fn ble_wake_expires_and_cannot_cross_attachment_or_disabled_preferences() {
        let (mut core, owner, _dir) = fixture();
        assert!(core.set_host_ble_activity(&owner, 1, true));
        core.ble_background.active = Some((1, Instant::now() - Duration::from_secs(1)));
        let event = note("expired wake");
        core.handle_internal(InternalEvent::MeshEvent(event.clone()));
        assert!(!core.has_seen_event(&event.id.to_hex()));
        for _ in 0..MAX_PENDING_EVENTS * 2 {
            core.handle_internal(InternalEvent::MeshEvent(event.clone()));
        }
        assert_eq!(core.ble_background.pending.len(), MAX_PENDING_EVENTS);
        core.preferences.nearby_bluetooth_enabled = false;
        assert!(!core.set_host_ble_activity(&owner, 2, true));
        core.preferences.nearby_bluetooth_enabled = true;
        owner.cancel();
        assert!(!core.set_host_ble_activity(&owner, 2, true));
        core.detach_owned_host_ble(&owner);
        assert!(core.ble_background.pending.is_empty());
        assert!(!core.ble_background.enabled);
    }
}
