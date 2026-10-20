# Compile only. No packaging, runtime installation, or FUZE process launch.
param(
    [string] $TargetDir = '',
    [string] $VcToolsDir = '',
    [string] $SdkLibDir = '',
    [string] $SdkIncludeDir = '',
    [string] $Cargo = 'cargo',
    [ValidateSet('slint','win32')] [string] $Frontend = 'slint',
    [switch] $Offline
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
if (-not $TargetDir) { $TargetDir = Join-Path $repo 'rust/target' }
if (-not $VcToolsDir) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $vs) { throw 'runtime-msvc-missing' }
    # Hosted runners may update their default toolchain ahead of our reviewed CRT baseline.
    # Select an installed, compatible compiler/linker pair; never relax the policy.
    $baseline = (Get-Content -LiteralPath (Join-Path $repo 'rust/vc-runtime-policy.json') -Raw | ConvertFrom-Json).minimumVersion
    $candidates = @(Get-ChildItem -LiteralPath (Join-Path $vs 'VC/Tools/MSVC') -Directory |
        Where-Object { $_.Name -match '^14\.\d+\.\d+$' } | Sort-Object { [version]$_.Name } -Descending)
    foreach ($candidate in $candidates) {
        $cl = Join-Path $candidate.FullName 'bin/Hostx64/x64/cl.exe'
        $ld = Join-Path $candidate.FullName 'bin/Hostx64/x64/link.exe'
        if (-not (Test-Path -LiteralPath $cl) -or -not (Test-Path -LiteralPath $ld)) { continue }
        $cv = [Diagnostics.FileVersionInfo]::GetVersionInfo($cl).FileVersion
        $lv = [Diagnostics.FileVersionInfo]::GetVersionInfo($ld).FileVersion
        if ($cv -match '^19\.\d+\.\d+\.\d+$' -and $lv -match '^14\.\d+\.\d+\.\d+$' -and
            [version]$baseline -ge [version]$lv -and [version]$baseline -ge [version]($cv -replace '^19\.', '14.')) {
            $VcToolsDir = $candidate.FullName
            break
        }
    }
    if (-not $VcToolsDir) { throw 'runtime-no-reviewed-installed-toolchain' }
}
if (-not $SdkLibDir -or -not $SdkIncludeDir) {
    $sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10'
    $versions = @(Get-ChildItem -LiteralPath (Join-Path $sdk 'Lib') -Directory | Where-Object { $_.Name -match '^10\.\d+\.\d+\.\d+$' } | Sort-Object { [version]$_.Name } -Descending)
    if (-not $versions.Count) { throw 'runtime-sdk-missing' }
    $SdkLibDir = $versions[0].FullName
    $SdkIncludeDir = Join-Path $sdk ('Include/' + $versions[0].Name)
}
$toolBin = Join-Path $VcToolsDir 'bin/Hostx64/x64'
$compiler = Join-Path $toolBin 'cl.exe'
$linker = Join-Path $toolBin 'link.exe'
$compilerVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($compiler).FileVersion
$linkerVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($linker).FileVersion
$policyFile = Join-Path $repo 'rust/vc-runtime-policy.json'
$policyHash = (Get-FileHash -LiteralPath $policyFile -Algorithm SHA256).Hash.ToLowerInvariant()
$policy = Get-Content -LiteralPath $policyFile -Raw | ConvertFrom-Json
if ($compilerVersion -notmatch '^19\.\d+\.\d+\.\d+$' -or $linkerVersion -notmatch '^14\.\d+\.\d+\.\d+$' -or
    [version]$policy.minimumVersion -lt [version]$linkerVersion -or
    [version]$policy.minimumVersion -lt [version]($compilerVersion -replace '^19\.', '14.')) { throw 'runtime-toolchain-newer-than-reviewed-baseline' }
$revision = (& git -C $repo rev-parse --short=12 HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $revision -notmatch '^[a-f0-9]{12}$') { throw 'runtime-build-identity' }
function Assert-CleanBuildSources {
    # Local handoff notes are unrelated; never stash/reset or silently include them.
    $changes = & git -C $repo status --porcelain --untracked-files=all -- rust scripts resources package.json package-lock.json .github tests
    if ($LASTEXITCODE -ne 0 -or $changes) { throw 'runtime-build-sources-uncommitted' }
}
Assert-CleanBuildSources
$env:CC = $compiler
$env:CXX = $compiler
$env:CC_x86_64_pc_windows_msvc = $compiler
$env:CXX_x86_64_pc_windows_msvc = $compiler
$env:TARGET_CC = $compiler
$env:TARGET_CXX = $compiler
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $linker
$env:LIB = (Join-Path $VcToolsDir 'lib/x64') + ';' + (Join-Path $SdkLibDir 'ucrt/x64') + ';' + (Join-Path $SdkLibDir 'um/x64')
$env:INCLUDE = (Join-Path $VcToolsDir 'include') + ';' + ((@('ucrt','shared','um','winrt') | ForEach-Object { Join-Path $SdkIncludeDir $_ }) -join ';')
$env:PATH = $toolBin + ';' + $env:PATH
$env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath($TargetDir)
# Different toolchain/policy identity cannot silently reuse a binary built with older settings.
$env:CARGO_ENCODED_RUSTFLAGS = '-C' + [char]31 + ('metadata=fuze-vc-' + $compilerVersion + '-' + $linkerVersion + '-' + $policyHash.Substring(0,16))
$receipt = Join-Path $env:CARGO_TARGET_DIR 'release/fuze-build-evidence.json'
if (Test-Path -LiteralPath $receipt) {
    # Invalidate this single generated receipt before any build; never clean the output tree.
    [IO.File]::WriteAllText($receipt, '{"schema":1,"completed":false}')
}
$common = @('+1.90.0','build','--release','--locked','-j','2')
if ($Offline) { $common += '--offline' }
& $Cargo @common --manifest-path (Join-Path $repo 'rust/Cargo.toml') --workspace
if ($LASTEXITCODE -ne 0) { throw 'runtime-core-build-failed' }
if ($Frontend -eq 'slint') {
    & $Cargo @common --manifest-path (Join-Path $repo 'rust/ui-lab/Cargo.toml') --features gpu
    if ($LASTEXITCODE -ne 0) { throw 'runtime-frontend-build-failed' }
}
if ((& git -C $repo rev-parse --short=12 HEAD).Trim() -ne $revision -or (Get-FileHash -LiteralPath $policyFile -Algorithm SHA256).Hash.ToLowerInvariant() -ne $policyHash) { throw 'runtime-build-source-changed' }
Assert-CleanBuildSources
$hashes = [ordered]@{}
$names = @('fuze-rust-host.exe','fuze-ocr-worker.exe','fuze-update.exe')
if ($Frontend -eq 'slint') { $names += 'fuze-ui-lab.exe' }
foreach ($name in $names) { $hashes[$name] = (Get-FileHash -LiteralPath (Join-Path $env:CARGO_TARGET_DIR "release/$name") -Algorithm SHA256).Hash.ToLowerInvariant() }
$record = [ordered]@{schema=1;completed=$true;sourceClean=$true;revision=$revision;frontend=$Frontend;compilerVersion=$compilerVersion;linkerVersion=$linkerVersion;policySha256=$policyHash;binaries=$hashes}
[IO.File]::WriteAllText($receipt, ($record | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false)))
$record | ConvertTo-Json -Depth 5
