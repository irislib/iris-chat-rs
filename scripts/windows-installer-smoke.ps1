param(
    [Parameter(Mandatory = $true)]
    [string] $InstallerPath,
    [int] $AliveSeconds = 5
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$InstallerPath = [IO.Path]::GetFullPath($InstallerPath)
if (-not (Test-Path $InstallerPath)) { throw "Windows installer not found: $InstallerPath" }
if ($AliveSeconds -lt 1) { throw 'AliveSeconds must be positive' }

$TempRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { $env:TEMP }
$RunId = 'release-smoke-' + (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssZ') + '-' + $PID
$InstallDir = Join-Path $TempRoot "iris-chat-install-$PID"
$DataDir = Join-Path $TempRoot "iris-chat-data-$PID"
$App = $null

Remove-Item $InstallDir, $DataDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

try {
    $Setup = Start-Process -FilePath $InstallerPath `
        -ArgumentList @('/S', "/D=$InstallDir") -Wait -PassThru
    if ($Setup.ExitCode -ne 0) { throw "Installer exited with code $($Setup.ExitCode)" }

    $AppPath = Join-Path $InstallDir 'IrisChat.exe'
    $CorePath = Join-Path $InstallDir 'iris_chat_core.dll'
    if (-not (Test-Path $AppPath)) { throw "Installed app not found: $AppPath" }
    if (-not (Test-Path $CorePath)) { throw "Installed core library not found: $CorePath" }

    $env:IRIS_UI_TEST_RESET = '1'
    $env:IRIS_UI_TEST_RUN_ID = $RunId
    $env:IRIS_UI_TEST_DATA_DIR = $DataDir
    $env:IRIS_DISABLE_NOTIFICATIONS_FOR_AUTOMATION = '1'
    $App = Start-Process -FilePath $AppPath -PassThru
    Start-Sleep -Seconds $AliveSeconds
    $App.Refresh()
    if ($App.HasExited) { throw 'Installed Windows app exited during startup' }

    Write-Host 'WINDOWS_INSTALLER_SMOKE_OK'
} finally {
    if ($App -and -not $App.HasExited) {
        Stop-Process -Id $App.Id -Force -ErrorAction SilentlyContinue
        Wait-Process -Id $App.Id -ErrorAction SilentlyContinue
    }
    Remove-Item Env:IRIS_UI_TEST_RESET -ErrorAction SilentlyContinue
    Remove-Item Env:IRIS_UI_TEST_RUN_ID -ErrorAction SilentlyContinue
    Remove-Item Env:IRIS_UI_TEST_DATA_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:IRIS_DISABLE_NOTIFICATIONS_FOR_AUTOMATION -ErrorAction SilentlyContinue

    $Uninstaller = Join-Path $InstallDir 'Uninstall.exe'
    if (Test-Path $Uninstaller) {
        Start-Process -FilePath $Uninstaller -ArgumentList '/S' -Wait -ErrorAction SilentlyContinue |
            Out-Null
    }
    Remove-Item $InstallDir, $DataDir -Recurse -Force -ErrorAction SilentlyContinue
}
