$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'windows-build-inputs.ps1')
$temporary = Join-Path ([IO.Path]::GetTempPath()) ("iris-input-tests-" + [guid]::NewGuid())
New-Item -ItemType Directory (Join-Path $temporary 'bin') -Force | Out-Null
$passed = 0
function Assert-Rejected([scriptblock] $Action, [string] $Message) {
    $rejected = $false
    try { & $Action } catch { $rejected = $_.Exception.Message -like "*$Message*" }
    if (-not $rejected) { throw "Expected rejection containing: $Message" }
    $script:passed++
}
try {
    $revision = 'e4f18be96ca812571fd85fc39555646158b06b9c'
    if ($null -ne (Get-PinnedBindgen $temporary $revision)) { throw 'Missing tool was accepted.' }
    $passed++
    $executable = Join-Path $temporary 'bin\uniffi-bindgen-cs.exe'
    Set-Content $executable 'test fixture: never executed'
    Assert-Rejected { Get-PinnedBindgen $temporary $revision } 'no Cargo installation metadata'
    $metadata = Join-Path $temporary '.crates2.json'
    $identity = "uniffi-bindgen-cs 0.10.0+v0.29.4 (git+https://github.com/NordSecurity/uniffi-bindgen-cs?rev=$revision#$revision)"
    @{installs = @{$identity = @{bins = @('uniffi-bindgen-cs.exe')}}} | ConvertTo-Json -Depth 5 | Set-Content $metadata
    if ((Get-PinnedBindgen $temporary $revision) -ne $executable) { throw 'Pinned tool path was not selected.' }
    $passed++
    Assert-Rejected { Get-PinnedBindgen $temporary ('0' * 40) } 'does not match pinned commit'
    (Get-Content -Raw $metadata).Replace('NordSecurity', 'different-owner') | Set-Content $metadata
    Assert-Rejected { Get-PinnedBindgen $temporary $revision } 'does not match pinned commit'

    $archiveDirectory = Join-Path $temporary 'example.sdk\1.0.0'
    New-Item -ItemType Directory $archiveDirectory -Force | Out-Null
    $archive = Join-Path $archiveDirectory 'example.sdk.1.0.0.nupkg'
    Set-Content $archive 'reviewed SDK package fixture'
    $pinsPath = Join-Path $temporary 'pins.json'
    @(
        @{id = 'Unused.SDK'; version = '1.0.0'; sha512 = ('0' * 128)}
        @{id = 'Example.SDK'; version = '1.0.0'; sha512 = (Get-FileHash $archive -Algorithm SHA512).Hash}
    ) |
        ConvertTo-Json | Set-Content $pinsPath
    $assetsPath = Join-Path $temporary 'assets.json'
    $assets = @{packageFolders = @{$temporary = @{}}; project = @{frameworks = @{'net8.0' = @{downloadDependencies = @(@{name = 'Example.SDK'; version = '[1.0.0, 1.0.0]'})}}}}
    $assets | ConvertTo-Json -Depth 8 | Set-Content $assetsPath
    Assert-PinnedDownloadPackages $assetsPath $pinsPath
    $passed++
    Add-Content $archive 'modified bytes'
    Assert-Rejected { Assert-PinnedDownloadPackages $assetsPath $pinsPath } 'content hash mismatch'
    (Get-Content -Raw $assetsPath).Replace('[1.0.0, 1.0.0]', '[1.0.1, 1.0.1]') | Set-Content $assetsPath
    Assert-Rejected { Assert-PinnedDownloadPackages $assetsPath $pinsPath } 'no exact reviewed pin'
    (Get-Content -Raw $assetsPath).Replace('Example.SDK', 'Unknown.SDK') | Set-Content $assetsPath
    Assert-Rejected { Assert-PinnedDownloadPackages $assetsPath $pinsPath } 'no exact reviewed pin'
    Write-Output "$passed Windows build input checks passed."
} finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force
}
