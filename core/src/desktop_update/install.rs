//! Platform steps after the download: check the file, put the new app in
//! place while this one still runs, and hand over to a relaunch that waits
//! for this process to exit.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

/// A downloaded update that passed its checks and is ready to install.
#[derive(Debug)]
pub(super) enum Prepared {
    /// macOS: the unpacked app bundle, inside a folder we own.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    AppBundle { bundle: PathBuf, unpacked: PathBuf },
    /// Windows: the setup program the shell starts before quitting.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Installer(PathBuf),
}

#[derive(Debug)]
pub(super) enum Applied {
    /// The new app is in place; a helper reopens it once this process exits.
    Relaunch,
    /// The shell starts this installer and quits.
    Installer(PathBuf),
}

#[derive(Debug)]
pub(super) enum InstallError {
    /// The folder holding the app cannot be changed by this user.
    NoPermission(String),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for InstallError {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error)
    }
}

/// Where and how the macOS relaunch helper runs. Tests replace the program.
#[derive(Clone, Debug)]
pub(crate) struct Relaunch {
    /// Process the helper waits for before reopening the app.
    pub(crate) wait_pid: u32,
    /// Program run with the app path once the process is gone.
    pub(crate) program: PathBuf,
}

impl Default for Relaunch {
    fn default() -> Self {
        Self {
            wait_pid: std::process::id(),
            program: PathBuf::from("/usr/bin/open"),
        }
    }
}

/// Check the downloaded file and unpack it where needed.
pub(super) fn prepare(download: &Path, expected_len: Option<u64>) -> Result<Prepared> {
    let length = fs::metadata(download)
        .with_context(|| format!("downloaded update missing at {}", download.display()))?
        .len();
    if length == 0 {
        return Err(anyhow!("downloaded update is empty"));
    }
    if let Some(expected) = expected_len.filter(|expected| *expected > 0) {
        if expected != length {
            return Err(anyhow!(
                "downloaded update has {length} bytes, expected {expected}"
            ));
        }
    }
    prepare_platform(download)
}

#[cfg(target_os = "macos")]
fn prepare_platform(download: &Path) -> Result<Prepared> {
    let name = download
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !name.ends_with(".app.tar.gz") {
        return Err(anyhow!("{name} is not a Mac app archive"));
    }
    let unpacked = super::create_temp_dir("IrisChatUpdate")?;
    let unpack = run_tool(
        std::process::Command::new("/usr/bin/tar")
            .arg("-xzf")
            .arg(download)
            .arg("-C")
            .arg(&unpacked),
        "unpack the update",
    )
    .and_then(|()| {
        find_app_bundle(&unpacked).ok_or_else(|| anyhow!("the update has no Iris Chat app"))
    });
    match unpack {
        Ok(bundle) => Ok(Prepared::AppBundle { bundle, unpacked }),
        Err(error) => {
            let _ = fs::remove_dir_all(&unpacked);
            Err(error)
        }
    }
}

#[cfg(target_os = "windows")]
fn prepare_platform(download: &Path) -> Result<Prepared> {
    let is_installer = download
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    if !is_installer {
        return Err(anyhow!("{} is not a Windows installer", download.display()));
    }
    Ok(Prepared::Installer(download.to_path_buf()))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn prepare_platform(download: &Path) -> Result<Prepared> {
    Err(anyhow!(
        "{} cannot be installed from inside the app here",
        download.display()
    ))
}

/// macOS: the running app's signer must also have signed the update.
#[cfg(target_os = "macos")]
pub(super) fn check_signature(running_app: Option<&Path>, prepared: &Prepared) -> Result<()> {
    let (Some(running_app), Prepared::AppBundle { bundle, .. }) = (running_app, prepared) else {
        return Ok(());
    };
    // Development builds are not signed by a team; nothing to compare.
    let Some(team) = signing_team(running_app) else {
        return Ok(());
    };
    run_tool(
        std::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(bundle),
        "verify the update's signature",
    )?;
    match signing_team(bundle) {
        Some(update_team) if update_team == team => Ok(()),
        other => Err(anyhow!(
            "update is signed by {}, expected {team}",
            other.as_deref().unwrap_or("nobody")
        )),
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn check_signature(_running_app: Option<&Path>, _prepared: &Prepared) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn signing_team(bundle: &Path) -> Option<String> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=2"])
        .arg(bundle)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|team| !team.is_empty() && *team != "not set")
        .map(str::to_owned)
}

/// Install the prepared update. On macOS the new bundle replaces the old one
/// now, so a failure is reported while the app is still open.
pub(super) fn apply(
    prepared: Prepared,
    app_path: Option<&Path>,
    relaunch: &Relaunch,
) -> Result<Applied, InstallError> {
    match prepared {
        Prepared::Installer(path) => Ok(Applied::Installer(path)),
        Prepared::AppBundle { bundle, unpacked } => {
            let result = app_path
                .ok_or_else(|| InstallError::Failed(anyhow!("the app's location is unknown")))
                .and_then(|app| replace_bundle(&bundle, app, relaunch));
            let _ = fs::remove_dir_all(&unpacked);
            result.map(|()| Applied::Relaunch)
        }
    }
}

fn replace_bundle(bundle: &Path, app: &Path, relaunch: &Relaunch) -> Result<(), InstallError> {
    let parent = app
        .parent()
        .ok_or_else(|| InstallError::Failed(anyhow!("{} has no folder", app.display())))?;
    let name = app
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| InstallError::Failed(anyhow!("{} has no name", app.display())))?;
    let pid = std::process::id();
    let staged = parent.join(format!(".{name}.update-{pid}"));
    let previous = parent.join(format!(".{name}.previous-{pid}"));
    let no_permission = || InstallError::NoPermission(folder_name(parent));
    let _ = fs::remove_dir_all(&staged);
    let _ = fs::remove_dir_all(&previous);
    // Copy inside the app's own folder so both swaps below are renames.
    fs::create_dir(&staged).map_err(|error| match error.kind() {
        std::io::ErrorKind::PermissionDenied => no_permission(),
        _ => InstallError::Failed(anyhow!(error).context("prepare the app folder")),
    })?;
    let _ = fs::remove_dir(&staged);
    if let Err(error) = copy_bundle(bundle, &staged) {
        let _ = fs::remove_dir_all(&staged);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(app, &previous) {
        let _ = fs::remove_dir_all(&staged);
        return Err(match error.kind() {
            std::io::ErrorKind::PermissionDenied => no_permission(),
            _ => InstallError::Failed(anyhow!(error).context("move the current app aside")),
        });
    }
    let restore = |cause: anyhow::Error| {
        let _ = fs::remove_dir_all(app);
        let _ = fs::rename(&previous, app);
        let _ = fs::remove_dir_all(&staged);
        InstallError::Failed(cause)
    };
    if let Err(error) = fs::rename(&staged, app) {
        return Err(restore(anyhow!(error).context("put the new app in place")));
    }
    spawn_relaunch(relaunch, app, &previous).map_err(restore)
}

#[cfg(unix)]
fn copy_bundle(from: &Path, to: &Path) -> Result<()> {
    let tool = if cfg!(target_os = "macos") {
        "/usr/bin/ditto"
    } else {
        "cp"
    };
    let mut command = std::process::Command::new(tool);
    if !cfg!(target_os = "macos") {
        command.arg("-R");
    }
    run_tool(command.arg(from).arg(to), "copy the new app")
}

#[cfg(not(unix))]
fn copy_bundle(_from: &Path, _to: &Path) -> Result<()> {
    Err(anyhow!("app bundles are only installed on macOS"))
}

/// Wait for the running app to exit before reopening it. Reopening earlier
/// reaches the old process or loses the single-instance lock to it, and then
/// no app comes back at all.
#[cfg(unix)]
fn spawn_relaunch(relaunch: &Relaunch, app: &Path, previous: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    const SCRIPT: &str = r#"pid="$1"; app="$2"; previous="$3"; program="$4"
tries=0
while kill -0 "$pid" 2>/dev/null && [ "$tries" -lt 1200 ]; do
  sleep 0.1
  tries=$((tries + 1))
done
rm -rf "$previous"
exec "$program" "$app""#;
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(SCRIPT)
        .arg("iris-chat-relaunch")
        .arg(relaunch.wait_pid.to_string())
        .arg(app)
        .arg(previous)
        .arg(&relaunch.program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Survive the app quitting and any signal sent to its group.
        .process_group(0)
        .spawn()
        .map(drop)
        .context("start the relaunch helper")
}

#[cfg(not(unix))]
fn spawn_relaunch(_relaunch: &Relaunch, _app: &Path, _previous: &Path) -> Result<()> {
    Err(anyhow!("relaunching is only supported on macOS"))
}

#[cfg(unix)]
fn run_tool(command: &mut std::process::Command, what: &str) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("couldn't {what}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "{}",
        super::command_error(&format!("couldn't {what}"), &output)
    ))
}

#[cfg(target_os = "macos")]
fn find_app_bundle(directory: &Path) -> Option<PathBuf> {
    let mut pending = vec![(directory.to_path_buf(), 0usize)];
    while let Some((folder, depth)) = pending.pop() {
        for entry in fs::read_dir(&folder).ok()?.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "Iris Chat.app" || name == "IrisChatMac.app" {
                return Some(path);
            }
            if depth < 2 && path.is_dir() && !name.ends_with(".app") {
                pending.push((path, depth + 1));
            }
        }
    }
    None
}

fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}
