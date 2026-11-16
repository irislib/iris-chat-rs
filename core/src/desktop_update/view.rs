//! What the desktop shells show for an update. Every word lives here so the
//! macOS and Windows shells only render the view they are given.

/// Coarse step for shells that pick an icon or a layout per step.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopUpdateStep {
    Idle,
    Checking,
    UpToDate,
    Available,
    Fetching,
    Downloading,
    Verifying,
    Installing,
    Restarting,
    Failed,
}

/// Why an update stopped. Each one has its own plain message.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopUpdateFailure {
    CheckFailed,
    CheckTimedOut,
    NotVerified,
    NoAppForComputer,
    FetchFailed,
    FetchTimedOut,
    DownloadFailed,
    DownloadStalled,
    VerifyFailed,
    InstallFailed,
    NoPermission,
    InstallerDidNotStart,
    DidNotFinish,
}

impl DesktopUpdateFailure {
    /// Failures that happened while installing; Retry installs again.
    pub(crate) fn during_install(self) -> bool {
        !matches!(
            self,
            Self::CheckFailed | Self::CheckTimedOut | Self::NotVerified | Self::NoAppForComputer
        )
    }
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum DesktopUpdateStage {
    Idle,
    Checking,
    UpToDate {
        updated_to: Option<String>,
    },
    Available {
        version: String,
    },
    Fetching {
        version: String,
    },
    Downloading {
        version: String,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    Verifying {
        version: String,
    },
    Installing {
        version: String,
    },
    Restarting {
        version: String,
        installer_path: Option<String>,
    },
    Failed {
        /// The update that was found or being installed, if any.
        version: Option<String>,
        failure: DesktopUpdateFailure,
        /// Plain detail such as the installed version or the folder name.
        context: Option<String>,
    },
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct DesktopUpdateSnapshot {
    pub stage: DesktopUpdateStage,
    pub last_checked_secs: Option<u64>,
    /// Raw cause for support logs. Never shown as the message.
    pub diagnostic: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct DesktopUpdateView {
    pub step: DesktopUpdateStep,
    /// The update's version tag, empty when none is known.
    pub version: String,
    /// One short line: what is happening now, or what went wrong.
    pub status: String,
    /// Optional second line: progress in bytes, or what to do next.
    pub detail: Option<String>,
    /// 0.0 to 1.0 while the size is known; None for an indeterminate step.
    pub progress: Option<f64>,
    /// A step is running: show a spinner or bar and disable the buttons.
    pub busy: bool,
    /// Label for the one button, such as "Install" or "Retry".
    pub action_label: Option<String>,
    /// Show the update strip above the chats.
    pub show_banner: bool,
    pub can_check: bool,
    /// The shell must quit now so the update can finish.
    pub quit_to_finish: bool,
    /// Windows: start this installer before quitting.
    pub installer_path: Option<String>,
    pub last_checked_secs: Option<u64>,
    pub diagnostic: Option<String>,
}

const APP_NAME: &str = "Iris Chat";

pub(crate) fn installs_in_app() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Turn a state into words. Shells call this only for previews and tests;
/// a running updater sends finished views to its observer.
#[uniffi::export]
pub fn desktop_update_view(snapshot: DesktopUpdateSnapshot) -> DesktopUpdateView {
    let DesktopUpdateSnapshot {
        stage,
        last_checked_secs,
        diagnostic,
    } = snapshot;
    let mut view = DesktopUpdateView {
        step: DesktopUpdateStep::Idle,
        version: String::new(),
        status: String::new(),
        detail: None,
        progress: None,
        busy: false,
        action_label: None,
        show_banner: false,
        can_check: true,
        quit_to_finish: false,
        installer_path: None,
        last_checked_secs,
        diagnostic,
    };
    match stage {
        DesktopUpdateStage::Idle => {}
        DesktopUpdateStage::Checking => {
            view.step = DesktopUpdateStep::Checking;
            view.status = "Checking for updates…".into();
            view.busy = true;
        }
        DesktopUpdateStage::UpToDate { updated_to } => {
            view.step = DesktopUpdateStep::UpToDate;
            view.status = match updated_to {
                Some(version) => format!("Updated to {version}"),
                None => "Up to date".into(),
            };
        }
        DesktopUpdateStage::Available { version } => {
            view.step = DesktopUpdateStep::Available;
            view.status = format!("Update {version} available");
            if installs_in_app() {
                view.action_label = Some("Install".into());
            } else {
                view.detail = Some("Get it from iris.to or your package manager.".into());
            }
            view.show_banner = true;
            view.version = version;
        }
        DesktopUpdateStage::Fetching { version } => {
            view.step = DesktopUpdateStep::Fetching;
            view.status = format!("Finding update {version}…");
            busy_banner(&mut view, version);
        }
        DesktopUpdateStage::Downloading {
            version,
            downloaded_bytes,
            total_bytes,
        } => {
            view.step = DesktopUpdateStep::Downloading;
            view.status = format!("Downloading update {version}…");
            view.detail = Some(match total_bytes.filter(|total| *total > 0) {
                Some(total) => {
                    let done = downloaded_bytes.min(total);
                    view.progress = Some(done as f64 / total as f64);
                    format!("{} of {}", megabytes(done), megabytes(total))
                }
                None => megabytes(downloaded_bytes),
            });
            busy_banner(&mut view, version);
        }
        DesktopUpdateStage::Verifying { version } => {
            view.step = DesktopUpdateStep::Verifying;
            view.status = "Checking the download…".into();
            view.progress = Some(1.0);
            busy_banner(&mut view, version);
        }
        DesktopUpdateStage::Installing { version } => {
            view.step = DesktopUpdateStep::Installing;
            view.status = format!("Installing update {version}…");
            busy_banner(&mut view, version);
        }
        DesktopUpdateStage::Restarting {
            version,
            installer_path,
        } => {
            view.step = DesktopUpdateStep::Restarting;
            if installer_path.is_some() {
                view.status = "Opening the installer…".into();
                view.detail = Some(format!("{APP_NAME} closes while the update installs."));
            } else {
                view.status = format!("Restarting {APP_NAME}…");
            }
            view.installer_path = installer_path;
            view.quit_to_finish = true;
            busy_banner(&mut view, version);
        }
        DesktopUpdateStage::Failed {
            version,
            failure,
            context,
        } => {
            view.step = DesktopUpdateStep::Failed;
            let (status, detail) = failure_words(failure, version.as_deref(), context.as_deref());
            view.status = status;
            view.detail = detail;
            let installable = installs_in_app() && version.is_some();
            view.action_label = match failure {
                DesktopUpdateFailure::NoAppForComputer => None,
                // A newer check failed but the verified update is still here.
                _ if !failure.during_install() && installable => Some("Install".into()),
                _ if failure.during_install() && !installable => None,
                _ => Some("Retry".into()),
            };
            view.show_banner = failure.during_install() || installable;
            view.version = version.unwrap_or_default();
        }
    }
    view
}

fn busy_banner(view: &mut DesktopUpdateView, version: String) {
    view.busy = true;
    view.can_check = false;
    view.show_banner = true;
    view.version = version;
}

fn failure_words(
    failure: DesktopUpdateFailure,
    version: Option<&str>,
    context: Option<&str>,
) -> (String, Option<String>) {
    let connection = "Check your connection and try again.";
    let (status, detail): (String, Option<String>) = match failure {
        DesktopUpdateFailure::CheckFailed => {
            ("Couldn’t check for updates".into(), Some(connection.into()))
        }
        DesktopUpdateFailure::CheckTimedOut => (
            "Couldn’t check for updates".into(),
            Some(format!("The update servers didn’t answer. {connection}")),
        ),
        DesktopUpdateFailure::NotVerified => (
            "Update could not be verified".into(),
            Some("Nothing was downloaded.".into()),
        ),
        DesktopUpdateFailure::NoAppForComputer => (
            match version {
                Some(version) => format!("Update {version} has no app for this computer"),
                None => "This update has no app for this computer".into(),
            },
            None,
        ),
        DesktopUpdateFailure::FetchFailed => {
            ("Couldn’t get the update".into(), Some(connection.into()))
        }
        DesktopUpdateFailure::FetchTimedOut => (
            "Couldn’t get the update".into(),
            Some(format!("The update servers didn’t answer. {connection}")),
        ),
        DesktopUpdateFailure::DownloadFailed => (
            "Couldn’t download the update".into(),
            Some(connection.into()),
        ),
        DesktopUpdateFailure::DownloadStalled => (
            "The download stopped".into(),
            Some(format!("No data arrived for a while. {connection}")),
        ),
        DesktopUpdateFailure::VerifyFailed => (
            "The download was damaged".into(),
            Some("Nothing was installed. Try again.".into()),
        ),
        DesktopUpdateFailure::InstallFailed => (
            "Couldn’t install the update".into(),
            Some(format!("{APP_NAME} was not changed. Try again.")),
        ),
        DesktopUpdateFailure::NoPermission => (
            format!("Couldn’t replace {APP_NAME}"),
            Some(match context {
                Some(folder) => format!("No permission to change apps in {folder}."),
                None => "No permission to change the app's folder.".into(),
            }),
        ),
        DesktopUpdateFailure::InstallerDidNotStart => (
            "The installer didn’t start".into(),
            Some(context.map_or_else(|| "Try again.".into(), str::to_owned)),
        ),
        DesktopUpdateFailure::DidNotFinish => (
            "The update didn’t finish".into(),
            Some(match context {
                Some(current) => format!("{APP_NAME} is still on {current}. Try again."),
                None => "Try again.".into(),
            }),
        ),
    };
    (status, detail)
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}
