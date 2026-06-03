use super::*;

pub(super) fn preferred_manifest_asset(
    manifest: &ReleaseManifest,
    request: &UpdateRequest,
) -> Option<ReleaseAsset> {
    match request.mode {
        UpdateMode::Cli => {
            preferred_cli_asset(&manifest.assets, &manifest.tag, request.preferred_kind())
        }
        UpdateMode::App => preferred_app_asset(&manifest.assets),
    }
}

pub(super) fn preferred_secure_asset(
    manifest: &UpdateManifest,
    request: &UpdateRequest,
) -> Option<UpdateAsset> {
    match request.mode {
        UpdateMode::Cli => preferred_secure_cli_asset(manifest, request.preferred_kind()),
        UpdateMode::App => preferred_secure_app_asset(manifest),
    }
}

pub(super) fn preferred_cli_asset(
    assets: &[ReleaseAsset],
    tag: &str,
    kind: Option<&str>,
) -> Option<ReleaseAsset> {
    if let Some(kind) = kind.filter(|value| !value.trim().is_empty()) {
        if let Some(asset) = assets.iter().find(|asset| asset.name.contains(kind)) {
            return Some(asset.clone());
        }
    }

    let target = current_target();
    let archive_ext = if cfg!(target_os = "windows") {
        ".zip"
    } else {
        ".tar.gz"
    };
    let exact = format!("iris-{tag}-{target}{archive_ext}");
    let unversioned = format!("iris-{target}{archive_ext}");
    assets
        .iter()
        .find(|asset| asset.name == exact)
        .or_else(|| assets.iter().find(|asset| asset.name == unversioned))
        .or_else(|| {
            assets.iter().find(|asset| {
                asset.name.starts_with("iris-")
                    && asset.name.contains(target)
                    && asset.name.ends_with(archive_ext)
            })
        })
        .cloned()
}

pub(super) fn preferred_secure_cli_asset(
    manifest: &UpdateManifest,
    kind: Option<&str>,
) -> Option<UpdateAsset> {
    if let Some(kind) = kind.filter(|value| !value.trim().is_empty()) {
        if let Some(asset) = manifest
            .assets
            .iter()
            .find(|asset| asset.name.contains(kind))
        {
            return Some(asset.clone());
        }
    }

    let tag = display_manifest_tag(manifest);
    let target = current_target();
    let archive_ext = if cfg!(target_os = "windows") {
        ".zip"
    } else {
        ".tar.gz"
    };
    let exact = format!("iris-{tag}-{target}{archive_ext}");
    let unversioned = format!("iris-{target}{archive_ext}");
    manifest
        .assets
        .iter()
        .find(|asset| asset.name == exact)
        .or_else(|| {
            manifest
                .assets
                .iter()
                .find(|asset| asset.name == unversioned)
        })
        .or_else(|| {
            manifest.assets.iter().find(|asset| {
                asset.name.starts_with("iris-")
                    && asset.name.contains(target)
                    && asset.name.ends_with(archive_ext)
            })
        })
        .cloned()
}

pub(super) fn preferred_app_asset(assets: &[ReleaseAsset]) -> Option<ReleaseAsset> {
    assets
        .iter()
        .find(|asset| app_asset_name_matches_current_target(&asset.name))
        .cloned()
}

pub(super) fn preferred_secure_app_asset(manifest: &UpdateManifest) -> Option<UpdateAsset> {
    manifest
        .assets
        .iter()
        .find(|asset| app_asset_name_matches_current_target(&asset.name))
        .cloned()
}

pub(super) fn app_asset_name_matches_current_target(name: &str) -> bool {
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

pub(super) fn display_manifest_tag(manifest: &UpdateManifest) -> String {
    manifest
        .tag
        .clone()
        .filter(|tag| !tag.trim().is_empty())
        .unwrap_or_else(|| format!("v{}", manifest.effective_version()))
}

pub(super) fn current_target() -> &'static str {
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

pub(super) fn manifest_asset_url(manifest_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("file://") {
        return path.to_string();
    }
    let base = manifest_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(manifest_url);
    format!("{}/{}", base, path.trim_start_matches('/'))
}
