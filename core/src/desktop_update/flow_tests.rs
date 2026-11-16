// Install steps run only where the app can replace itself (macOS here).
#![cfg_attr(
    not(all(target_os = "macos", target_arch = "aarch64")),
    allow(dead_code)
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::super::view::DesktopUpdateStep;
use super::*;

const TAG: &str = "v2099.1.1";

#[derive(Default)]
struct Recorder {
    views: Mutex<Vec<DesktopUpdateView>>,
    changed: Condvar,
}

impl DesktopUpdateObserver for Recorder {
    fn desktop_update_changed(&self, view: DesktopUpdateView) {
        self.views.lock().unwrap().push(view);
        self.changed.notify_all();
    }
}

impl Recorder {
    /// Wait for a view after the first `skip` ones.
    fn wait_after(
        &self,
        skip: usize,
        matches: impl Fn(&DesktopUpdateView) -> bool,
    ) -> DesktopUpdateView {
        let views = self.views.lock().unwrap();
        let (views, timeout) = self
            .changed
            .wait_timeout_while(views, Duration::from_secs(20), |views| {
                !views.iter().skip(skip).any(&matches)
            })
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "no matching update view; saw {:#?}",
            views.iter().map(|view| &view.status).collect::<Vec<_>>()
        );
        views
            .iter()
            .skip(skip)
            .find(|view| matches(view))
            .unwrap()
            .clone()
    }

    fn wait_for_step(&self, skip: usize, step: DesktopUpdateStep) -> DesktopUpdateView {
        self.wait_after(skip, |view| view.step == step)
    }

    fn count(&self) -> usize {
        self.views.lock().unwrap().len()
    }

    fn steps_since(&self, skip: usize) -> Vec<DesktopUpdateStep> {
        let mut steps: Vec<DesktopUpdateStep> = Vec::new();
        for view in self.views.lock().unwrap().iter().skip(skip) {
            if steps.last() != Some(&view.step) {
                steps.push(view.step);
            }
        }
        steps
    }

    fn views_since(&self, skip: usize) -> Vec<DesktopUpdateView> {
        self.views
            .lock()
            .unwrap()
            .iter()
            .skip(skip)
            .cloned()
            .collect()
    }
}

/// The release name this computer's updater picks, if it has one.
fn platform_asset_name() -> Option<String> {
    [
        format!("iris-chat-{TAG}-macos-arm64.app.tar.gz"),
        format!("iris-chat-{TAG}-linux-x64.tar.gz"),
        format!("iris-chat-{TAG}-linux-arm64.tar.gz"),
        format!("iris-chat-{TAG}-windows-x64-setup.exe"),
    ]
    .into_iter()
    .find(|name| super::super::app_asset_name_matches_current_target(name))
}

struct Fixture {
    _root: tempfile::TempDir,
    root: PathBuf,
    release: PathBuf,
    asset: PathBuf,
}

impl Fixture {
    fn new() -> Option<Self> {
        let name = platform_asset_name()?;
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path().canonicalize().unwrap();
        let release = root.join("release");
        fs::create_dir_all(&release).unwrap();
        let manifest = serde_json::json!({
            "tag": TAG,
            "assets": [{ "name": name, "path": name }],
        });
        fs::write(release.join("release.json"), manifest.to_string()).unwrap();
        Some(Self {
            asset: release.join(&name),
            _root: root_dir,
            root,
            release,
        })
    }

    fn config(&self, current_version: &str, wait_pid: u32) -> UpdaterConfig {
        UpdaterConfig {
            current_version: current_version.into(),
            state_dir: self.root.join("state"),
            app_path: Some(self.app()),
            download_dir: self.root.join("downloads"),
            manifest_url: Some(format!(
                "file://{}",
                self.release.join("release.json").display()
            )),
            check_timeout: Duration::from_secs(10),
            fetch_timeout: Duration::from_millis(1500),
            stall_timeout: Duration::from_millis(1000),
            relaunch: Relaunch {
                wait_pid,
                program: self.relauncher(),
            },
        }
    }

    fn app(&self) -> PathBuf {
        self.root.join("Applications").join("Iris Chat.app")
    }

    fn relaunched(&self) -> PathBuf {
        self.root.join("relaunched.txt")
    }

    /// Stands in for `open`: records which app it was asked to start.
    fn relauncher(&self) -> PathBuf {
        let path = self.root.join("relaunch.sh");
        if !path.exists() {
            fs::write(
                &path,
                format!(
                    "#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n",
                    self.relaunched().display()
                ),
            )
            .unwrap();
            make_executable(&path);
        }
        path
    }

    fn install_running_app(&self, version: &str) {
        write_bundle(&self.app(), version);
    }

    fn app_version(&self) -> String {
        fs::read_to_string(self.app().join("Contents/version")).unwrap()
    }

    /// Publish a working update archive at the asset path.
    fn publish_update(&self) {
        let build = self.root.join("build");
        let _ = fs::remove_dir_all(&build);
        write_bundle(&build.join("Iris Chat.app"), "new");
        // Big enough to arrive in several chunks.
        let noise: Vec<u8> = (0..1_200_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        fs::write(build.join("Iris Chat.app/Contents/payload"), noise).unwrap();
        let _ = fs::remove_file(&self.asset);
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&self.asset)
            .arg("-C")
            .arg(&build)
            .arg("Iris Chat.app")
            .status()
            .unwrap();
        assert!(status.success());
    }
}

fn write_bundle(app: &Path, version: &str) {
    fs::create_dir_all(app.join("Contents")).unwrap();
    fs::write(app.join("Contents/version"), version).unwrap();
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

fn new_updater(config: UpdaterConfig) -> (Arc<Updater>, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    (Updater::new(config, recorder.clone()), recorder)
}

#[test]
fn each_step_has_plain_words() {
    let view = |stage| {
        desktop_update_view(DesktopUpdateSnapshot {
            stage,
            last_checked_secs: None,
            diagnostic: None,
        })
    };
    let downloading = view(DesktopUpdateStage::Downloading {
        version: TAG.into(),
        downloaded_bytes: 12_400_000,
        total_bytes: Some(36_200_000),
    });
    assert_eq!(downloading.status, "Downloading update v2099.1.1…");
    assert_eq!(downloading.detail.as_deref(), Some("12.4 MB of 36.2 MB"));
    assert!((downloading.progress.unwrap() - 0.3425).abs() < 0.001);
    assert!(downloading.busy && downloading.show_banner && downloading.action_label.is_none());

    let stalled = view(DesktopUpdateStage::Failed {
        version: Some(TAG.into()),
        failure: DesktopUpdateFailure::DownloadStalled,
        context: None,
    });
    assert_eq!(stalled.status, "The download stopped");
    assert!(stalled.show_banner && !stalled.busy);

    let check_failed = view(DesktopUpdateStage::Failed {
        version: None,
        failure: DesktopUpdateFailure::CheckFailed,
        context: None,
    });
    assert_eq!(check_failed.status, "Couldn’t check for updates");
    assert_eq!(check_failed.action_label.as_deref(), Some("Retry"));
    assert!(!check_failed.show_banner);

    for stage in [
        DesktopUpdateStage::Fetching {
            version: TAG.into(),
        },
        DesktopUpdateStage::Verifying {
            version: TAG.into(),
        },
        DesktopUpdateStage::Installing {
            version: TAG.into(),
        },
        DesktopUpdateStage::Restarting {
            version: TAG.into(),
            installer_path: None,
        },
    ] {
        let view = view(stage);
        assert!(view.busy && !view.status.is_empty(), "{view:?}");
    }
}

#[test]
fn failed_check_says_so_and_retry_checks_again() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let manifest = fixture.release.join("release.json");
    let saved = fs::read(&manifest).unwrap();
    fs::remove_file(&manifest).unwrap();
    let (updater, views) = new_updater(fixture.config("2026.11.1", std::process::id()));

    updater.check(true, false);
    let failed = views.wait_for_step(0, DesktopUpdateStep::Failed);
    assert_eq!(failed.status, "Couldn’t check for updates");
    assert_eq!(
        failed.detail.as_deref(),
        Some("Check your connection and try again.")
    );
    assert_eq!(failed.action_label.as_deref(), Some("Retry"));
    assert!(failed.diagnostic.is_some());
    assert!(failed.last_checked_secs.is_some());

    fs::write(&manifest, saved).unwrap();
    let seen = views.count();
    updater.perform_action();
    let found = views.wait_for_step(seen, DesktopUpdateStep::Available);
    assert_eq!(views.steps_since(seen)[0], DesktopUpdateStep::Checking);
    assert_eq!(found.status, "Update v2099.1.1 available");

    // The next launch remembers the last check.
    let (_, restarted) = updater_with_views(fixture.config("2026.11.1", std::process::id()));
    assert_eq!(restarted.last_checked_secs, found.last_checked_secs);
}

fn updater_with_views(config: UpdaterConfig) -> (Arc<Updater>, DesktopUpdateView) {
    let (updater, _) = new_updater(config);
    let view = updater.view();
    (updater, view)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod mac_install {
    use super::*;

    struct RunningApp(std::process::Child);

    impl RunningApp {
        fn start() -> Self {
            Self(Command::new("sleep").arg("60").spawn().unwrap())
        }

        fn quit(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    impl Drop for RunningApp {
        fn drop(&mut self) {
            self.quit();
        }
    }

    fn found(fixture: &Fixture, current: &str, pid: u32) -> (Arc<Updater>, Arc<Recorder>) {
        let (updater, views) = new_updater(fixture.config(current, pid));
        updater.check(true, false);
        let available = views.wait_for_step(0, DesktopUpdateStep::Available);
        assert_eq!(available.action_label.as_deref(), Some("Install"));
        assert!(available.show_banner);
        (updater, views)
    }

    fn wait_until(what: &str, done: impl Fn() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn install_shows_every_step_and_reopens_only_after_the_app_quits() {
        let fixture = Fixture::new().unwrap();
        fixture.install_running_app("old");
        fixture.publish_update();
        let mut app = RunningApp::start();
        let (updater, views) = found(&fixture, "2026.11.1", app.0.id());

        let seen = views.count();
        updater.perform_action();
        let restarting = views.wait_for_step(seen, DesktopUpdateStep::Restarting);
        assert_eq!(
            views.steps_since(seen),
            [
                DesktopUpdateStep::Fetching,
                DesktopUpdateStep::Downloading,
                DesktopUpdateStep::Verifying,
                DesktopUpdateStep::Installing,
                DesktopUpdateStep::Restarting,
            ]
        );
        let progress: Vec<f64> = views
            .views_since(seen)
            .iter()
            .filter(|view| view.step == DesktopUpdateStep::Downloading)
            .map(|view| view.progress.expect("download size is known"))
            .collect();
        assert_eq!(progress.first().copied(), Some(0.0));
        assert_eq!(progress.last().copied(), Some(1.0));
        assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(views
            .views_since(seen)
            .iter()
            .all(|view| view.busy && view.show_banner && view.action_label.is_none()));
        assert_eq!(restarting.status, "Restarting Iris Chat…");
        assert!(restarting.quit_to_finish);
        assert_eq!(updater.view().step, DesktopUpdateStep::Restarting);

        // The new app is already in place, but it is reopened only after the
        // running copy has exited.
        assert_eq!(fixture.app_version(), "new");
        std::thread::sleep(Duration::from_millis(400));
        assert!(!fixture.relaunched().exists(), "reopened while running");
        app.quit();
        wait_until("the relaunch", || fixture.relaunched().exists());
        assert_eq!(
            fs::read_to_string(fixture.relaunched()).unwrap(),
            fixture.app().display().to_string()
        );
        // Only the new app is left in its folder.
        wait_until("the old copy to go", || {
            fs::read_dir(fixture.app().parent().unwrap())
                .unwrap()
                .count()
                == 1
        });

        // After the restart the new version says it was updated.
        let (_, after) = updater_with_views(fixture.config("2099.1.1", 0));
        assert_eq!(after.step, DesktopUpdateStep::UpToDate);
        assert_eq!(after.status, "Updated to v2099.1.1");
        assert!(!after.show_banner);
    }

    #[test]
    fn update_that_did_not_take_effect_is_reported_with_retry() {
        let fixture = Fixture::new().unwrap();
        fixture.install_running_app("old");
        fixture.publish_update();
        let mut app = RunningApp::start();
        let (updater, views) = found(&fixture, "2026.11.1", app.0.id());
        updater.perform_action();
        views.wait_for_step(0, DesktopUpdateStep::Restarting);
        app.quit();
        wait_until("the relaunch", || fixture.relaunched().exists());

        // The app comes back still on the old version.
        let mut app = RunningApp::start();
        let (updater, views) = new_updater(fixture.config("2026.11.1", app.0.id()));
        let view = updater.view();
        assert_eq!(view.step, DesktopUpdateStep::Failed);
        assert_eq!(view.status, "The update didn’t finish");
        assert_eq!(
            view.detail.as_deref(),
            Some("Iris Chat is still on v2026.11.1. Try again.")
        );
        assert_eq!(view.action_label.as_deref(), Some("Retry"));
        assert!(view.show_banner);
        updater.perform_action();
        views.wait_for_step(0, DesktopUpdateStep::Restarting);
        app.quit();
    }

    #[test]
    fn damaged_download_fails_plainly_and_keeps_the_app() {
        let fixture = Fixture::new().unwrap();
        fixture.install_running_app("old");
        fs::write(&fixture.asset, b"not an archive").unwrap();
        let app = RunningApp::start();
        let (updater, views) = found(&fixture, "2026.11.1", app.0.id());

        updater.perform_action();
        let failed = views.wait_for_step(0, DesktopUpdateStep::Failed);
        assert_eq!(failed.status, "The download was damaged");
        assert_eq!(failed.action_label.as_deref(), Some("Retry"));
        assert!(failed.show_banner && !failed.busy);
        assert_eq!(fixture.app_version(), "old");

        fixture.publish_update();
        let seen = views.count();
        updater.perform_action();
        views.wait_for_step(seen, DesktopUpdateStep::Restarting);
        assert_eq!(fixture.app_version(), "new");
    }

    #[test]
    fn app_folder_without_permission_is_named_and_left_unchanged() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new().unwrap();
        fixture.install_running_app("old");
        fixture.publish_update();
        let folder = fixture.app().parent().unwrap().to_path_buf();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).unwrap();
        let app = RunningApp::start();
        let (updater, views) = found(&fixture, "2026.11.1", app.0.id());

        updater.perform_action();
        let failed = views.wait_for_step(0, DesktopUpdateStep::Failed);
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(failed.status, "Couldn’t replace Iris Chat");
        assert_eq!(
            failed.detail.as_deref(),
            Some("No permission to change apps in Applications.")
        );
        assert_eq!(failed.action_label.as_deref(), Some("Retry"));
        assert_eq!(fixture.app_version(), "old");
        assert!(!fixture.relaunched().exists());
    }

    #[test]
    fn stalled_download_times_out_and_retry_finishes() {
        let fixture = Fixture::new().unwrap();
        fixture.install_running_app("old");
        let status = Command::new("mkfifo").arg(&fixture.asset).status().unwrap();
        assert!(status.success());
        // A server that sends a little and then goes quiet.
        let asset = fixture.asset.clone();
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            let mut pipe = fs::OpenOptions::new().write(true).open(asset).unwrap();
            pipe.write_all(&vec![7u8; 300_000]).unwrap();
            std::thread::sleep(Duration::from_secs(3));
        });
        let app = RunningApp::start();
        let (updater, views) = found(&fixture, "2026.11.1", app.0.id());

        updater.perform_action();
        let failed = views.wait_for_step(0, DesktopUpdateStep::Failed);
        assert!(views
            .steps_since(0)
            .contains(&DesktopUpdateStep::Downloading));
        assert_eq!(failed.status, "The download stopped");
        assert_eq!(
            failed.detail.as_deref(),
            Some("No data arrived for a while. Check your connection and try again.")
        );
        assert_eq!(failed.action_label.as_deref(), Some("Retry"));
        writer.join().unwrap();
        // Let the abandoned read reach the end of the pipe.
        std::thread::sleep(Duration::from_millis(300));

        fs::remove_file(&fixture.asset).unwrap();
        fixture.publish_update();
        let seen = views.count();
        updater.perform_action();
        views.wait_for_step(seen, DesktopUpdateStep::Restarting);
        assert_eq!(fixture.app_version(), "new");
    }

    #[test]
    fn release_that_never_answers_times_out() {
        let fixture = Fixture::new().unwrap();
        fixture.install_running_app("old");
        let status = Command::new("mkfifo").arg(&fixture.asset).status().unwrap();
        assert!(status.success());
        let app = RunningApp::start();
        let (updater, views) = found(&fixture, "2026.11.1", app.0.id());

        updater.perform_action();
        let failed = views.wait_for_step(0, DesktopUpdateStep::Failed);
        assert_eq!(failed.status, "Couldn’t get the update");
        assert_eq!(
            failed.detail.as_deref(),
            Some("The update servers didn’t answer. Check your connection and try again.")
        );
        assert_eq!(failed.action_label.as_deref(), Some("Retry"));
        // Let the abandoned read finish.
        drop(fs::OpenOptions::new().write(true).open(&fixture.asset));
    }
}
