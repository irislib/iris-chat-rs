use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use hashtree_blossom::{BlossomClient, BlossomStore};
use hashtree_core::{HashTree, HashTreeConfig};
use hashtree_resolver::nostr::{NostrResolverConfig, NostrRootResolver};
use hashtree_updater::{
    DownloadOptions, HashtreeUpdater, UpdateAsset, UpdateCheckOptions, UpdateManifest, UpdateRef,
    UpdateTarget,
};
use serde::Deserialize;

const HTREE_MANIFEST_URL: &str = "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-chat-rs/latest/release.json";
const HTREE_UPDATE_REF: &str =
    "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-chat-rs/latest";
const UPDATE_CONNECT_TIMEOUT_SECS: &str = "4";
const UPDATE_MANIFEST_TIMEOUT_SECS: &str = "8";
const UPDATE_DOWNLOAD_TIMEOUT_SECS: &str = "180";
const UPDATE_USER_AGENT: &str = "iris-chat-updater";
const SECURE_SOURCE_NAME: &str = "hashtree-nostr-blossom";
const MANIFEST_SOURCE_NAME: &str = "hashtree-release-json";
const DEFAULT_UPDATE_RELAYS: &[&str] = &[
    "wss://temp.iris.to",
    "wss://relay.damus.io",
    "wss://relay.snort.social",
    "wss://relay.primal.net",
    "wss://upload.iris.to/nostr",
];
const DEFAULT_BLOSSOM_READ_SERVERS: &[&str] = &[
    "https://cdn.iris.to",
    "https://upload.iris.to",
    "https://blossom.primal.net",
];

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct IrisDesktopUpdateResult {
    pub ok: bool,
    pub error: Option<String>,
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub tag: String,
    pub asset: String,
    pub source: String,
    pub verified: bool,
    pub url: Option<String>,
    pub path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum UpdateSource {
    Auto,
    Hashtree,
    Manifest,
}

#[derive(Debug)]
enum UpdateOperation {
    Check,
    Download { download_dir: Option<PathBuf> },
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

impl IrisDesktopUpdateResult {
    fn error(message: String) -> Self {
        Self {
            ok: false,
            error: Some(message),
            available: false,
            current_version: current_version().to_string(),
            latest_version: String::new(),
            tag: String::new(),
            asset: String::new(),
            source: String::new(),
            verified: false,
            url: None,
            path: None,
        }
    }

    fn from_error(error: anyhow::Error) -> Self {
        Self::error(error.to_string())
    }
}

#[uniffi::export]
pub fn iris_desktop_update_check() -> IrisDesktopUpdateResult {
    crate::ffi_or(
        "iris_desktop_update_check",
        IrisDesktopUpdateResult::error("Update check failed".to_string()),
        || match run_app_update(UpdateOperation::Check, UpdateSource::Auto) {
            Ok(result) => result,
            Err(error) => IrisDesktopUpdateResult::from_error(error),
        },
    )
}

#[uniffi::export]
pub fn iris_desktop_update_download(download_dir: Option<String>) -> IrisDesktopUpdateResult {
    crate::ffi_or(
        "iris_desktop_update_download",
        IrisDesktopUpdateResult::error("Update download failed".to_string()),
        || {
            let download_dir = download_dir
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from);
            match run_app_update(
                UpdateOperation::Download { download_dir },
                UpdateSource::Auto,
            ) {
                Ok(result) => result,
                Err(error) => IrisDesktopUpdateResult::from_error(error),
            }
        },
    )
}

fn run_app_update(
    operation: UpdateOperation,
    source: UpdateSource,
) -> Result<IrisDesktopUpdateResult> {
    if should_use_secure_hashtree(source) {
        run_secure_update(operation)
    } else {
        run_manifest_update(operation)
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

fn run_secure_update(operation: UpdateOperation) -> Result<IrisDesktopUpdateResult> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start update runtime")?;
    runtime.block_on(run_secure_update_async(operation))
}

async fn run_secure_update_async(operation: UpdateOperation) -> Result<IrisDesktopUpdateResult> {
    let resolver = NostrRootResolver::new(NostrResolverConfig {
        relays: update_relays(),
        resolve_timeout: Duration::from_secs(
            UPDATE_MANIFEST_TIMEOUT_SECS.parse::<u64>().unwrap_or(8),
        ),
        secret_key: None,
    })
    .await
    .context("failed to connect to release message servers")?;
    let blossom = BlossomClient::new_empty(nostr35::Keys::generate())
        .with_read_servers(blossom_read_servers())
        .with_timeout(Duration::from_secs(
            UPDATE_DOWNLOAD_TIMEOUT_SECS.parse::<u64>().unwrap_or(180),
        ));
    let store = Arc::new(BlossomStore::new(blossom));
    let tree = HashTree::new(HashTreeConfig::new(store).public());
    let updater = HashtreeUpdater::new(resolver, tree);
    let mut check = updater
        .check(UpdateCheckOptions {
            reference: secure_update_ref()?,
            current_version: current_version().to_string(),
            target: UpdateTarget::new(current_target()),
            ..UpdateCheckOptions::default()
        })
        .await
        .context("failed to resolve signed release")?;
    let asset = preferred_secure_app_asset(&check.manifest).ok_or_else(|| {
        anyhow!(
            "release {} has no app update for {}",
            display_manifest_tag(&check.manifest),
            current_target()
        )
    })?;
    check.asset = Some(asset.clone());
    let tag = display_manifest_tag(&check.manifest);
    let available = version_is_newer(&tag, current_version());

    match operation {
        UpdateOperation::Check => Ok(update_result(
            available,
            &tag,
            &asset.name,
            SECURE_SOURCE_NAME,
            true,
            None,
            None,
        )),
        UpdateOperation::Download { download_dir } => {
            let temp_dir = create_temp_dir("iris-update")?;
            let destination =
                selected_download_path(download_dir.as_deref(), &asset.name, &temp_dir)?;
            let downloaded = updater
                .download(&check, DownloadOptions::default(), None)
                .await
                .with_context(|| format!("failed to download verified update {}", asset.name))?;
            write_downloaded_asset(&destination, &downloaded.bytes)?;
            Ok(update_result(
                available,
                &tag,
                &asset.name,
                SECURE_SOURCE_NAME,
                true,
                None,
                Some(&destination),
            ))
        }
    }
}

fn run_manifest_update(operation: UpdateOperation) -> Result<IrisDesktopUpdateResult> {
    let selection = manifest_selection()?;
    match operation {
        UpdateOperation::Check => Ok(update_result(
            selection.update_available,
            &selection.manifest.tag,
            &selection.asset.name,
            MANIFEST_SOURCE_NAME,
            false,
            Some(&selection.asset_url),
            None,
        )),
        UpdateOperation::Download { download_dir } => {
            let temp_dir = create_temp_dir("iris-update")?;
            let destination =
                selected_download_path(download_dir.as_deref(), &selection.asset.name, &temp_dir)?;
            download_asset(&selection.asset_url, &destination)?;
            Ok(update_result(
                selection.update_available,
                &selection.manifest.tag,
                &selection.asset.name,
                MANIFEST_SOURCE_NAME,
                false,
                Some(&selection.asset_url),
                Some(&destination),
            ))
        }
    }
}

fn update_result(
    available: bool,
    tag: &str,
    asset: &str,
    source: &'static str,
    verified: bool,
    url: Option<&str>,
    path: Option<&Path>,
) -> IrisDesktopUpdateResult {
    IrisDesktopUpdateResult {
        ok: true,
        error: None,
        available,
        current_version: current_version().to_string(),
        latest_version: tag.trim_start_matches(['v', 'V']).to_string(),
        tag: tag.to_string(),
        asset: asset.to_string(),
        source: source.to_string(),
        verified,
        url: url.map(ToOwned::to_owned),
        path: path.map(|value| value.display().to_string()),
    }
}

fn manifest_selection() -> Result<SelectedManifestAsset> {
    let manifest_url = manifest_url();
    let manifest = fetch_manifest(&manifest_url)?;
    let asset = preferred_app_asset(&manifest.assets).ok_or_else(|| {
        anyhow!(
            "release {} has no app update for {}",
            manifest.tag,
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

fn secure_update_ref() -> Result<UpdateRef> {
    let raw = std::env::var("IRIS_UPDATE_HTREE_REF")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| HTREE_UPDATE_REF.to_string());
    UpdateRef::parse(&raw).with_context(|| format!("invalid update hashtree ref: {raw}"))
}

fn update_relays() -> Vec<String> {
    split_env_csv("IRIS_UPDATE_RELAYS").unwrap_or_else(|| {
        DEFAULT_UPDATE_RELAYS
            .iter()
            .map(|value| (*value).to_string())
            .collect()
    })
}

fn blossom_read_servers() -> Vec<String> {
    split_env_csv("IRIS_UPDATE_BLOSSOM_SERVERS").unwrap_or_else(|| {
        DEFAULT_BLOSSOM_READ_SERVERS
            .iter()
            .map(|value| (*value).to_string())
            .collect()
    })
}

fn split_env_csv(name: &str) -> Option<Vec<String>> {
    let values = std::env::var(name)
        .ok()?
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn manifest_url() -> String {
    std::env::var("IRIS_UPDATE_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| HTREE_MANIFEST_URL.to_string())
}

fn fetch_manifest(url: &str) -> Result<ReleaseManifest> {
    let bytes = read_url(url, UPDATE_MANIFEST_TIMEOUT_SECS)
        .with_context(|| format!("failed to fetch release manifest from {url}"))?;
    serde_json::from_slice(&bytes).context("failed to parse release manifest")
}

fn preferred_app_asset(assets: &[ReleaseAsset]) -> Option<ReleaseAsset> {
    assets
        .iter()
        .find(|asset| app_asset_name_matches_current_target(&asset.name))
        .cloned()
}

fn preferred_secure_app_asset(manifest: &UpdateManifest) -> Option<UpdateAsset> {
    manifest
        .assets
        .iter()
        .find(|asset| app_asset_name_matches_current_target(&asset.name))
        .cloned()
}

fn app_asset_name_matches_current_target(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        lower.ends_with("-macos-arm64.app.tar.gz")
            || lower.ends_with("-macos-arm64.dmg")
            || lower.ends_with("-macos-arm64.zip")
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        lower.ends_with("-linux-x64.deb") || lower.ends_with("-linux-x64.tar.gz")
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        lower.ends_with("-linux-arm64.deb") || lower.ends_with("-linux-arm64.tar.gz")
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        lower.ends_with("-windows-x64-setup.exe")
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        let _ = lower;
        false
    }
}

fn display_manifest_tag(manifest: &UpdateManifest) -> String {
    manifest
        .tag
        .clone()
        .filter(|tag| !tag.trim().is_empty())
        .unwrap_or_else(|| format!("v{}", manifest.effective_version()))
}

fn current_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        "unsupported"
    }
}

fn manifest_asset_url(manifest_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("file://") {
        return path.to_string();
    }
    if Path::new(path).is_absolute() {
        return format!("file://{path}");
    }
    let base = manifest_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(manifest_url);
    format!("{}/{}", base, path.trim_start_matches('/'))
}

fn download_asset(url: &str, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if let Some(path) = local_file_url_path(url)? {
        fs::copy(&path, destination).with_context(|| {
            format!(
                "failed to copy update from {} to {}",
                path.display(),
                destination.display()
            )
        })?;
        return Ok(());
    }
    let output = curl_command(UPDATE_DOWNLOAD_TIMEOUT_SECS)
        .arg("-o")
        .arg(destination)
        .arg(url)
        .output()
        .with_context(|| format!("failed to run curl for {url}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{}",
            command_error("update download failed", &output)
        ));
    }
    Ok(())
}

fn read_url(url: &str, max_time: &str) -> Result<Vec<u8>> {
    if let Some(path) = local_file_url_path(url)? {
        return fs::read(&path).with_context(|| format!("failed to read {}", path.display()));
    }
    let output = curl_command(max_time)
        .arg(url)
        .output()
        .with_context(|| format!("failed to run curl for {url}"))?;
    if !output.status.success() {
        return Err(anyhow!("{}", command_error("update check failed", &output)));
    }
    Ok(output.stdout)
}

fn local_file_url_path(value: &str) -> Result<Option<PathBuf>> {
    if value.starts_with("file://") {
        let url = url::Url::parse(value).with_context(|| format!("invalid file URL: {value}"))?;
        return url
            .to_file_path()
            .map(Some)
            .map_err(|_| anyhow!("invalid file URL path: {value}"));
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Ok(Some(path.to_path_buf()));
    }
    Ok(None)
}

fn curl_command(max_time: &str) -> Command {
    let mut command = Command::new("curl");
    command.args([
        "-fsSL",
        "--connect-timeout",
        UPDATE_CONNECT_TIMEOUT_SECS,
        "--max-time",
        max_time,
        "-H",
    ]);
    command.arg(format!("User-Agent: {UPDATE_USER_AGENT}"));
    command
}

fn selected_download_path(
    download_dir: Option<&Path>,
    asset_name: &str,
    temp_dir: &Path,
) -> Result<PathBuf> {
    let file_name = safe_file_name(asset_name);
    let parent = download_dir
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf))
        })
        .unwrap_or_else(|| temp_dir.to_path_buf());
    fs::create_dir_all(&parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    Ok(parent.join(file_name))
}

fn write_downloaded_asset(destination: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(destination, bytes).with_context(|| {
        format!(
            "failed to write verified update to {}",
            destination.display()
        )
    })
}

fn current_version() -> &'static str {
    crate::core::app_version_string()
}

fn create_temp_dir(prefix: &str) -> Result<PathBuf> {
    let base = std::env::temp_dir();
    for attempt in 0..128u32 {
        let path = base.join(format!(
            "{prefix}-{}-{}-{attempt}",
            std::process::id(),
            unix_timestamp()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to create {}", path.display()));
            }
        }
    }
    Err(anyhow!("failed to allocate temporary update directory"))
}

fn safe_file_name(name: &str) -> String {
    let value = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "iris-update".to_string()
    } else {
        value
    }
}

fn command_error(prefix: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        format!("{prefix}: {stderr}")
    } else if !stdout.is_empty() {
        format!("{prefix}: {stdout}")
    } else {
        format!("{prefix}: exit {}", output.status)
    }
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    if is_dev_placeholder_version(current) {
        return false;
    }
    let left = version_parts(candidate);
    let right = version_parts(current);
    for index in 0..left.len().max(right.len()) {
        let left_value = left.get(index).copied().unwrap_or_default();
        let right_value = right.get(index).copied().unwrap_or_default();
        if left_value != right_value {
            return left_value > right_value;
        }
    }
    false
}

fn is_dev_placeholder_version(value: &str) -> bool {
    version_parts(value)
        .first()
        .is_none_or(|major| *major < 2000)
}

fn version_parts(value: &str) -> Vec<u32> {
    value
        .trim_matches(|ch: char| ch == 'v' || ch == 'V' || ch.is_whitespace())
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().unwrap_or_default())
        .collect()
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
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
