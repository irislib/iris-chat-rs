use super::*;

pub(super) fn download_asset(url: &str, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
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

pub(super) fn curl_command(max_time: &str) -> Command {
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

pub(super) fn selected_download_path(
    out: Option<&Path>,
    download_dir: Option<&Path>,
    asset_name: &str,
    temp_dir: &Path,
) -> Result<PathBuf> {
    if let Some(out) = out {
        if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        return Ok(out.to_path_buf());
    }

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

pub(super) fn write_downloaded_asset(destination: &Path, bytes: &[u8]) -> Result<()> {
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

pub(super) fn install_cli_archive(
    archive_path: &Path,
    temp_dir: &Path,
    destination: Option<&Path>,
) -> Result<()> {
    extract_archive(archive_path, temp_dir)?;
    let binary = find_iris_binary(temp_dir)?;
    let destination = destination
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(|| {
            std::env::current_exe().context("failed to resolve current executable")
        })?;
    install_binary(&binary, &destination)
}

pub(super) fn install_destination(destination: Option<&Path>) -> Result<PathBuf> {
    destination
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(|| std::env::current_exe().context("failed to resolve current executable"))
}

pub(super) fn extract_archive(archive_path: &Path, destination: &Path) -> Result<()> {
    let mut command = Command::new("tar");
    if archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".tar.gz") || name.ends_with(".tgz"))
    {
        command.arg("-xzf");
    } else {
        command.arg("-xf");
    }
    let output = command
        .arg(archive_path)
        .arg("-C")
        .arg(destination)
        .output()
        .with_context(|| format!("failed to extract {}", archive_path.display()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{}",
            command_error("archive extraction failed", &output)
        ));
    }
    Ok(())
}

pub(super) fn find_iris_binary(root: &Path) -> Result<PathBuf> {
    let binary_name = if cfg!(target_os = "windows") {
        "iris.exe"
    } else {
        "iris"
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in
            fs::read_dir(&path).with_context(|| format!("failed to read {}", path.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file()
                && path.file_name().and_then(|value| value.to_str()) == Some(binary_name)
            {
                return Ok(path);
            }
        }
    }
    Err(anyhow!("downloaded archive did not contain {binary_name}"))
}

pub(super) fn install_binary(source: &Path, destination: &Path) -> Result<()> {
    let parent = install_parent(destination)?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory {}", parent.display()))?;
    let temp_path = parent.join(format!(
        ".iris-update-{}-{}{}",
        std::process::id(),
        unix_timestamp(),
        std::env::consts::EXE_SUFFIX
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    fs::copy(source, &temp_path).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            temp_path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o755)).with_context(|| {
            format!(
                "failed to set executable permissions on {}",
                temp_path.display()
            )
        })?;
    }
    #[cfg(target_os = "windows")]
    if destination.exists() {
        fs::remove_file(destination)
            .with_context(|| format!("failed to replace {}", destination.display()))?;
    }
    fs::rename(&temp_path, destination).with_context(|| {
        format!(
            "failed to move {} into {}",
            temp_path.display(),
            destination.display()
        )
    })?;
    Ok(())
}

pub(super) fn install_parent(destination: &Path) -> Result<&Path> {
    if destination.as_os_str().is_empty() {
        return Err(anyhow!("install path must not be empty"));
    }
    if destination.is_dir() {
        return Err(anyhow!(
            "install path points to a directory: {}",
            destination.display()
        ));
    }
    destination.parent().ok_or_else(|| {
        anyhow!(
            "install path must include parent directory: {}",
            destination.display()
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn print_update_check(
    mode: UpdateMode,
    available: bool,
    tag: &str,
    asset: &str,
    source: &'static str,
    verified: bool,
    url: Option<&str>,
    json: bool,
    path: Option<&Path>,
) -> Result<()> {
    if json {
        print_update_json(UpdateJson {
            available,
            current_version: current_version(),
            latest_version: tag.trim_start_matches(['v', 'V']).to_string(),
            tag: tag.to_string(),
            asset: asset.to_string(),
            source,
            verified,
            url: url.map(ToOwned::to_owned),
            path: path.map(|value| value.display().to_string()),
        })?;
        return Ok(());
    }

    if available {
        println!("update available: {} -> {tag}", current_version());
    } else {
        println!("{} {} is up to date", mode.noun(), current_version());
    }
    println!("asset={asset}");
    println!("source={source}");
    println!("verified={verified}");
    if let Some(url) = url {
        println!("url={url}");
    }
    if let Some(path) = path {
        println!("path={}", path.display());
    }
    Ok(())
}

pub(super) fn print_downloaded(download: DownloadedUpdate<'_>, json: bool) -> Result<()> {
    if json {
        print_update_json(UpdateJson {
            available: download.available,
            current_version: current_version(),
            latest_version: download.tag.trim_start_matches(['v', 'V']).to_string(),
            tag: download.tag.to_string(),
            asset: download.asset.to_string(),
            source: download.source,
            verified: download.verified,
            url: download.url.map(ToOwned::to_owned),
            path: Some(download.path.display().to_string()),
        })?;
        return Ok(());
    }
    println!("downloaded {}", download.asset);
    println!("path={}", download.path.display());
    println!("source={}", download.source);
    println!("verified={}", download.verified);
    if let Some(url) = download.url {
        println!("url={url}");
    }
    Ok(())
}

pub(super) fn print_up_to_date(
    mode: UpdateMode,
    tag: &str,
    source: &'static str,
    verified: bool,
    json: bool,
) -> Result<()> {
    if json {
        print_update_json(UpdateJson {
            available: false,
            current_version: current_version(),
            latest_version: tag.trim_start_matches(['v', 'V']).to_string(),
            tag: tag.to_string(),
            asset: String::new(),
            source,
            verified,
            url: None,
            path: None,
        })?;
        return Ok(());
    }
    println!("{} {} is up to date", mode.noun(), current_version());
    Ok(())
}

pub(super) fn print_update_json(output: UpdateJson<'_>) -> Result<()> {
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

pub(super) fn current_version() -> &'static str {
    env!("IRIS_APP_VERSION")
}

pub(super) fn create_temp_dir(prefix: &str) -> Result<PathBuf> {
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

pub(super) fn safe_file_name(name: &str) -> String {
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

pub(super) fn command_error(prefix: &str, output: &std::process::Output) -> String {
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

pub(super) fn version_is_newer(candidate: &str, current: &str) -> bool {
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

pub(super) fn is_dev_placeholder_version(value: &str) -> bool {
    version_parts(value)
        .first()
        .map_or(true, |major| *major < 2000)
}

pub(super) fn version_parts(value: &str) -> Vec<u32> {
    value
        .trim_matches(|ch: char| ch == 'v' || ch == 'V' || ch.is_whitespace())
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().unwrap_or_default())
        .collect()
}

pub(super) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
