[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Root = (Get-Location).Path
$Output = Join-Path $Root 'work/nearby-layout'
$Core = 'bc698fe9cbd2058b247a82c56c04a791cb34a0ff'
$Baseline = '4b1bb0cea11c69d65fa50fb84eef371471bb845e'
$Fixed = '0d6a24ebc1b00da132d3187674f39a62aa3121ad'
$Dll = Join-Path $Root 'windows/IrisChat/Frameworks/iris_chat_core.dll'
$Project = 'windows/NearbyAvatarTests/NearbyAvatarTests.csproj'
$Result = [ordered]@{ core = $Core; baseline = $Baseline; fixed = $Fixed; phases = @(); qualified = $false }

function Git-Value([string[]] $Arguments) {
    $value = & git @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Git command failed: $Arguments" }
    return ($value -join "`n").Trim()
}

function Save-Result {
    $Result | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $Output 'result.json')
}

function Invoke-Dotnet([string[]] $Arguments, [string] $Stem, [int] $TimeoutSeconds) {
    $process = [Diagnostics.Process]::new()
    $process.StartInfo.FileName = (Get-Command dotnet).Source
    $process.StartInfo.WorkingDirectory = $Root
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $process.StartInfo.ArgumentList.Add($argument) }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    if (-not $process.Start()) { throw 'Could not start dotnet' }
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $timedOut = -not $process.WaitForExit($TimeoutSeconds * 1000)
    if ($timedOut) { $process.Kill($true); $process.WaitForExit() }
    $stdout.GetAwaiter().GetResult() | Set-Content "$Stem.stdout.log"
    $stderr.GetAwaiter().GetResult() | Set-Content "$Stem.stderr.log"
    $run = [ordered]@{ arguments = $Arguments; exitCode = $process.ExitCode; timedOut = $timedOut; elapsedSeconds = $watch.Elapsed.TotalSeconds }
    $process.Dispose()
    return $run
}

try {
    if ((Git-Value @('rev-parse', 'HEAD')) -ne $Core) { throw 'Core was not built at the pinned source' }
    $Result.coreTree = Git-Value @('rev-parse', "${Core}:core")
    $Result.nativeDllSHA256 = (Get-FileHash $Dll -Algorithm SHA256).Hash.ToLowerInvariant()
    $Result.bindings = @(Get-ChildItem windows/IrisChat/Bindings -File -Recurse | ForEach-Object {
        @{ path = [IO.Path]::GetRelativePath($Root, $_.FullName); sha256 = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    if ($Result.bindings.Count -eq 0) { throw 'Matching generated bindings are missing' }
    $testPath = 'windows/NearbyAvatarTests/Program.cs'
    if ((Git-Value @('rev-parse', "${Baseline}:$testPath")) -ne (Git-Value @('rev-parse', "${Fixed}:$testPath"))) {
        throw 'Baseline and fixed regression tests differ'
    }
    $productionPath = 'windows/IrisChat/Chrome/DesktopShell.xaml.cs'
    if ((Git-Value @('rev-parse', "${Baseline}:$productionPath")) -ne (Git-Value @('rev-parse', "${Core}:$productionPath"))) {
        throw 'Baseline production implementation differs from the retained release source'
    }
    foreach ($phase in @(@{ name = 'baseline'; source = $Baseline }, @{ name = 'fixed'; source = $Fixed })) {
        Git-Value @('checkout', '--detach', $phase.source) | Out-Null
        if ((Git-Value @('rev-parse', 'HEAD:core')) -ne $Result.coreTree) { throw 'Native core source changed' }
        if ((Get-FileHash $Dll -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Result.nativeDllSHA256) { throw 'Native DLL changed' }
        foreach ($binding in $Result.bindings) {
            if ((Get-FileHash (Join-Path $Root $binding.path) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $binding.sha256) { throw 'Native bindings changed' }
        }
        $directory = Join-Path $Output $phase.name
        New-Item -ItemType Directory -Path $directory | Out-Null
        $record = [ordered]@{ phase = $phase.name; source = $phase.source; qualified = $false }
        $Result.phases += $record
        $record.build = Invoke-Dotnet @('build', $Project, '-c', 'Release') (Join-Path $directory 'build') 600
        Save-Result
        if ($record.build.timedOut -or $record.build.exitCode -ne 0) { throw "$($phase.name) managed build failed; no retry" }
        $targetPath = & dotnet msbuild $Project -getProperty:TargetPath -p:Configuration=Release
        if ($LASTEXITCODE -ne 0) { throw 'Managed output discovery failed' }
        $targetPath = ($targetPath -join "`n").Trim()
        $record.managedAssemblySHA256 = (Get-FileHash $targetPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $loadedDll = Join-Path (Split-Path -Parent $targetPath) 'iris_chat_core.dll'
        $record.loadedNativeDllSHA256 = (Get-FileHash $loadedDll -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($record.loadedNativeDllSHA256 -ne $Result.nativeDllSHA256) { throw 'Test output uses a different native DLL' }
        $record.test = Invoke-Dotnet @('run', '--no-build', '--project', $Project, '-c', 'Release', '--', $directory, '--layout-only') (Join-Path $directory 'test') 120
        $testOutput = (Get-Content (Join-Path $directory 'test.stdout.log') -Raw) + (Get-Content (Join-Path $directory 'test.stderr.log') -Raw)
        if ($record.test.timedOut) { throw "$($phase.name) test timed out; no retry" }
        if ($phase.name -eq 'baseline') {
            $record.qualified = $record.test.exitCode -ne 0 -and $testOutput.Contains('Nearby slot height changed with 1 peers')
        } else {
            $record.qualified = $record.test.exitCode -eq 0 -and $testOutput.Contains('PASS: Nearby row geometry for 0, 1, 8, 0 peers')
            $record.screenshots = @(Get-ChildItem $directory -Filter 'windows-nearby-layout-*.png' | ForEach-Object { $_.Name })
            $record.qualified = $record.qualified -and $record.screenshots.Count -eq 4
        }
        Save-Result
        if (-not $record.qualified) { throw "$($phase.name) did not produce the required geometry result; no retry" }
    }
    $Result.qualified = $true
} catch {
    $Result.error = $_.Exception.Message
    throw
} finally {
    Save-Result
}
