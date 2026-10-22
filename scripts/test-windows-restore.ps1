[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('8.0.420', '10.0.402')][string] $SdkVersion,
    [Parameter(Mandatory)][string] $OutputDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$PSNativeCommandUseErrorActionPreference = $false
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path $OutputDirectory) { throw 'Use a fresh evidence directory.' }
New-Item -ItemType Directory $OutputDirectory | Out-Null
. (Join-Path $PSScriptRoot 'windows-build-inputs.ps1')
$env:DOTNET_NOLOGO = 'true'
$env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'
$projectSource = Join-Path $Root 'windows\IrisChat'
$work = Join-Path $env:TEMP ('iris-restore-evaluation-' + [guid]::NewGuid())
New-Item -ItemType Directory $work | Out-Null
@{sdk = @{version = $SdkVersion; rollForward = 'disable'; allowPrerelease = $false}} |
    ConvertTo-Json -Depth 4 | Set-Content (Join-Path $work 'global.json')
$results = [Collections.Generic.List[object]]::new()
$passed = $false

function Invoke-RecordedDotnet([string] $Name, [string[]] $Arguments) {
    $log = Join-Path $OutputDirectory "$Name.log"
    & dotnet @Arguments *> $log
    $code = $LASTEXITCODE
    [pscustomobject]@{exitCode = $code; log = $log; arguments = $Arguments}
}

function New-Fixture([string] $Name, [bool] $ExplicitBase) {
    $directory = Join-Path $work $Name
    New-Item -ItemType Directory $directory | Out-Null
    foreach ($file in @('IrisChat.csproj', 'packages.lock.json', 'sdk-packages.json')) {
        Copy-Item (Join-Path $projectSource $file) (Join-Path $directory $file)
    }
    if ($ExplicitBase) {
        $path = Join-Path $directory 'IrisChat.csproj'
        $source = Get-Content -Raw $path
        $old = '<FrameworkReference Update="Microsoft.Windows.SDK.NET.Ref" RuntimeFrameworkVersion="$(WindowsSdkPackageVersion)" />'
        $replacement = '<FrameworkReference Remove="Microsoft.Windows.SDK.NET.Ref" />' + "`n    " +
            '<FrameworkReference Include="Microsoft.Windows.SDK.NET.Ref" RuntimeFrameworkVersion="$(WindowsSdkPackageVersion)" IsImplicitlyDefined="true" Pack="false" PrivateAssets="All" />'
        if (-not $source.Contains($old)) { throw 'The original Windows SDK override changed; review this diagnostic.' }
        $source.Replace($old, $replacement) | Set-Content $path
    }
    return $directory
}

function Restore-Fixture([string] $Name, [string] $Directory) {
    $project = Join-Path $Directory 'IrisChat.csproj'
    return Invoke-RecordedDotnet $Name @('restore', $project, '-r', 'win-x64', '--locked-mode', '--force',
        '-p:RestoreForceEvaluate=false', '-p:Configuration=Release', '--source', 'https://api.nuget.org/v3/index.json')
}

try {
    Push-Location $work
    $actualSdk = (& dotnet --version).Trim()
    if ($LASTEXITCODE -ne 0 -or $actualSdk -ne $SdkVersion) { throw "Selected SDK $actualSdk differs from $SdkVersion." }
    & dotnet --info *> (Join-Path $OutputDirectory 'dotnet-info.log')
    if ($LASTEXITCODE -ne 0) { throw 'Could not record the exact .NET toolchain.' }
    $inputs = @('IrisChat.csproj', 'packages.lock.json', 'sdk-packages.json') | ForEach-Object {
        $path = Join-Path $projectSource $_
        @{name = $_; sha256 = (Get-FileHash $path -Algorithm SHA256).Hash}
    }
    $inputs | ConvertTo-Json | Set-Content (Join-Path $OutputDirectory 'source-inputs.json')
    foreach ($name in @('original', 'explicit-base')) {
        $fixture = New-Fixture $name ($name -eq 'explicit-base')
        $project = Join-Path $fixture 'IrisChat.csproj'
        $evaluation = Invoke-RecordedDotnet "$name-evaluation" @('msbuild', $project, '-nologo',
            '-target:ProcessFrameworkReferences', '-p:RuntimeIdentifier=win-x64', '-p:Configuration=Release',
            '-getProperty:NETCoreSdkVersion,RuntimeFrameworkVersion,WindowsSdkPackageVersion,MSBuildVersion',
            '-getItem:FrameworkReference,KnownFrameworkReference,PackageDownload,RuntimePack')
        if ($evaluation.exitCode -ne 0) { throw "$name MSBuild evaluation failed; inspect its original output." }
        $restore = Restore-Fixture "$name-restore" $fixture
        $expectFailure = $name -eq 'original' -and $SdkVersion -eq '10.0.402'
        if ($expectFailure) {
            $log = Get-Content -Raw $restore.log
            if ($restore.exitCode -eq 0 -or $log -notmatch 'NU1102' -or
                $log -notmatch 'Microsoft.Windows.SDK.NET.Ref with version \(= 8\.0\.26\)') {
                throw 'The original .NET 10 failure was not reproduced; inspect the evaluated inputs.'
            }
        } else {
            if ($restore.exitCode -ne 0) { throw "$name locked restore failed; inspect its original output." }
            Assert-PinnedDownloadPackages (Join-Path $fixture 'obj\project.assets.json') (Join-Path $fixture 'sdk-packages.json')
        }
        $evidence = Join-Path $OutputDirectory $name
        New-Item -ItemType Directory $evidence | Out-Null
        Copy-Item $project $evidence
        if (Test-Path (Join-Path $fixture 'obj')) { Copy-Item (Join-Path $fixture 'obj') $evidence -Recurse }
        $results.Add(@{name = $name; evaluation = $evaluation; restore = $restore; expectedFailure = $expectFailure})
    }
    $fixed = Join-Path $work 'explicit-base'
    $project = Join-Path $fixed 'IrisChat.csproj'
    $unchanged = Get-Content -Raw $project
    $changed = $unchanged.Replace('Version="7.1.3"', 'Version="7.1.2"')
    if ($changed -eq $unchanged) { throw 'The dependency negative fixture no longer matches.' }
    $changed | Set-Content $project
    $negative = Restore-Fixture 'changed-dependency-negative' $fixed
    if ($negative.exitCode -eq 0 -or (Get-Content -Raw $negative.log) -notmatch 'NU1004') {
        throw 'Locked restore failed to reject a changed direct dependency.'
    }
    $results.Add(@{name = 'changed-dependency-negative'; restore = $negative; expectedFailure = $true})
    $unchanged | Set-Content $project
    $pins = Get-Content -Raw (Join-Path $fixed 'sdk-packages.json') | ConvertFrom-Json
    foreach ($pin in $pins) { $pin.sha512 = '0' * 128 }
    $badPins = Join-Path $work 'changed-sdk-hashes.json'
    $pins | ConvertTo-Json | Set-Content $badPins
    $rejected = $false
    try { Assert-PinnedDownloadPackages (Join-Path $OutputDirectory 'explicit-base\obj\project.assets.json') $badPins }
    catch { $rejected = $_.Exception.Message -like '*content hash mismatch*' }
    if (-not $rejected) { throw 'SDK package verification failed to reject changed content hashes.' }
    $results.Add(@{name = 'changed-sdk-hashes-negative'; expectedFailure = $true; rejected = $true})
    $passed = $true
} finally {
    Pop-Location
    @{passed = $passed; requestedSdk = $SdkVersion; results = @($results.ToArray());
      sourceCommit = $env:GITHUB_SHA; compilerOrAppExecuted = $false; fixtureDirectory = $work} |
        ConvertTo-Json -Depth 12 | Set-Content (Join-Path $OutputDirectory 'result.json')
}
