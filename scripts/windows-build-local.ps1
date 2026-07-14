[CmdletBinding()]
param(
    [ValidateSet('doctor', 'rust', 'bindings', 'dotnet', 'build', 'installer', 'zip', 'artifacts', 'run')]
    [string] $Command = 'artifacts'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Core = Join-Path $Root 'core'
$Project = Join-Path $Root 'windows\IrisChat\IrisChat.csproj'
$Target = if ($env:IRIS_WINDOWS_TARGET) { $env:IRIS_WINDOWS_TARGET } else { 'x86_64-pc-windows-msvc' }
$Configuration = if ($env:IRIS_WINDOWS_DOTNET_CONFIG) { $env:IRIS_WINDOWS_DOTNET_CONFIG } else { 'Release' }
$RustProfile = if ($env:IRIS_WINDOWS_RUST_PROFILE) { $env:IRIS_WINDOWS_RUST_PROFILE } else { 'release' }
$BindgenRef = if ($env:IRIS_WINDOWS_BINDGEN_CS_GIT_REF) { $env:IRIS_WINDOWS_BINDGEN_CS_GIT_REF } else { 'v0.10.0+v0.29.4' }
$VersionName = if ($env:IRIS_APP_VERSION_NAME) { $env:IRIS_APP_VERSION_NAME } else { '0.1.0' }
$VersionCode = if ($env:IRIS_APP_VERSION_CODE) { $env:IRIS_APP_VERSION_CODE } else { '1' }
$BuildSha = if ($env:IRIS_BUILD_GIT_SHA) { $env:IRIS_BUILD_GIT_SHA } else { 'unknown' }
$Dist = Join-Path $Root 'dist\windows'

function Invoke-Checked {
    param([scriptblock] $Script, [string] $Description)
    & $Script
    if ($LASTEXITCODE -ne 0) {
        throw "$Description failed with exit code $LASTEXITCODE"
    }
}

function Import-VisualStudioEnvironment {
    if (Get-Command link.exe -ErrorAction SilentlyContinue) { return }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { throw 'vswhere.exe was not found' }
    $install = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath |
        Select-Object -First 1
    if (-not $install) { throw 'Visual Studio C++ tools were not found' }
    $vcvars = Join-Path $install 'VC\Auxiliary\Build\vcvars64.bat'
    if (-not (Test-Path $vcvars)) { throw "vcvars64.bat was not found under $install" }
    cmd.exe /d /s /c "`"$vcvars`" >nul && set" | ForEach-Object {
        if ($_ -match '^([^=]+)=(.*)$') {
            Set-Item -Path "Env:$($matches[1])" -Value $matches[2]
        }
    }
    if (-not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
        throw 'MSVC link.exe is unavailable after loading vcvars64.bat'
    }
}

function Add-ToolPaths {
    $paths = @(
        (Join-Path $env:USERPROFILE '.cargo\bin'),
        (Join-Path $env:USERPROFILE '.dotnet\tools'),
        'C:\Program Files\LLVM\bin',
        'C:\Program Files (x86)\NSIS',
        'C:\Program Files\NSIS'
    )
    $env:PATH = (($paths + @($env:PATH)) -join ';')
}

function Get-CargoTargetDirectory {
    if (-not $env:CARGO_TARGET_DIR) { return (Join-Path $Core 'target') }
    if ([IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) { return $env:CARGO_TARGET_DIR }
    return (Join-Path $Core $env:CARGO_TARGET_DIR)
}

function Get-PublishDirectory {
    $projectDir = Split-Path -Parent $Project
    $candidates = @(
        (Join-Path $projectDir "bin\x64\$Configuration\net8.0-windows\win-x64\publish"),
        (Join-Path $projectDir "bin\$Configuration\net8.0-windows\win-x64\publish")
    )
    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) { return $candidate }
    }
    return $candidates[0]
}

function Get-VersionParts {
    $numeric = ($VersionName -split '[-+]')[0]
    $parts = @($numeric -split '\.')
    if ($parts.Count -gt 4 -or $parts.Count -eq 0) { throw "Unsupported Windows version: $VersionName" }
    foreach ($part in $parts) {
        $number = 0
        if (-not [int]::TryParse($part, [ref] $number) -or $number -lt 0 -or $number -gt 65535) {
            throw "Windows version components must be integers from 0 through 65535: $VersionName"
        }
    }
    while ($parts.Count -lt 4) { $parts += '0' }
    return $parts
}

function Invoke-Doctor {
    Import-VisualStudioEnvironment
    Add-ToolPaths
    foreach ($tool in @('rustc', 'cargo', 'dotnet', 'link.exe', 'clang.exe')) {
        $found = Get-Command $tool -ErrorAction SilentlyContinue
        if (-not $found) { throw "Required tool was not found: $tool" }
        Write-Host "[ok] $tool`: $($found.Source)"
    }
    Write-Host "[ok] target: $Target"
    Write-Host "[ok] repo: $Root"
}

function Build-Rust {
    Import-VisualStudioEnvironment
    Add-ToolPaths
    Invoke-Checked { rustup target add $Target | Out-Null } "rustup target add $Target"
    $profileFlag = if ($RustProfile -eq 'release') { @('--release') } else { @() }
    Push-Location $Core
    try {
        Invoke-Checked { cargo build --locked --target $Target @profileFlag } 'cargo build'
    } finally {
        Pop-Location
    }
}

function Build-Bindings {
    Add-ToolPaths
    $bindgen = Get-Command uniffi-bindgen-cs -ErrorAction SilentlyContinue
    if (-not $bindgen) {
        Invoke-Checked {
            cargo install uniffi-bindgen-cs --git https://github.com/NordSecurity/uniffi-bindgen-cs --tag $BindgenRef --locked
        } 'install uniffi-bindgen-cs'
    }
    $library = Join-Path (Get-CargoTargetDirectory) "$Target\$RustProfile\iris_chat_core.dll"
    if (-not (Test-Path $library)) { throw "Rust DLL was not found: $library" }
    $bindings = Join-Path $Root 'windows\IrisChat\Bindings'
    $frameworks = Join-Path $Root 'windows\IrisChat\Frameworks'
    Remove-Item $bindings -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $bindings, $frameworks | Out-Null
    Push-Location $Core
    try {
        Invoke-Checked {
            uniffi-bindgen-cs --library --out-dir $bindings --config (Join-Path $Core 'uniffi.toml') $library
        } 'uniffi-bindgen-cs'
    } finally {
        Pop-Location
    }
    Copy-Item $library (Join-Path $frameworks 'iris_chat_core.dll') -Force
}

function Build-Dotnet {
    Add-ToolPaths
    $parts = Get-VersionParts
    $assemblyVersion = $parts -join '.'
    $informationalVersion = "$VersionName+$BuildSha"
    Invoke-Checked {
        dotnet publish $Project -c $Configuration -r win-x64 --self-contained true `
            -p:WindowsPackageType=None "-p:Version=$VersionName" `
            "-p:AssemblyVersion=$assemblyVersion" "-p:FileVersion=$assemblyVersion" `
            "-p:InformationalVersion=$informationalVersion"
    } 'dotnet publish'
    $publish = Get-PublishDirectory
    if (-not (Test-Path (Join-Path $publish 'IrisChat.exe'))) { throw "Published app was not found: $publish" }
    if (-not (Test-Path (Join-Path $publish 'iris_chat_core.dll'))) { throw 'Published app is missing iris_chat_core.dll' }
}

function Build-All {
    Build-Rust
    Build-Bindings
    Build-Dotnet
}

function Build-Zip {
    $publish = Get-PublishDirectory
    if (-not (Test-Path $publish)) { throw "Publish directory was not found: $publish" }
    New-Item -ItemType Directory -Force -Path $Dist | Out-Null
    $output = Join-Path $Dist "iris-chat-v$VersionName-windows-x64.zip"
    Remove-Item $output -Force -ErrorAction SilentlyContinue
    Compress-Archive -Path (Join-Path $publish '*') -DestinationPath $output -CompressionLevel Optimal
    if (-not (Test-Path $output)) { throw "ZIP was not produced: $output" }
    Write-Output $output
}

function Find-MakeNsis {
    Add-ToolPaths
    $command = Get-Command makensis.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    throw 'makensis.exe was not found; install NSIS first'
}

function Build-Installer {
    $publish = Get-PublishDirectory
    if (-not (Test-Path $publish)) { throw "Publish directory was not found: $publish" }
    $parts = Get-VersionParts
    $marketingVersion = ($parts[0..2] -join '.')
    $buildNumber = $parts[3]
    New-Item -ItemType Directory -Force -Path $Dist | Out-Null
    $output = Join-Path $Dist "iris-chat-v$VersionName-windows-x64-setup.exe"
    Remove-Item $output -Force -ErrorAction SilentlyContinue
    $nsiArgs = @(
        "/DIRIS_VERSION=$marketingVersion",
        "/DIRIS_BUILD_NUM=$buildNumber",
        "/DIRIS_PUBLISH_DIR=$publish",
        "/DIRIS_OUTPUT=$output",
        '/DIRIS_EXE_NAME=IrisChat.exe'
    )
    $icon = Join-Path $Root 'windows\IrisChat\Resources\IrisChat.ico'
    if (Test-Path $icon) { $nsiArgs += "/DIRIS_ICON_PATH=$icon" }
    $nsiArgs += (Join-Path $Root 'scripts\windows-installer.nsi')
    $makeNsis = Find-MakeNsis
    Invoke-Checked { & $makeNsis @nsiArgs } 'makensis'
    if (-not (Test-Path $output)) { throw "Installer was not produced: $output" }
    Write-Output $output
}

function Run-App {
    $exe = Join-Path (Get-PublishDirectory) 'IrisChat.exe'
    if (-not (Test-Path $exe)) { throw "App was not found: $exe" }
    Start-Process $exe
}

switch ($Command) {
    'doctor' { Invoke-Doctor }
    'rust' { Build-Rust }
    'bindings' { Build-Bindings }
    'dotnet' { Build-Dotnet }
    'build' { Build-All }
    'installer' { Build-Installer }
    'zip' { Build-Zip }
    'artifacts' { Build-All; Build-Installer; Build-Zip }
    'run' { Run-App }
}
