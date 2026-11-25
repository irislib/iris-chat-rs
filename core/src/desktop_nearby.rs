use crate::FfiApp;
use std::sync::{Arc, RwLock};

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DesktopNearbyPeerSnapshot {
    pub id: String,
    pub name: String,
    pub owner_pubkey_hex: Option<String>,
    pub picture_url: Option<String>,
    pub profile_event_id: Option<String>,
    pub last_seen_secs: u64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DesktopNearbySnapshot {
    pub visible: bool,
    pub status: String,
    pub peers: Vec<DesktopNearbyPeerSnapshot>,
}

#[uniffi::export(callback_interface)]
pub trait DesktopNearbyObserver: Send + Sync + 'static {
    fn desktop_nearby_changed(&self, snapshot: DesktopNearbySnapshot);
}

/// Compatibility facade for desktop callers that still own the Nearby UI lifecycle.
/// FIPS owns discovery and transport; peer snapshots arrive through `AppUpdate`.
#[derive(uniffi::Object)]
pub struct FfiDesktopNearby {
    observer: Arc<dyn DesktopNearbyObserver>,
    snapshot: RwLock<DesktopNearbySnapshot>,
}

#[uniffi::export]
impl FfiDesktopNearby {
    #[uniffi::constructor]
    pub fn new(_app: Arc<FfiApp>, observer: Box<dyn DesktopNearbyObserver>) -> Arc<Self> {
        Arc::new(Self {
            observer: observer.into(),
            snapshot: RwLock::new(DesktopNearbySnapshot {
                visible: false,
                status: "Off".to_string(),
                peers: Vec::new(),
            }),
        })
    }

    pub fn start(&self, _local_name: String) {
        self.set_visible(true);
    }

    pub fn stop(&self) {
        self.set_visible(false);
    }

    pub fn snapshot(&self) -> DesktopNearbySnapshot {
        match self.snapshot.read() {
            Ok(snapshot) => snapshot.clone(),
            Err(poison) => poison.into_inner().clone(),
        }
    }

    pub fn publish(
        &self,
        _event_id: String,
        _kind: u32,
        _created_at_secs: u64,
        _event_json: String,
    ) {
    }
}

impl FfiDesktopNearby {
    /// Reports visibility only. Peers come from `AppUpdate::NearbyPeersChanged`,
    /// so shells must not let this empty peer list replace theirs. Repeated
    /// start/stop calls are no-ops: shells call them on every state sync.
    fn set_visible(&self, visible: bool) {
        let snapshot = DesktopNearbySnapshot {
            visible,
            status: if visible { "Visible" } else { "Off" }.to_string(),
            peers: Vec::new(),
        };
        let mut current = match self.snapshot.write() {
            Ok(current) => current,
            Err(poison) => poison.into_inner(),
        };
        if current.visible == visible {
            return;
        }
        *current = snapshot.clone();
        drop(current);
        self.observer.desktop_nearby_changed(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<DesktopNearbySnapshot>>);

    impl DesktopNearbyObserver for Arc<Recorder> {
        fn desktop_nearby_changed(&self, snapshot: DesktopNearbySnapshot) {
            self.0.lock().unwrap().push(snapshot);
        }
    }

    #[test]
    fn repeated_start_does_not_reemit_empty_peer_lists() {
        let recorder = Arc::new(Recorder::default());
        let nearby = FfiDesktopNearby {
            observer: Arc::new(recorder.clone()),
            snapshot: RwLock::new(DesktopNearbySnapshot {
                visible: false,
                status: "Off".to_string(),
                peers: Vec::new(),
            }),
        };
        nearby.start("a".into());
        nearby.start("a".into());
        nearby.start("a".into());
        nearby.stop();
        nearby.stop();
        nearby.start("a".into());
        let visibility = recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|snapshot| snapshot.visible)
            .collect::<Vec<_>>();
        assert_eq!(visibility, vec![true, false, true]);
    }
}
