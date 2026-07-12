param(
  [string]$AppPath = $env:IRIS_CHAT_WINDOWS_IDLE_CPU_APP,
  [double]$MaxPercent = -1,
  [double]$SettleSeconds = -1,
  [double]$SampleSeconds = -1
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
if ($MaxPercent -lt 0) { $MaxPercent = if ($env:IRIS_CHAT_IDLE_CPU_MAX_PERCENT) { [double]$env:IRIS_CHAT_IDLE_CPU_MAX_PERCENT } else { 5 } }
if ($SettleSeconds -lt 0) { $SettleSeconds = if ($env:IRIS_CHAT_IDLE_CPU_SETTLE_SECONDS) { [double]$env:IRIS_CHAT_IDLE_CPU_SETTLE_SECONDS } else { 30 } }
if ($SampleSeconds -le 0) { $SampleSeconds = if ($env:IRIS_CHAT_IDLE_CPU_SAMPLE_SECONDS) { [double]$env:IRIS_CHAT_IDLE_CPU_SAMPLE_SECONDS } else { 60 } }

$RunId = 'idle-cpu-' + (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssZ') + '-' + $PID
$RunDir = if ($env:IRIS_CHAT_IDLE_CPU_ARTIFACT_ROOT) {
  Join-Path $env:IRIS_CHAT_IDLE_CPU_ARTIFACT_ROOT ('windows-' + $RunId)
} else {
  Join-Path (Join-Path $Root 'artifacts\idle-cpu') ('windows-' + $RunId)
}
$DataDir = Join-Path $env:TEMP ('iris-chat-' + $RunId)
$ResultPath = Join-Path $RunDir 'result.json'
$FixturePath = Join-Path $RunDir 'fixture.json'
$PeerHex = '1111111111111111111111111111111111111111111111111111111111111111'
New-Item -ItemType Directory -Force -Path $RunDir, $DataDir | Out-Null

$App = $null
try {
  cargo build --manifest-path (Join-Path $Root 'core\Cargo.toml') --bin iris
  if ($LASTEXITCODE -ne 0) { throw "iris CLI build failed ($LASTEXITCODE)" }
  $Metadata = cargo metadata --manifest-path (Join-Path $Root 'core\Cargo.toml') --format-version 1 --no-deps | ConvertFrom-Json
  $Iris = Join-Path $Metadata.target_directory 'debug\iris.exe'
  if (!(Test-Path $Iris)) { throw "Missing iris CLI: $Iris" }

  & $Iris --json --data-dir $DataDir relay set ws://127.0.0.1:9 | Out-File (Join-Path $DataDir 'relay.json')
  & $Iris --json --data-dir $DataDir account create --name 'Idle CPU Alice' | Out-File (Join-Path $DataDir 'account.out.json')
  if (!(Test-Path (Join-Path $DataDir 'cli-account.json'))) { throw 'Account fixture was not persisted' }
  & $Iris --json --data-dir $DataDir chat create $PeerHex | Out-File (Join-Path $DataDir 'direct.json')
  if ($LASTEXITCODE -ne 0) { throw "Direct chat fixture failed ($LASTEXITCODE)" }
  & $Iris --json --data-dir $DataDir group create 'Idle CPU group' | Out-File (Join-Path $DataDir 'group.out.json')
  & $Iris --json --data-dir $DataDir relay reset | Out-File (Join-Path $DataDir 'relay-reset.json')
  $StateRaw = & $Iris --json --data-dir $DataDir state
  if ($LASTEXITCODE -ne 0) { throw "Fixture state read failed ($LASTEXITCODE)" }
  $State = $StateRaw | ConvertFrom-Json
  $DirectCount = @($State.data.chats | Where-Object { $_.kind -eq 'direct' }).Count
  $GroupCount = @($State.data.chats | Where-Object { $_.kind -eq 'group' }).Count
  if (!$State.data.account -or $DirectCount -lt 1 -or $GroupCount -lt 1) {
    throw "Fixture invalid: loggedIn=$([bool]$State.data.account) direct=$DirectCount group=$GroupCount"
  }

  $CliSecret = Get-Content (Join-Path $DataDir 'cli-account.json') -Raw | ConvertFrom-Json
  @{
    OwnerNsec = $CliSecret.owner_nsec
    OwnerPubkeyHex = $CliSecret.owner_pubkey_hex
    DeviceNsec = $CliSecret.device_nsec
  } | ConvertTo-Json -Compress | Set-Content (Join-Path $DataDir 'account-secret.json') -Encoding utf8
  $Fixture = [ordered]@{
    loggedIn = $true
    directChatCount = $DirectCount
    groupChatCount = $GroupCount
  }
  $Fixture | ConvertTo-Json | Set-Content $FixturePath -Encoding utf8

  if (!$AppPath) {
    $Candidate = Get-ChildItem (Join-Path $Root 'windows\IrisChat\bin') -Recurse -Filter IrisChat.exe -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -like '*\publish\IrisChat.exe' } |
      Sort-Object LastWriteTime -Descending |
      Select-Object -First 1
    if (!$Candidate) { throw 'No published IrisChat.exe found; run scripts/windows-build windows-build first' }
    $AppPath = $Candidate.FullName
  }
  if (!(Test-Path $AppPath)) { throw "Missing Windows app: $AppPath" }

  $env:IRIS_UI_TEST_RUN_ID = $RunId
  $env:IRIS_UI_TEST_DATA_DIR = $DataDir
  $App = Start-Process -FilePath $AppPath -PassThru
  Start-Sleep -Milliseconds ([int]($SettleSeconds * 1000))
  $App.Refresh()
  if ($App.HasExited) { throw 'Windows Iris Chat exited before idle sampling' }
  $StartCpu = $App.TotalProcessorTime.TotalSeconds
  $Watch = [Diagnostics.Stopwatch]::StartNew()
  Start-Sleep -Milliseconds ([int]($SampleSeconds * 1000))
  $Watch.Stop()
  $App.Refresh()
  if ($App.HasExited) { throw 'Windows Iris Chat exited during idle sampling' }
  $CpuPercent = [Math]::Max(0, $App.TotalProcessorTime.TotalSeconds - $StartCpu) * 100 / [Math]::Max(0.001, $Watch.Elapsed.TotalSeconds)
  $Ok = $CpuPercent -le $MaxPercent
  [ordered]@{
    ok = $Ok
    mode = 'windows-process'
    label = 'Windows Iris Chat'
    cpuPercent = $CpuPercent
    maxPercent = $MaxPercent
    settleSeconds = $SettleSeconds
    sampleSeconds = $SampleSeconds
    processId = $App.Id
    fixture = $Fixture
    generatedAt = (Get-Date).ToUniversalTime().ToString('o')
  } | ConvertTo-Json -Depth 5 | Set-Content $ResultPath -Encoding utf8
  if (!$Ok) { throw ("Windows Iris Chat idle CPU {0:N3}% > {1:N3}%" -f $CpuPercent, $MaxPercent) }
  Write-Host ("Windows Iris Chat idle CPU ok: {0:N3}% <= {1:N3}%" -f $CpuPercent, $MaxPercent)
  Write-Host "Result: $ResultPath"
} finally {
  if ($App -and !$App.HasExited) { Stop-Process -Id $App.Id -Force -ErrorAction SilentlyContinue }
  Remove-Item Env:IRIS_UI_TEST_RUN_ID -ErrorAction SilentlyContinue
  Remove-Item Env:IRIS_UI_TEST_DATA_DIR -ErrorAction SilentlyContinue
  Remove-Item $DataDir -Recurse -Force -ErrorAction SilentlyContinue
}
