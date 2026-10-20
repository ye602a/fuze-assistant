param(
    [Parameter(Mandatory)] [string] $Bundle,
    [string] $Output = '',
    [string] $Version = '0.7.1',
    [string] $Compiler = "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe"
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$Bundle = (Resolve-Path -LiteralPath $Bundle).Path
if (-not $Output) { $Output = Join-Path $repo 'rust/artifacts/setup' }
$Output = [IO.Path]::GetFullPath($Output)
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'setup-version' }
& node (Join-Path $PSScriptRoot 'check-rust-setup-input.mjs') $Bundle
if ($LASTEXITCODE -ne 0) { throw 'setup-payload-not-allowlisted' }
. (Join-Path $repo 'scripts/install-rust-prerequisites.ps1')
$contract = Read-VcContract $Bundle
if (-not $contract) { throw 'setup-runtime-contract-missing' }
$build = Get-Content -LiteralPath (Join-Path $Bundle 'build.json') -Raw | ConvertFrom-Json
$head = (& git -C $repo rev-parse HEAD).Trim()
if ($build.revision -ne $head.Substring(0,12)) { throw 'setup-source-mismatch' }
$changes = & git -C $repo status --porcelain --untracked-files=all -- installer scripts rust
if ($LASTEXITCODE -ne 0 -or $changes) { throw 'setup-uncommitted-sources' }
New-Item -ItemType Directory -Path $Output -Force | Out-Null
$exe = Join-Path $Output "FUZE-Setup-$Version-windows-x64.exe"
if (Test-Path -LiteralPath $exe) { throw 'setup-output-exists' }
& $Compiler "/DPayload=$Bundle" "/DOutput=$Output" "/DVersion=$Version" (Join-Path $repo 'installer/rust-setup.iss')
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $exe)) { throw 'setup-build-failed' }
$hash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$exe.sha256", "$hash  $([IO.Path]::GetFileName($exe))`n")
$receipt = [ordered]@{
    schema=1; version=$Version; installerSource=$head; packageRevision=$build.revision
    installerSha256=$hash; compilerVersion=[Diagnostics.FileVersionInfo]::GetVersionInfo($Compiler).FileVersion
    signed=$false; minimumVcRuntime=$contract.minimumVersion
    nativeHashes=$contract.binaries
}
$receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Output 'installer-build.json') -Encoding utf8
