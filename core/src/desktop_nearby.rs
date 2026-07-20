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
    fn set_visible(&self, visible: bool) {
        let snapshot = DesktopNearbySnapshot {
            visible,
            status: if visible { "Visible" } else { "Off" }.to_string(),
            peers: Vec::new(),
        };
        match self.snapshot.write() {
            Ok(mut current) => *current = snapshot.clone(),
            Err(poison) => *poison.into_inner() = snapshot.clone(),
        }
        self.observer.desktop_nearby_changed(snapshot);
    }
}
