//! The desktop update flow: check, then on Install fetch, download, verify,
//! install and restart. Every step and failure is sent to the shell as a
//! finished view, and every network step has a time limit.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use hashtree_updater::{DownloadCallback, DownloadEvent};

use super::install::{self, Applied, InstallError, Relaunch};
use super::store::{CheckOutcome, PendingUpdate, SavedStatus, UpdateStore};
use super::view::{
    desktop_update_view, installs_in_app, DesktopUpdateFailure, DesktopUpdateSnapshot,
    DesktopUpdateStage, DesktopUpdateView,
};
use super::{
    run_app_update, unix_timestamp, version_is_newer, IrisDesktopUpdateResult, UpdateOperation,
    UpdateRun, UpdateSource, SECURE_SOURCE_NAME,
};

const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
/// From pressing Install until the first byte of the download.
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
/// Longest wait for more data once the download has started.
const STALL_TIMEOUT: Duration = Duration::from_secs(45);
/// Unpacking and checking a downloaded app is local work.
const PREPARE_TIMEOUT: Duration = Duration::from_secs(120);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

#[uniffi::export(callback_interface)]
pub trait DesktopUpdateObserver: Send + Sync + 'static {
    /// Called from a background thread; hop to the UI thread to render.
    fn desktop_update_changed(&self, view: DesktopUpdateView);
}

#[derive(Clone)]
pub(crate) struct UpdaterConfig {
    pub(crate) current_version: String,
    pub(crate) state_dir: PathBuf,
    pub(crate) app_path: Option<PathBuf>,
    pub(crate) download_dir: PathBuf,
    pub(crate) manifest_url: Option<String>,
    pub(crate) check_timeout: Duration,
    pub(crate) fetch_timeout: Duration,
    pub(crate) stall_timeout: Duration,
    pub(crate) relaunch: Relaunch,
}

impl UpdaterConfig {
    fn production(state_dir: PathBuf, app_path: Option<PathBuf>) -> Self {
        let download_dir = std::env::var("IRIS_UPDATE_DOWNLOAD_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("IrisChatDownloads"));
        Self {
            current_version: super::current_version().to_string(),
            state_dir,
            app_path,
            download_dir,
            manifest_url: None,
            check_timeout: CHECK_TIMEOUT,
            fetch_timeout: FETCH_TIMEOUT,
            stall_timeout: STALL_TIMEOUT,
            relaunch: Relaunch::default(),
        }
    }
}

struct State {
    stage: DesktopUpdateStage,
    /// The newest verified update that this app can install.
    available: Option<String>,
    last_checked_secs: Option<u64>,
    diagnostic: Option<String>,
    busy: bool,
}

pub(crate) struct Updater {
    config: UpdaterConfig,
    store: UpdateStore,
    state: Mutex<State>,
    observer: Arc<dyn DesktopUpdateObserver>,
}

/// One updater per desktop app. The shell renders each view it receives and
/// calls `perform_action` for the view's button.
#[derive(uniffi::Object)]
pub struct FfiDesktopUpdater {
    inner: Arc<Updater>,
}

#[uniffi::export]
impl FfiDesktopUpdater {
    /// `state_dir` keeps the last check and the update started before a
    /// restart; `app_path` is the running app bundle on macOS.
    #[uniffi::constructor]
    pub fn new(
        observer: Box<dyn DesktopUpdateObserver>,
        state_dir: String,
        app_path: Option<String>,
    ) -> Arc<Self> {
        let app_path = app_path
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from);
        let config = UpdaterConfig::production(PathBuf::from(state_dir), app_path);
        Arc::new(Self {
            inner: Updater::new(config, observer.into()),
        })
    }

    pub fn view(&self) -> DesktopUpdateView {
        self.inner.view()
    }

    /// Look for a newer release. Automatic checks keep the current message
    /// until they finish. `install_when_found` starts Install right away.
    pub fn check(&self, manual: bool, install_when_found: bool) {
        self.inner.check(manual, install_when_found);
    }

    pub fn install(&self) {
        self.inner.install();
    }

    /// What the view's button does: Install, or Retry the step that failed.
    pub fn perform_action(&self) {
        self.inner.perform_action();
    }

    /// Windows: the shell could not start the installer it was given.
    pub fn installer_failed(&self, reason: String) {
        self.inner.installer_failed(reason);
    }
}

enum Waited<T> {
    Done(T),
    TimedOut,
}

/// Run blocking network work on its own thread so a hung request cannot
/// hold the flow past its limit.
fn run_with_limit<T, F>(name: &str, limit: Duration, work: F) -> Waited<anyhow::Result<T>>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let spawned = thread::Builder::new().name(name.into()).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("update step stopped unexpectedly")));
        let _ = sender.send(result);
    });
    if let Err(error) = spawned {
        return Waited::Done(Err(anyhow::anyhow!(error).context("start update step")));
    }
    match receiver.recv_timeout(limit) {
        Ok(result) => Waited::Done(result),
        Err(_) => Waited::TimedOut,
    }
}

enum Flow {
    Download(DownloadEvent),
    Done(anyhow::Result<IrisDesktopUpdateResult>),
}

impl Updater {
    pub(crate) fn new(
        config: UpdaterConfig,
        observer: Arc<dyn DesktopUpdateObserver>,
    ) -> Arc<Self> {
        let store = UpdateStore::new(config.state_dir.clone());
        let state = restored_state(&config, &store);
        Arc::new(Self {
            config,
            store,
            state: Mutex::new(state),
            observer,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn view(&self) -> DesktopUpdateView {
        let state = self.lock();
        desktop_update_view(DesktopUpdateSnapshot {
            stage: state.stage.clone(),
            last_checked_secs: state.last_checked_secs,
            diagnostic: state.diagnostic.clone(),
        })
    }

    fn emit(&self) {
        self.observer.desktop_update_changed(self.view());
    }

    fn show(&self, stage: DesktopUpdateStage) {
        self.lock().stage = stage;
        self.emit();
    }

    /// End a running flow with a final stage and let the buttons work again.
    fn settle(&self, stage: DesktopUpdateStage, diagnostic: Option<String>) {
        {
            let mut state = self.lock();
            state.stage = stage;
            state.busy = false;
            if diagnostic.is_some() {
                state.diagnostic = diagnostic;
            }
        }
        self.emit();
    }

    fn fail(&self, version: Option<String>, failure: DesktopUpdateFailure, cause: String) {
        crate::perflog!("desktop_update.failed failure={failure:?} detail={cause}");
        self.settle(
            DesktopUpdateStage::Failed {
                version,
                failure,
                context: None,
            },
            Some(cause),
        );
    }

    fn update_run(&self, on_event: Option<DownloadCallback>) -> UpdateRun {
        UpdateRun {
            manifest_url: self.config.manifest_url.clone(),
            on_event,
        }
    }

    /// Releases found through a manifest override are trusted by the person
    /// who set it; everything else must come from signed discovery.
    fn unverified(&self, result: &IrisDesktopUpdateResult) -> bool {
        let overridden = self.update_run(None).manifest_override().is_some();
        !overridden && (!result.verified || result.source != SECURE_SOURCE_NAME)
    }

    pub(crate) fn check(self: &Arc<Self>, manual: bool, install_when_found: bool) {
        {
            let mut state = self.lock();
            if state.busy {
                return;
            }
            state.busy = true;
            if manual {
                state.stage = DesktopUpdateStage::Checking;
            }
        }
        if manual {
            self.emit();
        }
        let this = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("iris-update-check".into())
            .spawn(move || this.run_check(install_when_found));
        if let Err(error) = spawned {
            self.fail(None, DesktopUpdateFailure::CheckFailed, error.to_string());
        }
    }

    fn run_check(self: Arc<Self>, install_when_found: bool) {
        let run = self.update_run(None);
        let outcome = run_with_limit(
            "iris-update-check-net",
            self.config.check_timeout,
            move || run_app_update(UpdateOperation::Check, UpdateSource::Auto, &run),
        );
        let known = self.lock().available.clone();
        let checked_at = unix_timestamp();
        let (stage, outcome, diagnostic) = match outcome {
            Waited::TimedOut => (
                failed(known, DesktopUpdateFailure::CheckTimedOut),
                CheckOutcome::CheckFailed,
                Some("update check timed out".to_string()),
            ),
            Waited::Done(Err(error)) => (
                failed(known, DesktopUpdateFailure::CheckFailed),
                CheckOutcome::CheckFailed,
                Some(format!("{error:#}")),
            ),
            Waited::Done(Ok(result)) if !result.ok => (
                failed(known, DesktopUpdateFailure::CheckFailed),
                CheckOutcome::CheckFailed,
                result.error.clone(),
            ),
            Waited::Done(Ok(result)) if self.unverified(&result) => (
                failed(None, DesktopUpdateFailure::NotVerified),
                CheckOutcome::CheckFailed,
                Some(format!(
                    "update came from {} without a signature",
                    result.source
                )),
            ),
            Waited::Done(Ok(result)) => {
                let newer = version_is_newer(&result.tag, &self.config.current_version);
                if !newer {
                    self.lock().available = None;
                    (
                        DesktopUpdateStage::UpToDate { updated_to: None },
                        CheckOutcome::UpToDate,
                        None,
                    )
                } else if result.asset.is_empty() {
                    self.lock().available = None;
                    (
                        failed(Some(result.tag), DesktopUpdateFailure::NoAppForComputer),
                        CheckOutcome::CheckFailed,
                        None,
                    )
                } else {
                    self.lock().available = Some(result.tag.clone());
                    (
                        DesktopUpdateStage::Available {
                            version: result.tag,
                        },
                        CheckOutcome::Available,
                        None,
                    )
                }
            }
        };
        let version = self.lock().available.clone();
        self.store.save_status(&SavedStatus {
            checked_at,
            outcome,
            version,
        });
        let found = matches!(stage, DesktopUpdateStage::Available { .. });
        self.lock().last_checked_secs = Some(checked_at);
        self.settle(stage, diagnostic);
        if found && install_when_found && installs_in_app() {
            self.install();
        }
    }

    pub(crate) fn install(self: &Arc<Self>) {
        let version = {
            let mut state = self.lock();
            let Some(version) = state.available.clone() else {
                return;
            };
            if state.busy || !installs_in_app() {
                return;
            }
            state.busy = true;
            state.stage = DesktopUpdateStage::Fetching {
                version: version.clone(),
            };
            version
        };
        self.emit();
        let this = Arc::clone(self);
        let thread_version = version.clone();
        let spawned = thread::Builder::new()
            .name("iris-update-install".into())
            .spawn(move || this.run_install(thread_version));
        if let Err(error) = spawned {
            self.fail(
                Some(version),
                DesktopUpdateFailure::FetchFailed,
                error.to_string(),
            );
        }
    }

    fn run_install(self: Arc<Self>, version: String) {
        let Some((result, total)) = self.download(&version) else {
            return;
        };
        self.show(DesktopUpdateStage::Verifying {
            version: version.clone(),
        });
        let fail = |failure, cause: String| self.fail(Some(version.clone()), failure, cause);
        if self.unverified(&result) {
            return fail(
                DesktopUpdateFailure::VerifyFailed,
                "download did not come from a signed release".into(),
            );
        }
        let Some(path) = result.path.clone().map(PathBuf::from) else {
            return fail(
                DesktopUpdateFailure::VerifyFailed,
                "download has no file".into(),
            );
        };
        let app_path = self.config.app_path.clone();
        let prepared = run_with_limit("iris-update-verify", PREPARE_TIMEOUT, move || {
            let prepared = install::prepare(&path, total)?;
            install::check_signature(app_path.as_deref(), &prepared)?;
            Ok(prepared)
        });
        let prepared = match prepared {
            Waited::Done(Ok(prepared)) => prepared,
            Waited::Done(Err(error)) => {
                return fail(DesktopUpdateFailure::VerifyFailed, format!("{error:#}"))
            }
            Waited::TimedOut => {
                return fail(
                    DesktopUpdateFailure::VerifyFailed,
                    "checking the download took too long".into(),
                )
            }
        };
        self.show(DesktopUpdateStage::Installing {
            version: version.clone(),
        });
        let applied = install::apply(
            prepared,
            self.config.app_path.as_deref(),
            &self.config.relaunch,
        );
        let installer_path = match applied {
            Ok(Applied::Relaunch) => None,
            Ok(Applied::Installer(path)) => Some(path.display().to_string()),
            Err(InstallError::NoPermission(folder)) => {
                crate::perflog!("desktop_update.failed failure=NoPermission folder={folder}");
                return self.settle(
                    DesktopUpdateStage::Failed {
                        version: Some(version.clone()),
                        failure: DesktopUpdateFailure::NoPermission,
                        context: Some(folder.clone()),
                    },
                    Some(format!("no write permission in {folder}")),
                );
            }
            Err(InstallError::Failed(error)) => {
                return fail(DesktopUpdateFailure::InstallFailed, format!("{error:#}"))
            }
        };
        self.store.mark_pending(&PendingUpdate {
            version: version.clone(),
            from_version: self.config.current_version.clone(),
            started_at: unix_timestamp(),
        });
        // Stay busy: the shell quits on this view.
        self.show(DesktopUpdateStage::Restarting {
            version,
            installer_path,
        });
    }

    /// Fetch and download with progress. Returns None after reporting a
    /// failure or timeout.
    fn download(&self, version: &str) -> Option<(IrisDesktopUpdateResult, Option<u64>)> {
        let (sender, receiver) = mpsc::channel::<Flow>();
        let events = Mutex::new(sender.clone());
        let on_event: DownloadCallback = Arc::new(move |event| {
            if let Ok(events) = events.lock() {
                let _ = events.send(Flow::Download(event));
            }
        });
        let run = self.update_run(Some(on_event));
        let download_dir = self.config.download_dir.clone();
        let spawned = thread::Builder::new()
            .name("iris-update-download".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_app_update(
                        UpdateOperation::Download {
                            download_dir: Some(download_dir),
                        },
                        UpdateSource::Auto,
                        &run,
                    )
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("download stopped unexpectedly")));
                let _ = sender.send(Flow::Done(result));
            });
        let failed = |failure, cause: String| {
            self.fail(Some(version.to_string()), failure, cause);
            None
        };
        if let Err(error) = spawned {
            return failed(DesktopUpdateFailure::FetchFailed, error.to_string());
        }
        let mut started = false;
        let mut total = None;
        let mut downloaded = 0u64;
        let mut last_shown = Instant::now() - PROGRESS_INTERVAL;
        loop {
            let limit = if started {
                self.config.stall_timeout
            } else {
                self.config.fetch_timeout
            };
            let event = match receiver.recv_timeout(limit) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) if started => {
                    return failed(
                        DesktopUpdateFailure::DownloadStalled,
                        format!("no data for {}s after {downloaded} bytes", limit.as_secs()),
                    )
                }
                Err(RecvTimeoutError::Timeout) => {
                    return failed(
                        DesktopUpdateFailure::FetchTimedOut,
                        format!("no download started within {}s", limit.as_secs()),
                    )
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return failed(
                        DesktopUpdateFailure::FetchFailed,
                        "download stopped unexpectedly".into(),
                    )
                }
            };
            match event {
                Flow::Download(DownloadEvent::Started { content_length }) => {
                    started = true;
                    total = content_length;
                    downloaded = 0;
                }
                Flow::Download(DownloadEvent::Progress { downloaded: d, .. }) => {
                    started = true;
                    downloaded = d;
                    if last_shown.elapsed() < PROGRESS_INTERVAL {
                        continue;
                    }
                }
                Flow::Download(DownloadEvent::Finished { total: done }) => {
                    downloaded = done;
                }
                Flow::Done(Ok(result)) if result.ok => return Some((result, total)),
                Flow::Done(Ok(result)) => {
                    let cause = result.error.unwrap_or_default();
                    return failed(download_failure(started), cause);
                }
                Flow::Done(Err(error)) => {
                    return failed(download_failure(started), format!("{error:#}"));
                }
            }
            last_shown = Instant::now();
            self.show(DesktopUpdateStage::Downloading {
                version: version.to_string(),
                downloaded_bytes: downloaded,
                total_bytes: total,
            });
        }
    }

    pub(crate) fn perform_action(self: &Arc<Self>) {
        let (installable, retry_check) = {
            let state = self.lock();
            let retry_check = matches!(
                state.stage,
                DesktopUpdateStage::Failed { failure, .. } if !failure.during_install()
            );
            (state.available.is_some(), retry_check)
        };
        if installable {
            self.install();
        } else if retry_check {
            self.check(true, false);
        }
    }

    pub(crate) fn installer_failed(&self, reason: String) {
        let version = match &self.lock().stage {
            DesktopUpdateStage::Restarting { version, .. } => version.clone(),
            _ => return,
        };
        self.store.clear_pending();
        self.settle(
            DesktopUpdateStage::Failed {
                version: Some(version),
                failure: DesktopUpdateFailure::InstallerDidNotStart,
                context: Some(reason.clone()),
            },
            Some(reason),
        );
    }
}

fn failed(version: Option<String>, failure: DesktopUpdateFailure) -> DesktopUpdateStage {
    DesktopUpdateStage::Failed {
        version,
        failure,
        context: None,
    }
}

fn download_failure(started: bool) -> DesktopUpdateFailure {
    if started {
        DesktopUpdateFailure::DownloadFailed
    } else {
        DesktopUpdateFailure::FetchFailed
    }
}

/// What to show when the app starts: the result of an update that was
/// started before the restart, else the last check.
fn restored_state(config: &UpdaterConfig, store: &UpdateStore) -> State {
    let mut state = State {
        stage: DesktopUpdateStage::Idle,
        available: None,
        last_checked_secs: None,
        diagnostic: None,
        busy: false,
    };
    if let Some(saved) = store.load_status() {
        state.last_checked_secs = Some(saved.checked_at);
        let still_newer = saved
            .version
            .as_deref()
            .filter(|version| version_is_newer(version, &config.current_version));
        state.stage = match (saved.outcome, still_newer) {
            (CheckOutcome::Available, Some(version)) if installs_in_app() => {
                state.available = Some(version.to_string());
                DesktopUpdateStage::Available {
                    version: version.to_string(),
                }
            }
            (CheckOutcome::CheckFailed, _) => failed(None, DesktopUpdateFailure::CheckFailed),
            _ => DesktopUpdateStage::UpToDate { updated_to: None },
        };
    }
    if let Some(pending) = store.take_pending(unix_timestamp()) {
        if version_is_newer(&pending.version, &config.current_version) {
            state.available = Some(pending.version.clone());
            state.diagnostic = Some(format!(
                "still on {} after installing {}",
                config.current_version, pending.version
            ));
            state.stage = DesktopUpdateStage::Failed {
                version: Some(pending.version),
                failure: DesktopUpdateFailure::DidNotFinish,
                context: Some(format!("v{}", config.current_version)),
            };
        } else {
            state.stage = DesktopUpdateStage::UpToDate {
                updated_to: Some(pending.version),
            };
        }
    }
    state
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
