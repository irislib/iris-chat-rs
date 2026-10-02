use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flume::Sender;

use crate::{CoreMsg, FfiApp, FipsBleCommand, FipsBleEvent};

#[derive(uniffi::Object)]
pub struct FfiFipsBle {
    runtime: tokio::runtime::Runtime,
    adapter: Arc<fips_core::transport::ble::host::HostBleAdapter>,
    core_tx: Sender<CoreMsg>,
    closed: AtomicBool,
    ownership: crate::core::HostBleOwnership,
}

#[derive(Debug, Clone, uniffi::Error)]
pub enum FipsBleBridgeError {
    Initialization(String),
    Teardown(String),
}

impl std::fmt::Display for FipsBleBridgeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initialization(message) | Self::Teardown(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for FipsBleBridgeError {}

#[uniffi::export]
impl FfiFipsBle {
    #[uniffi::constructor]
    pub fn new(app: Arc<FfiApp>) -> Result<Arc<Self>, FipsBleBridgeError> {
        Self::connect(app.background_tx.clone(), Duration::from_secs(2))
    }

    pub fn next_command(&self, timeout_ms: u64) -> Option<FipsBleCommand> {
        let timeout = Duration::from_millis(timeout_ms.clamp(1, 60_000));
        self.runtime.block_on(async {
            tokio::time::timeout(timeout, self.adapter.next_command())
                .await
                .ok()
                .flatten()
                .map(Into::into)
        })
    }

    pub fn emit(&self, event: FipsBleEvent) -> bool {
        self.runtime
            .block_on(self.adapter.emit(event.into()))
            .is_ok()
    }

    pub fn detach(&self) -> Result<(), FipsBleBridgeError> {
        self.detach_with_timeout(Duration::from_secs(2))
    }
}

impl FfiFipsBle {
    pub(crate) fn connect(
        core_tx: Sender<CoreMsg>,
        timeout: Duration,
    ) -> Result<Arc<Self>, FipsBleBridgeError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_time()
            .build()
            .map_err(|error| {
                FipsBleBridgeError::Initialization(format!(
                    "Could not create the FIPS BLE runtime: {error}"
                ))
            })?;
        let local_token = format!(
            "iris-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let (io, adapter) = runtime
            .block_on(async {
                fips_core::transport::ble::host::HostBleIo::channel("mobile", local_token, 256)
            })
            .map_err(|error| {
                FipsBleBridgeError::Initialization(format!(
                    "Could not create the FIPS BLE bridge: {error}"
                ))
            })?;
        let attachment = crate::core::HostBleAttachment::new(io);
        let ownership = attachment.ownership();
        // Construct the owner before queueing attachment. Any error below drops
        // it, cancelling this exact attachment even if the core replies late.
        let bridge = Arc::new(Self {
            runtime,
            adapter: Arc::new(adapter),
            core_tx,
            closed: AtomicBool::new(false),
            ownership,
        });
        let (reply_tx, reply_rx) = flume::bounded(1);
        bridge
            .core_tx
            .send(CoreMsg::AttachHostBle {
                attachment,
                reply_tx,
            })
            .map_err(|error| {
                FipsBleBridgeError::Initialization(format!(
                    "Could not request the FIPS BLE bridge attachment: {error}"
                ))
            })?;
        reply_rx
            .recv_timeout(timeout)
            .map_err(|error| {
                FipsBleBridgeError::Initialization(format!(
                    "The Iris core did not attach the FIPS BLE bridge: {error}"
                ))
            })?
            .map_err(FipsBleBridgeError::Initialization)?;
        Ok(bridge)
    }

    pub(crate) fn detach_with_timeout(&self, timeout: Duration) -> Result<(), FipsBleBridgeError> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.ownership.cancel();
        let (reply_tx, reply_rx) = flume::bounded(1);
        self.core_tx
            .send(CoreMsg::DetachHostBle {
                ownership: self.ownership.clone(),
                reply_tx,
            })
            .map_err(|error| {
                FipsBleBridgeError::Teardown(format!(
                    "Could not request FIPS BLE teardown: {error}"
                ))
            })?;
        reply_rx.recv_timeout(timeout).map_err(|error| {
            FipsBleBridgeError::Teardown(format!(
                "The Iris core has not acknowledged FIPS BLE teardown: {error}"
            ))
        })?;
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}

impl Drop for FfiFipsBle {
    fn drop(&mut self) {
        if !self.closed.load(Ordering::Acquire) {
            self.ownership.cancel();
            let (reply_tx, _) = flume::bounded(1);
            let _ = self.core_tx.send(CoreMsg::DetachHostBle {
                ownership: self.ownership.clone(),
                reply_tx,
            });
        }
    }
}

impl From<fips_core::transport::ble::host::HostBleCommand> for FipsBleCommand {
    fn from(command: fips_core::transport::ble::host::HostBleCommand) -> Self {
        use fips_core::transport::ble::host::HostBleCommand as Host;
        match command {
            Host::Listen {
                request_id,
                preferred_psm,
            } => Self::Listen {
                request_id,
                preferred_psm,
            },
            Host::StopListening => Self::StopListening,
            Host::StartAdvertising {
                request_id,
                bootstrap,
            } => Self::StartAdvertising {
                request_id,
                bootstrap,
            },
            Host::StopAdvertising { request_id } => Self::StopAdvertising { request_id },
            Host::StartScanning { request_id } => Self::StartScanning { request_id },
            Host::StopScanning => Self::StopScanning,
            Host::Connect {
                request_id,
                peer_token,
                psm,
            } => Self::Connect {
                request_id,
                peer_token,
                psm,
            },
            Host::Write {
                request_id,
                connection_id,
                bytes,
            } => Self::Write {
                request_id,
                connection_id,
                bytes,
            },
            Host::Close { connection_id } => Self::Close { connection_id },
        }
    }
}

impl From<FipsBleEvent> for fips_core::transport::ble::host::HostBleEvent {
    fn from(event: FipsBleEvent) -> Self {
        use fips_core::transport::ble::host::HostBleEvent as Host;
        match event {
            FipsBleEvent::Listening { request_id, psm } => Host::Listening { request_id, psm },
            FipsBleEvent::AdvertisingStarted { request_id } => {
                Host::AdvertisingStarted { request_id }
            }
            FipsBleEvent::AdvertisingStopped { request_id } => {
                Host::AdvertisingStopped { request_id }
            }
            FipsBleEvent::ScanningStarted { request_id } => Host::ScanningStarted { request_id },
            FipsBleEvent::PeerDiscovered {
                peer_token,
                bootstrap,
            } => Host::PeerDiscovered {
                peer_token,
                bootstrap,
            },
            FipsBleEvent::Connected {
                request_id,
                connection_id,
                peer_token,
                send_segment_mtu,
                receive_segment_mtu,
            } => Host::Connected {
                request_id,
                connection_id,
                peer_token,
                send_segment_mtu,
                receive_segment_mtu,
            },
            FipsBleEvent::IncomingConnection {
                connection_id,
                peer_token,
                send_segment_mtu,
                receive_segment_mtu,
            } => Host::IncomingConnection {
                connection_id,
                peer_token,
                send_segment_mtu,
                receive_segment_mtu,
            },
            FipsBleEvent::BytesReceived {
                connection_id,
                bytes,
            } => Host::BytesReceived {
                connection_id,
                bytes,
            },
            FipsBleEvent::WriteCompleted { request_id } => Host::WriteCompleted { request_id },
            FipsBleEvent::Disconnected {
                connection_id,
                reason,
            } => Host::Disconnected {
                connection_id,
                reason,
            },
            FipsBleEvent::Failed {
                request_id,
                message,
            } => Host::Failed {
                request_id,
                message,
            },
        }
    }
}
