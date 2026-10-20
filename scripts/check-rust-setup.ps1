# Disposable GitHub runner only. Exercises the actual Setup.exe and native uninstall.
param([Parameter(Mandatory)] [string] $Bundle, [Parameter(Mandatory)] [string] $Installer)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'setup-ci-only' }
$root = Join-Path $env:LOCALAPPDATA 'FuzeRustPreview'
$managed = Join-Path $root 'releases'
$key = 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/FuzeRustPreview.Native'
if ((Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $key)) { throw 'setup-ci-existing-install' }
$build = Get-Content -LiteralPath (Join-Path $Bundle 'build.json') -Raw | ConvertFrom-Json
function Run-Checked([string] $File, [string[]] $Arguments) {
    $p = Start-Process -FilePath $File -ArgumentList $Arguments -WindowStyle Hidden -PassThru
    $null = $p.Handle
    if (-not $p.WaitForExit(180000)) { throw 'setup-ci-timeout' }
    $p.Refresh()
    if ($p.ExitCode -ne 0) { throw "setup-ci-exit-$($p.ExitCode)" }
}
# An explicitly restricted parent session must be honored, with no installation effects.
$oldPolicy = $env:PSExecutionPolicyPreference
try {
    $env:PSExecutionPolicyPreference = 'Restricted'
    $blocked = Start-Process -FilePath $Installer -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -WindowStyle Hidden -PassThru
    $null = $blocked.Handle
    if (-not $blocked.WaitForExit(60000)) { throw 'setup-ci-policy-timeout' }
    $blocked.Refresh()
    if ($blocked.ExitCode -eq 0 -or (Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $key)) { throw 'setup-ci-policy-overridden' }
} finally {
    $env:PSExecutionPolicyPreference = $oldPolicy
}
$setupLog = Join-Path (Split-Path $Installer -Parent) 'setup-ci.log'
try {
    Run-Checked $Installer @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',"/LOG=`"$setupLog`"")
} catch {
    # Synthetic CI only; emit fixed installer diagnostics, never the complete log or paths.
    if (Test-Path -LiteralPath $setupLog) {
        foreach ($line in [IO.File]::ReadLines($setupLog)) {
            if ($line -match 'FUZE (policy-check|bootstrap) exit=\d+') { Write-Host $Matches[0] }
            if ($line -match 'fuze-setup-[a-z-]+') { Write-Host $Matches[0] }
        }
    }
    throw
}
$pointer = Join-Path $managed 'current.json'
$state = Get-Content -LiteralPath $pointer -Raw | ConvertFrom-Json
if ($state.current.revision -ne $build.revision -or -not $state.pending) { throw 'setup-ci-pointer' }
if (-not (Test-Path -LiteralPath $key)) { throw 'setup-ci-registration' }
$programs = [Environment]::GetFolderPath('Programs')
if (-not (Test-Path -LiteralPath (Join-Path $programs 'Fuze Rust Preview.lnk'))) { throw 'setup-ci-shortcut' }
# Reinstall the same pending version repairs entries without duplicating/changing the pointer.
$before = [IO.File]::ReadAllText($pointer)
Run-Checked $Installer @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART')
if ([IO.File]::ReadAllText($pointer) -ne $before) { throw 'setup-ci-repeat-mutated-pointer' }
[IO.File]::WriteAllText((Join-Path $root 'user-note.fixture'), 'preserve-user-data')
[IO.File]::WriteAllText((Join-Path $managed 'unknown.fixture'), 'preserve-unknown-file')
# Run from the extracted source, outside the managed directory, as native uninstall requires.
Run-Checked (Join-Path $Bundle 'fuze-update.exe') @('--uninstall-local-confirmed')
if ((Test-Path -LiteralPath $key) -or (Test-Path -LiteralPath (Join-Path $programs 'Fuze Rust Preview.lnk'))) { throw 'setup-ci-uninstall-entries' }
if ([IO.File]::ReadAllText((Join-Path $root 'user-note.fixture')) -ne 'preserve-user-data' -or
    [IO.File]::ReadAllText((Join-Path $managed 'unknown.fixture')) -ne 'preserve-unknown-file') { throw 'setup-ci-user-data' }
if (Test-Path -LiteralPath (Join-Path $managed "versions/$($build.revision)/fuze-rust-host.exe")) { throw 'setup-ci-uninstall-files' }
$report = [ordered]@{schema=1;revision=$build.revision;setupExe=$true;explicitPolicyRespected=$true;installed=$true;repeatInstall=$true;uninstalled=$true;userDataPreserved=$true;gameAccess=$false}
$report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path (Split-Path $Installer -Parent) 'installer-validation.json') -Encoding utf8
