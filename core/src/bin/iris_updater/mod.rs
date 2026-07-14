use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use hashtree_updater::{
    DownloadOptions, UpdateAsset, UpdateCheckOptions, UpdateManifest, UpdateTarget,
};
use iris_chat_core::update_announcements::build_secure_update_updater;
use serde::{Deserialize, Serialize};

mod asset_selection;
mod io;

use asset_selection::*;
use io::*;

const HTREE_MANIFEST_URL: &str = "https://upload.iris.to/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/releases%2Firis-chat-rs/latest/release.json";
const UPDATE_CONNECT_TIMEOUT_SECS: &str = "4";
const UPDATE_MANIFEST_TIMEOUT_SECS: &str = "8";
const UPDATE_DOWNLOAD_TIMEOUT_SECS: &str = "180";
const UPDATE_USER_AGENT: &str = "iris-chat-updater";
const SECURE_SOURCE_NAME: &str = "hashtree-nostr-blossom";
const MANIFEST_SOURCE_NAME: &str = "hashtree-release-json";

#[derive(Subcommand)]
pub(crate) enum UpdateCommands {
    /// Print the latest published version and selected asset.
    Check(UpdateCheckArgs),
    /// Download the selected asset.
    Download(UpdateDownloadArgs),
    /// Replace the running iris binary with the latest CLI binary.
    Install(UpdateInstallArgs),
}

#[derive(Args)]
pub(crate) struct UpdateCheckArgs {
    /// Select the native desktop app artifact instead of the iris CLI archive.
    #[arg(long)]
    app: bool,
    /// Emit machine-readable JSON for app update helpers.
    #[arg(long)]
    json: bool,
    /// Release source to query.
    #[arg(long, value_enum, default_value = "auto")]
    source: UpdateSource,
}

#[derive(Args)]
pub(crate) struct UpdateDownloadArgs {
    /// Download to an exact path.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Select the native desktop app artifact instead of the iris CLI archive.
    #[arg(long)]
    app: bool,
    /// Directory for downloaded app helper artifacts.
    #[arg(long)]
    download_dir: Option<PathBuf>,
    /// Emit machine-readable JSON for app update helpers.
    #[arg(long)]
    json: bool,
    /// Release source to query.
    #[arg(long, value_enum, default_value = "auto")]
    source: UpdateSource,
}

#[derive(Args)]
pub(crate) struct UpdateInstallArgs {
    /// Override the install destination (defaults to current_exe()).
    #[arg(long)]
    to: Option<PathBuf>,
    /// Prefer an asset whose filename contains this value.
    #[arg(long)]
    kind: Option<String>,
    /// Skip if the published version is not newer than current.
    #[arg(long)]
    only_if_newer: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Release source to query.
    #[arg(long, value_enum, default_value = "auto")]
    source: UpdateSource,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum UpdateSource {
    Auto,
    #[value(alias = "htree")]
    Hashtree,
    Manifest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateMode {
    Cli,
    App,
}

impl UpdateMode {
    fn noun(self) -> &'static str {
        match self {
            Self::Cli => "iris CLI",
            Self::App => "Iris Chat app",
        }
    }
}

#[derive(Debug)]
enum UpdateOperation {
    Check,
    Download {
        out: Option<PathBuf>,
        download_dir: Option<PathBuf>,
    },
    Install {
        to: Option<PathBuf>,
        kind: Option<String>,
        only_if_newer: bool,
    },
}

#[derive(Debug)]
struct UpdateRequest {
    mode: UpdateMode,
    operation: UpdateOperation,
    source: UpdateSource,
    json: bool,
}

impl UpdateRequest {
    fn from_command(command: UpdateCommands, global_json: bool) -> Self {
        match command {
            UpdateCommands::Check(args) => Self {
                mode: if args.app {
                    UpdateMode::App
                } else {
                    UpdateMode::Cli
                },
                operation: UpdateOperation::Check,
                source: args.source,
                json: args.json || global_json,
            },
            UpdateCommands::Download(args) => Self {
                mode: if args.app {
                    UpdateMode::App
                } else {
                    UpdateMode::Cli
                },
                operation: UpdateOperation::Download {
                    out: args.out,
                    download_dir: args.download_dir,
                },
                source: args.source,
                json: args.json || global_json,
            },
            UpdateCommands::Install(args) => Self {
                mode: UpdateMode::Cli,
                operation: UpdateOperation::Install {
                    to: args.to,
                    kind: args.kind,
                    only_if_newer: args.only_if_newer,
                },
                source: args.source,
                json: args.json || global_json,
            },
        }
    }

    fn preferred_kind(&self) -> Option<&str> {
        match &self.operation {
            UpdateOperation::Install { kind, .. } => kind.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseManifest {
    #[serde(alias = "tag_name")]
    tag: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    #[serde(alias = "browser_download_url")]
    path: String,
}

struct SelectedManifestAsset {
    manifest: ReleaseManifest,
    asset: ReleaseAsset,
    asset_url: String,
    update_available: bool,
}

#[derive(Debug, Serialize)]
struct UpdateJson<'a> {
    available: bool,
    current_version: &'a str,
    latest_version: String,
    tag: String,
    asset: String,
    source: &'a str,
    verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

struct DownloadedUpdate<'a> {
    available: bool,
    tag: &'a str,
    asset: &'a str,
    source: &'static str,
    verified: bool,
    url: Option<&'a str>,
    path: &'a Path,
}

pub(crate) fn run_iris_update(command: UpdateCommands, global_json: bool) -> Result<()> {
    let request = UpdateRequest::from_command(command, global_json);
    if should_use_secure_hashtree(request.source) {
        run_secure_update(&request)
    } else {
        run_manifest_update(&request)
    }
}

fn should_use_secure_hashtree(source: UpdateSource) -> bool {
    let manifest_override = std::env::var("IRIS_UPDATE_MANIFEST_URL").ok();
    update_source_uses_secure_hashtree(source, manifest_override.as_deref())
}

fn update_source_uses_secure_hashtree(
    source: UpdateSource,
    manifest_override: Option<&str>,
) -> bool {
    match source {
        UpdateSource::Hashtree => true,
        UpdateSource::Manifest => false,
        UpdateSource::Auto => manifest_override
            .filter(|value| !value.trim().is_empty())
            .is_none(),
    }
}

fn run_secure_update(request: &UpdateRequest) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start update runtime")?;
    runtime.block_on(run_secure_update_async(request))
}

async fn run_secure_update_async(request: &UpdateRequest) -> Result<()> {
    let (reference, updater) = build_secure_update_updater()
        .await
        .context("failed to prepare signed release updater")?;
    let mut check = updater
        .check(UpdateCheckOptions {
            reference,
            current_version: current_version().to_string(),
            target: UpdateTarget::new(current_target()),
            ..UpdateCheckOptions::default()
        })
        .await
        .context("failed to resolve signed hashtree release")?;
    let asset = preferred_secure_asset(&check.manifest, request).ok_or_else(|| {
        anyhow!(
            "release {} has no {} asset for {}",
            display_manifest_tag(&check.manifest),
            request.mode.noun(),
            current_target()
        )
    })?;
    check.asset = Some(asset.clone());
    let tag = display_manifest_tag(&check.manifest);
    let available = version_is_newer(&tag, current_version());

    match &request.operation {
        UpdateOperation::Check => print_update_check(
            request.mode,
            available,
            &tag,
            &asset.name,
            SECURE_SOURCE_NAME,
            true,
            None,
            request.json,
            None,
        ),
        UpdateOperation::Download { out, download_dir } => {
            let temp_dir = create_temp_dir("iris-update")?;
            let destination = selected_download_path(
                out.as_deref(),
                download_dir.as_deref(),
                &asset.name,
                &temp_dir,
            )?;
            let downloaded = updater
                .download(&check, DownloadOptions::default(), None)
                .await
                .with_context(|| {
                    format!("failed to download verified hashtree asset {}", asset.name)
                })?;
            write_downloaded_asset(&destination, &downloaded.bytes)?;
            print_downloaded(
                DownloadedUpdate {
                    available,
                    tag: &tag,
                    asset: &asset.name,
                    source: SECURE_SOURCE_NAME,
                    verified: true,
                    url: None,
                    path: &destination,
                },
                request.json,
            )
        }
        UpdateOperation::Install {
            to, only_if_newer, ..
        } => {
            if *only_if_newer && !available {
                return print_up_to_date(
                    request.mode,
                    &tag,
                    SECURE_SOURCE_NAME,
                    true,
                    request.json,
                );
            }
            let temp_dir = create_temp_dir("iris-update")?;
            let destination =
                selected_download_path(None, Some(&temp_dir), &asset.name, &temp_dir)?;
            let downloaded = updater
                .download(&check, DownloadOptions::default(), None)
                .await
                .with_context(|| {
                    format!("failed to download verified hashtree asset {}", asset.name)
                })?;
            write_downloaded_asset(&destination, &downloaded.bytes)?;
            let installed_path = install_destination(to.as_deref())?;
            install_cli_archive(&destination, &temp_dir, Some(&installed_path))?;
            let _ = fs::remove_dir_all(&temp_dir);
            if request.json {
                print_downloaded(
                    DownloadedUpdate {
                        available,
                        tag: &tag,
                        asset: &asset.name,
                        source: SECURE_SOURCE_NAME,
                        verified: true,
                        url: None,
                        path: &installed_path,
                    },
                    true,
                )
            } else {
                println!(
                    "updated iris at {} from {} to {tag}",
                    installed_path.display(),
                    current_version()
                );
                Ok(())
            }
        }
    }
}

fn run_manifest_update(request: &UpdateRequest) -> Result<()> {
    let selection = manifest_selection(request)?;
    match &request.operation {
        UpdateOperation::Check => print_update_check(
            request.mode,
            selection.update_available,
            &selection.manifest.tag,
            &selection.asset.name,
            MANIFEST_SOURCE_NAME,
            false,
            Some(&selection.asset_url),
            request.json,
            None,
        ),
        UpdateOperation::Download { out, download_dir } => {
            let temp_dir = create_temp_dir("iris-update")?;
            let destination = selected_download_path(
                out.as_deref(),
                download_dir.as_deref(),
                &selection.asset.name,
                &temp_dir,
            )?;
            download_asset(&selection.asset_url, &destination)?;
            print_downloaded(
                DownloadedUpdate {
                    available: selection.update_available,
                    tag: &selection.manifest.tag,
                    asset: &selection.asset.name,
                    source: MANIFEST_SOURCE_NAME,
                    verified: false,
                    url: Some(&selection.asset_url),
                    path: &destination,
                },
                request.json,
            )
        }
        UpdateOperation::Install {
            to, only_if_newer, ..
        } => {
            if *only_if_newer && !selection.update_available {
                return print_up_to_date(
                    request.mode,
                    &selection.manifest.tag,
                    MANIFEST_SOURCE_NAME,
                    false,
                    request.json,
                );
            }
            let temp_dir = create_temp_dir("iris-update")?;
            let destination =
                selected_download_path(None, Some(&temp_dir), &selection.asset.name, &temp_dir)?;
            download_asset(&selection.asset_url, &destination)?;
            let installed_path = install_destination(to.as_deref())?;
            install_cli_archive(&destination, &temp_dir, Some(&installed_path))?;
            let _ = fs::remove_dir_all(&temp_dir);
            if request.json {
                print_downloaded(
                    DownloadedUpdate {
                        available: selection.update_available,
                        tag: &selection.manifest.tag,
                        asset: &selection.asset.name,
                        source: MANIFEST_SOURCE_NAME,
                        verified: false,
                        url: Some(&selection.asset_url),
                        path: &installed_path,
                    },
                    true,
                )
            } else {
                println!(
                    "updated iris at {} from {} to {}",
                    installed_path.display(),
                    current_version(),
                    selection.manifest.tag
                );
                Ok(())
            }
        }
    }
}

fn manifest_selection(request: &UpdateRequest) -> Result<SelectedManifestAsset> {
    let manifest_url = manifest_url();
    let manifest = fetch_manifest(&manifest_url)?;
    let asset = preferred_manifest_asset(&manifest, request).ok_or_else(|| {
        anyhow!(
            "release {} has no {} asset for {}",
            manifest.tag,
            request.mode.noun(),
            current_target()
        )
    })?;
    let asset_url = manifest_asset_url(&manifest_url, &asset.path);
    let update_available = version_is_newer(&manifest.tag, current_version());
    Ok(SelectedManifestAsset {
        manifest,
        asset,
        asset_url,
        update_available,
    })
}

fn manifest_url() -> String {
    std::env::var("IRIS_UPDATE_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| HTREE_MANIFEST_URL.to_string())
}

fn fetch_manifest(url: &str) -> Result<ReleaseManifest> {
    let output = curl_command(UPDATE_MANIFEST_TIMEOUT_SECS)
        .arg(url)
        .output()
        .with_context(|| format!("failed to run curl for {url}"))?;
    if !output.status.success() {
        return Err(anyhow!("{}", command_error("update check failed", &output)));
    }
    serde_json::from_slice(&output.stdout).context("failed to parse release manifest")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_versions_skip_dev_placeholders() {
        assert!(version_is_newer("v2026.5.18.6", "2026.5.18.5"));
        assert!(!version_is_newer("v2026.5.18.6", "0.1.30"));
    }

    #[test]
    fn relative_asset_urls_use_manifest_directory() {
        assert_eq!(
            manifest_asset_url(
                "https://example.invalid/releases/iris-chat-rs/latest/release.json",
                "assets/iris.tgz",
            ),
            "https://example.invalid/releases/iris-chat-rs/latest/assets/iris.tgz"
        );
    }

    #[test]
    fn app_assets_do_not_match_cli_archives() {
        assert!(!app_asset_name_matches_current_target(&format!(
            "iris-{}.tar.gz",
            current_target()
        )));
    }

    #[test]
    fn explicit_update_sources_override_manifest_environment() {
        assert!(!update_source_uses_secure_hashtree(
            UpdateSource::Auto,
            Some("file:///tmp/release.json"),
        ));
        assert!(update_source_uses_secure_hashtree(
            UpdateSource::Hashtree,
            Some("file:///tmp/release.json"),
        ));
        assert!(!update_source_uses_secure_hashtree(
            UpdateSource::Manifest,
            None,
        ));
        assert!(update_source_uses_secure_hashtree(
            UpdateSource::Auto,
            Some("  "),
        ));
    }
}
