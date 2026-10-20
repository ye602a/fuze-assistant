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
    if (-not $p.WaitForExit(180000)) { throw 'setup-ci-timeout' }
    $p.Refresh()
    if ($p.ExitCode -ne 0) { throw "setup-ci-exit-$($p.ExitCode)" }
}
Run-Checked $Installer @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART')
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
$report = [ordered]@{schema=1;revision=$build.revision;setupExe=$true;installed=$true;repeatInstall=$true;uninstalled=$true;userDataPreserved=$true;gameAccess=$false}
$report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path (Split-Path $Installer -Parent) 'installer-validation.json') -Encoding utf8
