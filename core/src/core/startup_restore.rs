//! Cancellation is limited to restoring a durable protocol checkpoint. Normal
//! message/ratchet transactions still use the acknowledged suspension barrier.
use super::SharedConnection;
use rusqlite::InterruptHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct StartupRestoreControl {
    state: Mutex<ControlState>,
}

impl std::fmt::Debug for StartupRestoreControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        formatter
            .debug_struct("StartupRestoreControl")
            .field("epoch", &state.epoch)
            .field("paused", &state.paused)
            .field("active", &state.active.is_some())
            .finish()
    }
}

#[derive(Default)]
struct ControlState {
    epoch: u64,
    paused: bool,
    active: Option<ActiveRestore>,
}

struct ActiveRestore {
    cancelled: Arc<AtomicBool>,
    interrupt: InterruptHandle,
}

#[derive(Debug)]
pub(super) struct StartupRestoreDeferred;

impl std::fmt::Display for StartupRestoreDeferred {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("startup restore deferred until foreground")
    }
}

impl std::error::Error for StartupRestoreDeferred {}

impl StartupRestoreControl {
    #[cfg(test)]
    pub(in crate::core) fn is_active_for_test(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .is_some()
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).epoch
    }

    /// Called directly by the host, never through the core queue or DB mutex.
    pub(crate) fn cancel(&self, epoch: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.epoch != epoch {
            return;
        }
        state.paused = true;
        if let Some(active) = &state.active {
            active.cancelled.store(true, Ordering::Release);
            active.interrupt.interrupt();
        }
    }

    pub(crate) fn foregrounded(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.epoch = state.epoch.wrapping_add(1);
        state.paused = false;
        // Cancellation of an already-running restore is latched until its
        // transaction unwinds. A foreground race must not revive partial work.
    }

    pub(super) fn begin(
        self: &Arc<Self>,
        connection: SharedConnection,
    ) -> anyhow::Result<StartupRestoreGuard> {
        let conn = connection
            .lock()
            .map_err(|_| anyhow::anyhow!("startup storage connection mutex poisoned"))?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        anyhow::ensure!(
            conn.is_autocommit(),
            "startup restore requires an idle connection"
        );
        if state.paused {
            return Err(StartupRestoreDeferred.into());
        }
        anyhow::ensure!(state.active.is_none(), "startup restore already active");
        let cancelled = Arc::new(AtomicBool::new(false));
        state.active = Some(ActiveRestore {
            cancelled: Arc::clone(&cancelled),
            interrupt: conn.get_interrupt_handle(),
        });
        let check = Arc::clone(&cancelled);
        // sqlite3_interrupt alone is a no-op between statements. The scoped
        // hook also prevents a subsequent short statement from restarting the
        // cancelled transaction. It is removed before ordinary runtime work.
        conn.progress_handler(1, Some(move || check.load(Ordering::Acquire)));
        drop(state);
        drop(conn);
        Ok(StartupRestoreGuard {
            control: Arc::clone(self),
            connection,
            cancelled,
            registered: true,
            cleaned: false,
        })
    }
}

pub(super) struct StartupRestoreGuard {
    control: Arc<StartupRestoreControl>,
    connection: SharedConnection,
    cancelled: Arc<AtomicBool>,
    registered: bool,
    cleaned: bool,
}

impl StartupRestoreGuard {
    pub(super) fn finish<T>(mut self, result: anyhow::Result<T>) -> anyhow::Result<T> {
        let rolled_back = self.cleanup()?;
        anyhow::ensure!(
            !rolled_back || result.is_err(),
            "startup restore left an unfinished transaction"
        );
        let interrupted = result.as_ref().err().is_some_and(|error| {
            error.chain().any(|cause| {
                cause
                    .downcast_ref::<iris_chat_protocol::StorageError>()
                    .is_some_and(iris_chat_protocol::StorageError::is_interrupted)
                    || matches!(
                        cause.downcast_ref::<rusqlite::Error>(),
                        Some(rusqlite::Error::SqliteFailure(inner, _))
                            if inner.code == rusqlite::ErrorCode::OperationInterrupted
                    )
            })
        });
        if interrupted && self.cancelled.load(Ordering::Acquire) {
            Err(StartupRestoreDeferred.into())
        } else {
            // A completed commit stays successful if cancellation arrived after
            // its final SQL statement. Its durable checkpoint is authoritative.
            result
        }
    }

    fn cleanup(&mut self) -> anyhow::Result<bool> {
        if self.cleaned {
            return Ok(false);
        }
        let conn = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        if self.registered {
            let mut state = self.control.state.lock().unwrap_or_else(|e| e.into_inner());
            if state
                .active
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(&active.cancelled, &self.cancelled))
            {
                conn.progress_handler(0, None::<fn() -> bool>);
                state.active = None;
            }
            self.registered = false;
        }
        // An interrupted DML statement normally rolls its transaction back.
        // A cancelled read/savepoint cleanup can leave it open instead. We
        // own no pre-existing transaction and must drain it with the hook
        // removed before acknowledgement. Do not hold the control lock across
        // disk I/O: an expiration request must remain nonblocking during undo.
        let rolled_back = !conn.is_autocommit();
        if rolled_back {
            conn.execute_batch("ROLLBACK")?;
        }
        self.cleaned = true;
        Ok(rolled_back)
    }
}

impl Drop for StartupRestoreGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
