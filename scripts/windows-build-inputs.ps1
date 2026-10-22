# Checks used by the native Windows build before consuming downloaded inputs.
function Get-PinnedBindgen {
    param([string] $CargoRoot, [string] $Revision)
    $executable = Join-Path $CargoRoot 'bin\uniffi-bindgen-cs.exe'
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { return $null }
    $metadata = Join-Path $CargoRoot '.crates2.json'
    if (-not (Test-Path -LiteralPath $metadata -PathType Leaf)) {
        throw 'Installed uniffi-bindgen-cs has no Cargo installation metadata. Reinstall the pinned revision.'
    }
    $installed = (Get-Content -Raw -LiteralPath $metadata | ConvertFrom-Json).installs
    $entries = @($installed.PSObject.Properties | Where-Object {
        $_.Name -match '^uniffi-bindgen-cs [^ ]+ \(git\+https://github\.com/NordSecurity/uniffi-bindgen-cs(?:\?[^#)]*)?#([0-9a-fA-F]{40})\)$' -and
        $Matches[1] -eq $Revision -and 'uniffi-bindgen-cs.exe' -in $_.Value.bins
    })
    if ($entries.Count -ne 1) {
        throw "Installed uniffi-bindgen-cs does not match pinned commit $Revision. Reinstall that revision with Cargo before building."
    }
    return $executable
}

function Assert-PinnedDownloadPackages {
    param([string] $AssetsPath, [string] $PinsPath)
    $assets = Get-Content -Raw -LiteralPath $AssetsPath | ConvertFrom-Json
    $pins = Get-Content -Raw -LiteralPath $PinsPath | ConvertFrom-Json
    $checked = @{}
    foreach ($framework in $assets.project.frameworks.PSObject.Properties) {
        $downloads = $framework.Value.PSObject.Properties['downloadDependencies']
        if (-not $downloads) { continue }
        foreach ($package in $downloads.Value) {
            $pin = @($pins | Where-Object { $_.id -eq $package.name })
            if ($pin.Count -ne 1 -or $package.version -ne "[$($pin[0].version), $($pin[0].version)]") {
                throw "SDK download has no exact reviewed pin: $($package.name) $($package.version)"
            }
            $id = $pin[0].id.ToLowerInvariant()
            $version = $pin[0].version
            $key = "$id/$version"
            if ($checked.ContainsKey($key)) { continue }
            $archives = @($assets.packageFolders.PSObject.Properties | ForEach-Object {
                $candidate = Join-Path $_.Name "$id\$version\$id.$version.nupkg"
                if (Test-Path -LiteralPath $candidate -PathType Leaf) { $candidate }
            })
            if ($archives.Count -eq 0) { throw "Restored SDK package archive is missing: $key" }
            foreach ($archive in $archives) {
                $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA512).Hash
                if ($actual -ne $pin[0].sha512) { throw "SDK package content hash mismatch: $key" }
            }
            $checked[$key] = $true
        }
    }
    if ($checked.Count -eq 0) { throw 'The restore did not record any SDK download packages.' }
}
