$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/../../scripts/rust-setup-bootstrap.ps1"
foreach ($case in @(
    @{state='setup-returned';exitCode=0;expected=0},
    @{state='setup-returned';exitCode=1;expected=5},
    @{state='cancelled';expected=2},
    @{state='runtime-required';expected=3},
    @{state='restart-required';exitCode=3010;expected=4},
    @{state='unknown-build';expected=5},
    @{state='runtime-install-failed';exitCode=1603;expected=5}
)) {
    if ((Get-SetupExitCode ([pscustomobject]$case)) -ne $case.expected) { throw "setup-exit-$($case.state)" }
}
# Parse all packaging scripts without invoking installation, downloads or UI.
foreach ($name in @('rust-setup-bootstrap.ps1','package-rust-setup.ps1','check-rust-setup.ps1')) {
    $tokens=$null; $errors=$null
    $null = [Management.Automation.Language.Parser]::ParseFile(
        "$PSScriptRoot/../../scripts/$name", [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw "setup-script-syntax-$name" }
}
Write-Output 'installer exit states and script syntax passed'
