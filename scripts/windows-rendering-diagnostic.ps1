[CmdletBinding()]
param(
    [string] $OutputDirectory = 'work/rendering-comparison',
    [string] $ExpectedSource = $env:GITHUB_SHA
)

# Temporary diagnostic only. This script never builds, retries, or qualifies a release.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$outputRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot $OutputDirectory))
if (Test-Path $outputRoot) { throw 'Comparison output must be a fresh directory' }
[IO.Directory]::CreateDirectory($outputRoot) | Out-Null
$totalClock = [Diagnostics.Stopwatch]::StartNew()
$totalLimitMs = 840000
$modes = @('persistent', 'one-shot', 'one-shot', 'persistent')

function Write-ExclusiveJson($Path, $Value) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 12) + "`n")
        $stream.Write($bytes, 0, $bytes.Length)
    } finally { $stream.Dispose() }
}

function Get-GitValue([string[]] $GitArguments) {
    $value = & git -C $repoRoot @GitArguments
    if ($LASTEXITCODE -ne 0) { throw 'Git provenance query failed' }
    return ($value -join "`n").Trim()
}

function Get-BinaryManifest([string] $Directory) {
    return @(Get-ChildItem -LiteralPath $Directory -Recurse -File |
        Where-Object { $_.Extension -in @('.exe', '.dll', '.json') } |
        Sort-Object FullName | ForEach-Object {
            [ordered]@{
                path = [IO.Path]::GetRelativePath($Directory, $_.FullName)
                bytes = $_.Length
                sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            }
        })
}

$plan = [ordered]@{
    schema = 1; diagnosticOnly = $true; fullReleaseQualification = $false
    order = $modes; childTimeoutMs = 180000; cleanupTimeoutMs = 10000
    logDrainTimeoutMs = 5000; totalTimeoutMs = $totalLimitMs
    changedVariable = 'Only persistent versus one-shot target Rendering subscription; recurring 1 ms target heartbeat remains ON'
    targetHeartbeat = 'on'
    measuredOutcome = 'Target subsequent_edit_settle_ms and first_dispatcher_drain wall_ms; unchanged assertions'
    callbackSeparation = 'Both persistent arms must record more than one callback; both one-shot arms must record exactly one callback and an unsubscribe timestamp before Finish'
    timingSeparation = 'Both persistent controls must reproduce the unchanged target 500 ms failure; both one-shot children must pass all original assertions and their target edit and first-drain times must be below both persistent values; overlapping or opposite ranges are inconclusive'
    acceptanceGuard = 'If persistent controls do not reproduce, a one-shot child fails, callback lifecycle evidence is missing, or timing follows process order instead of mode, stop inconclusive. Subscription end is recorded directly in the first callback, not inferred from Finish'
    decisionRule = 'All four valid lifecycle measurements plus callback separation and timing separation support observer contribution only, never the original failure mechanism or release qualification. Any absent, invalid or inconsistent separation stops inconclusive. No retries or favorable-sample selection.'
    separateBaseline = 'Existing default clipboard workflow step; not one of these four arms'
}
Write-ExclusiveJson (Join-Path $outputRoot 'plan.json') $plan
$source = Get-GitValue @('rev-parse', 'HEAD')
if ($ExpectedSource -notmatch '^[0-9a-f]{40}$' -or $source -ne $ExpectedSource) { throw 'Unexpected source SHA' }
if (Get-GitValue @('status', '--porcelain', '--untracked-files=no')) { throw 'Tracked source is dirty' }
$binaryDirectory = Join-Path $repoRoot 'windows/ClipboardTests/bin/Release/net8.0-windows10.0.17763.0/win-x64'
$executable = Join-Path $binaryDirectory 'ClipboardTests.exe'
foreach ($required in @($executable, (Join-Path $binaryDirectory 'ClipboardTests.dll'), (Join-Path $binaryDirectory 'IrisChat.dll'), (Join-Path $binaryDirectory 'iris_chat_core.dll'))) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Missing compiled comparison input: $required" }
}
$baselineBinaries = Get-BinaryManifest $binaryDirectory
$baselineFingerprint = ConvertTo-Json -InputObject $baselineBinaries -Depth 6 -Compress
$sourceFiles = @('windows/ClipboardTests/Program.cs', 'windows/ClipboardTests/LargeTextPasteTests.cs',
    'windows/ClipboardTests/PasteLayoutProbe.cs', 'windows/ClipboardTests/EditPhaseDiagnostics.cs',
    'windows/ClipboardTests/DispatcherIntervalDiagnostics.cs', 'windows/ClipboardTests/WindowsCpuSample.cs',
    'scripts/windows-rendering-diagnostic.ps1', '.github/workflows/ci.yml')
$provenance = [ordered]@{
    source = $source
    nativeCoreTree = Get-GitValue @('rev-parse', ($source + ':core'))
    protocolTree = Get-GitValue @('rev-parse', ($source + ':chat-protocol'))
    windowsProductTree = Get-GitValue @('rev-parse', ($source + ':windows/IrisChat'))
    binaries = $baselineBinaries
    sourceFiles = @($sourceFiles | ForEach-Object {
        [ordered]@{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $repoRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
}
Write-ExclusiveJson (Join-Path $outputRoot 'provenance.json') $provenance
$results = @()
$haltReason = $null
for ($index = 0; $index -lt $modes.Count; $index++) {
    $mode = $modes[$index]
    $arm = ('{0:D2}-{1}' -f ($index + 1), $mode)
    $armDirectory = Join-Path $outputRoot $arm
    [IO.Directory]::CreateDirectory($armDirectory) | Out-Null
    $result = [ordered]@{
        arm = $arm; mode = $mode; source = $source; started = $false; processId = $null
        startedAt = $null; completedAt = $null; exitCode = $null; timedOut = $false
        ownedProcessExited = $false; logDrainComplete = $false; binariesUnchanged = $false
        generatedFileCleanupVerified = $false; generatedFileCleanupChecked = $false
        generatedFileCount = $null; generatedFileRemainingCount = $null; generatedFileCleanupError = $null
        timingAvailable = $false; targetModeVerified = $false
        error = $null
    }
    Write-ExclusiveJson (Join-Path $armDirectory 'arguments.json') ([ordered]@{
        arm = $arm; executable = $executable; arguments = @('--clipboard-child', $armDirectory)
        environment = @{ IRIS_CLIPBOARD_TARGET_HEARTBEAT = 'on'; IRIS_CLIPBOARD_TARGET_RENDERING = $mode }; source = $source
    })
    $child = $null; $stdout = $null; $stderr = $null; $cancelCopies = $null; $cleanupAttempted = $false
    try {
        if ($haltReason) { throw $haltReason }
        $remainingMs = $totalLimitMs - $totalClock.ElapsedMilliseconds - 20000
        if ($remainingMs -le 0) { $haltReason = 'Total comparison deadline reached'; throw $haltReason }
        $before = Get-BinaryManifest $binaryDirectory
        Write-ExclusiveJson (Join-Path $armDirectory 'binaries-before.json') $before
        if ((ConvertTo-Json -InputObject $before -Depth 6 -Compress) -ne $baselineFingerprint) {
            $haltReason = 'Compiled inputs changed before an arm'; throw $haltReason
        }
        $startInfo = [Diagnostics.ProcessStartInfo]::new($executable)
        $startInfo.UseShellExecute = $false
        $startInfo.WorkingDirectory = $repoRoot
        $startInfo.RedirectStandardOutput = $true; $startInfo.RedirectStandardError = $true
        $startInfo.ArgumentList.Add('--clipboard-child'); $startInfo.ArgumentList.Add($armDirectory)
        $startInfo.Environment['IRIS_CLIPBOARD_TARGET_HEARTBEAT'] = 'on'
        $startInfo.Environment['IRIS_CLIPBOARD_TARGET_RENDERING'] = $mode
        $stdout = [IO.File]::Open((Join-Path $armDirectory 'stdout.log'), [IO.FileMode]::CreateNew)
        $stderr = [IO.File]::Open((Join-Path $armDirectory 'stderr.log'), [IO.FileMode]::CreateNew)
        $cancelCopies = [Threading.CancellationTokenSource]::new()
        $child = [Diagnostics.Process]::new(); $child.StartInfo = $startInfo
        $remainingMs = $totalLimitMs - $totalClock.ElapsedMilliseconds - 20000
        if ($remainingMs -le 0) { $haltReason = 'Total comparison deadline reached'; throw $haltReason }
        if (-not $child.Start()) { throw 'Owned child failed to start' }
        $result.started = $true; $result.processId = $child.Id
        $result.startedAt = [DateTime]::UtcNow.ToString('o')
        $copies = [Threading.Tasks.Task[]]@(
            $child.StandardOutput.BaseStream.CopyToAsync($stdout, 81920, $cancelCopies.Token),
            $child.StandardError.BaseStream.CopyToAsync($stderr, 81920, $cancelCopies.Token))
        $childBudget = [int][Math]::Min(180000, $remainingMs)
        if (-not $child.WaitForExit($childBudget)) {
            $result.timedOut = $true
            $cleanupAttempted = $true
            $child.Kill($true)
            if (-not $child.WaitForExit(10000)) { $haltReason = 'Owned child did not exit after timeout cleanup'; throw $haltReason }
        }
        $result.ownedProcessExited = $true; $result.exitCode = $child.ExitCode
        $result.logDrainComplete = [Threading.Tasks.Task]::WaitAll($copies, 5000)
        if (-not $result.logDrainComplete) { throw 'Owned child log streams did not finish' }
        # Read-only post-exit cleanup evidence, including assertion-failure arms.
        $namesPath = Join-Path $armDirectory 'generated-file-names.txt'
        if (Test-Path -LiteralPath $namesPath -PathType Leaf) {
            $names = [IO.File]::ReadAllLines($namesPath)
            $result.generatedFileCount = $names.Length
            $remainingCount = 0
            foreach ($name in $names) {
                if ([string]::IsNullOrWhiteSpace($name) -or [IO.Path]::GetFileName($name) -ne $name) { throw 'Invalid generated-file basename' }
                if ([IO.File]::Exists([IO.Path]::Combine([IO.Path]::GetTempPath(), $name))) { $remainingCount++ }
            }
            $result.generatedFileCleanupChecked = $true
            $result.generatedFileRemainingCount = $remainingCount
            $result.generatedFileCleanupVerified = $remainingCount -eq 0
        } else { $result.generatedFileCleanupError = 'Generated-file manifest unavailable; no cleanup inference' }
        $timingPath = Join-Path $armDirectory 'windows-immediate-edit-timings.json'
        $result.timingAvailable = Test-Path -LiteralPath $timingPath -PathType Leaf
        if ($result.timingAvailable) {
            $timing = Get-Content -LiteralPath $timingPath -Raw | ConvertFrom-Json
            $hasLifecycle = $null -ne $timing.first_render_ms -and $null -ne $timing.last_render_ms -and
                $null -ne $timing.rendering_subscription_ended_ms -and $timing.rendering_callback_count -ge 1 -and
                $timing.first_render_ms -le $timing.last_render_ms -and
                $timing.last_render_ms -le $timing.rendering_subscription_ended_ms -and
                $timing.rendering_subscription_ended_ms -le $timing.complete_paste_and_edit_ms
            $oneShotValid = $mode -ne 'one-shot' -or ($timing.rendering_callback_count -eq 1 -and
                $timing.first_render_ms -eq $timing.last_render_ms -and
                $timing.rendering_subscription_ended_ms -lt $timing.complete_paste_and_edit_ms)
            $result.targetModeVerified = $timing.diagnostic_child_pid -eq $child.Id -and
                $timing.target_heartbeat_enabled -eq $true -and $timing.heartbeat_metric_available -eq $true -and
                $null -ne $timing.longest_dispatcher_gap_ms -and $timing.target_rendering_mode -eq $mode -and
                $hasLifecycle -and $oneShotValid
        }
    } catch { $result.error = $_.Exception.Message }
    finally {
        if ($child -and $result.started -and -not $child.HasExited) {
            if (-not $cleanupAttempted) {
                try {
                    $cleanupAttempted = $true
                    $child.Kill($true)
                    $result.ownedProcessExited = $child.WaitForExit(10000)
                } catch { $result.error = "Owned child cleanup failed: $($_.Exception.Message)" }
            }
            if (-not $result.ownedProcessExited) { $haltReason = 'Owned child remains active; later arms not started' }
        }
        if ($cancelCopies) { $cancelCopies.Cancel() }
        if ($stdout) { $stdout.Dispose() }; if ($stderr) { $stderr.Dispose() }
        if ($cancelCopies) { $cancelCopies.Dispose() }; if ($child) { $child.Dispose() }
        try {
            $after = Get-BinaryManifest $binaryDirectory
            Write-ExclusiveJson (Join-Path $armDirectory 'binaries-after.json') $after
            $result.binariesUnchanged = (ConvertTo-Json -InputObject $after -Depth 6 -Compress) -eq $baselineFingerprint
            if (-not $result.binariesUnchanged) { $haltReason = 'Compiled inputs changed after an arm' }
        } catch { $result.error = "Binary provenance check failed: $($_.Exception.Message)"; $haltReason = $result.error }
        $result.completedAt = [DateTime]::UtcNow.ToString('o')
        Write-ExclusiveJson (Join-Path $armDirectory 'result.json') $result
        $results += $result
    }
}
$allPassed = @($results | Where-Object {
    -not $_.started -or $_.timedOut -or $_.exitCode -ne 0 -or $_.error -or -not $_.ownedProcessExited -or
    -not $_.logDrainComplete -or -not $_.binariesUnchanged -or -not $_.generatedFileCleanupVerified -or
    -not $_.timingAvailable -or -not $_.targetModeVerified
}).Count -eq 0
Write-ExclusiveJson (Join-Path $outputRoot 'comparison.json') ([ordered]@{
    schema = 1; source = $source; diagnosticOnly = $true; fullReleaseQualification = $false
    order = $modes; results = $results; allAssertionsPassed = $allPassed
    elapsedMs = $totalClock.ElapsedMilliseconds; totalDeadlineExceeded = $totalClock.ElapsedMilliseconds -gt $totalLimitMs
    decisionRule = $plan.decisionRule
})
if (-not $allPassed -or $totalClock.ElapsedMilliseconds -gt $totalLimitMs) { throw 'Diagnostic comparison retained failed or incomplete arms' }
