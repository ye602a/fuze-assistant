# ZIP-only bootstrap. Dot sourcing loads pure/testable functions without executing main.
param([switch] $CheckOnly, [string] $RuntimeInstaller = '', [switch] $LaunchThisBuild)
$ErrorActionPreference = 'Stop'

function Test-VcVersion([string] $Actual, [string] $Minimum) {
    return $Actual -match '^14\.\d{1,5}\.\d{1,5}\.\d{1,5}$' -and
        $Minimum -match '^14\.\d{1,5}\.\d{1,5}\.\d{1,5}$' -and [version]$Actual -ge [version]$Minimum
}
function Get-VcArchitecture([string] $File) {
    $stream = [IO.File]::OpenRead($File)
    $reader = New-Object IO.BinaryReader($stream)
    try {
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d) { return 'unsupported' }
        $stream.Position = 0x3c
        $offset = $reader.ReadUInt32()
        if ($offset -gt $stream.Length - 26) { return 'unsupported' }
        $stream.Position = $offset
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) { return 'unsupported' }
        $stream.Position = $offset + 24
        if ($reader.ReadUInt16() -ne 0x20b) { return 'unsupported' }
        return 'x64'
    } finally { $reader.Dispose(); $stream.Dispose() }
}
function Read-VcContract([string] $Bundle) {
    $buildFile = Join-Path $Bundle 'build.json'
    $info = Get-Item -LiteralPath $buildFile
    if ($info.Length -gt 16384 -or ($info.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'runtime-build-invalid' }
    $build = Get-Content -LiteralPath $buildFile -Raw | ConvertFrom-Json
    # Legacy build.json remains readable; no current-machine version is substituted.
    if ($build.PSObject.Properties.Name -cnotcontains 'vcRuntime') { return $null }
    $c = $build.vcRuntime
    $dlls = @('vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll','msvcp140_1.dll','msvcp140_2.dll','msvcp140_atomic_wait.dll','concrt140.dll')
    $names = @('fuze-rust-host.exe','fuze-ocr-worker.exe','fuze-update.exe','onnxruntime.dll','DirectML.dll','dxcompiler.dll','dxil.dll')
    if ($build.uiFrontend -eq 'slint') { $names += 'fuze-ui-lab.exe' }
    if ($build.revision -notmatch '^[a-f0-9]{12,40}$' -or $build.uiFrontend -notin @('slint','win32') -or
        $c.schema -ne 1 -or $c.architecture -ne 'x64' -or $c.basis -ne 'supported-baseline-not-lowest-abi-version' -or
        -not (Test-VcVersion $c.minimumVersion $c.linkerVersion) -or $c.compilerVersion -notmatch '^19\.\d{1,5}\.\d{1,5}\.\d{1,5}$' -or
        -not (Test-VcVersion $c.minimumVersion ($c.compilerVersion -replace '^19\.', '14.')) -or
        $c.policySha256 -notmatch '^[a-f0-9]{64}$' -or $c.redistributable.version -ne $c.minimumVersion -or
        $c.redistributable.sha256 -notmatch '^[a-f0-9]{64}$' -or
        $c.redistributable.url -cnotmatch '^https://download\.visualstudio\.microsoft\.com/download/pr/[a-zA-Z0-9/-]+/VC_redist\.x64\.exe$' -or
        @($c.requiredDlls).Count -lt 1 -or @($c.requiredDlls).Count -gt $dlls.Count -or
        @($c.requiredDlls | Sort-Object -Unique).Count -ne @($c.requiredDlls).Count -or
        @($c.requiredDlls | Where-Object { $_ -cnotin $dlls }).Count -gt 0 -or
        @($c.binaries.PSObject.Properties).Count -ne $names.Count) { throw 'runtime-contract-invalid' }
    foreach ($name in $names) {
        $file = Join-Path $Bundle $name
        $item = Get-Item -LiteralPath $file
        if ($c.binaries.$name -notmatch '^[a-f0-9]{64}$' -or $item.Length -gt 128MB -or
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $c.binaries.$name -or
            (Get-VcArchitecture $file) -ne 'x64') { throw 'runtime-artifact-changed' }
    }
    return $c
}
function Get-VcInventory($Contract) {
    foreach ($name in $Contract.requiredDlls) {
        $file = Join-Path $PSScriptRoot $name
        if (-not (Test-Path -LiteralPath $file)) {
            $file = Join-Path ([Environment]::SystemDirectory) $name
        }
        $present = [IO.File]::Exists($file)
        $version = $null; $architecture = $null
        if ($present) {
            $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($file)
            $version = '{0}.{1}.{2}.{3}' -f $info.FileMajorPart,$info.FileMinorPart,$info.FileBuildPart,$info.FilePrivatePart
            $architecture = Get-VcArchitecture $file
        }
        [pscustomobject]@{name=$name;present=$present;version=$version;architecture=$architecture}
    }
}
function Get-VcDecision($Contract, $Inventory) {
    if (-not $Contract) { return [pscustomobject]@{state='unknown-build';minimumVersion=$null;problems=@()} }
    $problems = @()
    foreach ($name in $Contract.requiredDlls) {
        $items = @($Inventory | Where-Object { $_.name -ceq $name })
        if ($items.Count -ne 1 -or -not $items[0].present) { $problems += "missing:$name" }
        elseif ($items[0].architecture -ne 'x64') { $problems += "architecture:$name" }
        elseif (-not (Test-VcVersion $items[0].version $Contract.minimumVersion)) { $problems += "version:$name" }
    }
    $state = 'ready'
    if ($problems.Count) { $state = 'runtime-required' }
    return [pscustomobject]@{state=$state;minimumVersion=$Contract.minimumVersion;problems=$problems}
}
function Assert-VcInstaller([string] $File, $Contract) {
    $item = Get-Item -LiteralPath $File
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $item.Length -gt 64MB -or
        (Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash -ne $Contract.redistributable.sha256) { throw 'runtime-installer-hash' }
    $signature = Get-AuthenticodeSignature -LiteralPath $File
    if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate -or
        $signature.SignerCertificate.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName,$false) -ne 'Microsoft Corporation' -or
        [Diagnostics.FileVersionInfo]::GetVersionInfo($File).FileVersion -ne $Contract.redistributable.version) { throw 'runtime-installer-signature-or-version' }
}
# Effects are explicit arguments so tests can exercise the same sequencing without a process launch.
function Invoke-VcFlow($Contract, [scriptblock] $Inspect, [scriptblock] $ChooseInstaller,
    [scriptblock] $Confirm, [scriptblock] $ValidateInstaller, [scriptblock] $InstallRuntime,
    [scriptblock] $StartSetup, [switch] $InspectOnly) {
    $state = Get-VcDecision $Contract (& $Inspect)
    if ($InspectOnly -or $state.state -eq 'unknown-build') { return $state }
    if ($state.state -eq 'runtime-required') {
        $installer = & $ChooseInstaller $Contract
        if (-not $installer -or -not (& $Confirm)) { return [pscustomobject]@{state='cancelled'} }
        & $ValidateInstaller $installer $Contract
        $code = & $InstallRuntime $installer
        if ($code -in @(3010,1641)) { return [pscustomobject]@{state='restart-required';exitCode=$code} }
        if ($code -eq 1223) { return [pscustomobject]@{state='cancelled';exitCode=$code} }
        if ($code -notin @(0,1638)) { return [pscustomobject]@{state='runtime-install-failed';exitCode=$code} }
        $state = Get-VcDecision $Contract (& $Inspect)
        if ($state.state -ne 'ready') { return $state }
    }
    $code = & $StartSetup
    return [pscustomobject]@{state='setup-returned';exitCode=$code}
}
function Get-VcStartInfo([string] $Bundle, [switch] $Portable) {
    # Recheck payload after user interaction. This function prepares, but never launches, a process.
    if (-not (Read-VcContract $Bundle)) { throw 'runtime-build-unknown' }
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.WorkingDirectory = $Bundle
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    if ($Portable) {
        $build = Get-Content -LiteralPath (Join-Path $Bundle 'build.json') -Raw | ConvertFrom-Json
        $program = if ($build.uiFrontend -eq 'slint') { 'fuze-ui-lab.exe' } else { 'fuze-rust-host.exe' }
        $start.FileName = Join-Path $Bundle $program
        $start.Arguments = '--desktop preview.json'
    } else {
        $start.FileName = Join-Path $Bundle 'fuze-update.exe'
        $start.Arguments = '--install-local'
    }
    return $start
}
function Invoke-VcBootstrap {
    if (-not [Environment]::Is64BitProcess) { throw 'runtime-x64-powershell-required' }
    $bundle = $PSScriptRoot
    $contract = Read-VcContract $bundle
    $inspect = { @(Get-VcInventory $contract) }
    $choose = {
        param($c)
        Write-Host "FUZE needs Microsoft Visual C++ x64 $($c.minimumVersion) or newer."
        Write-Host 'Download the reviewed Microsoft installer, then enter its local path. No automatic download.'
        Write-Host $c.redistributable.url
        if ($RuntimeInstaller) { return $RuntimeInstaller }
        return (Read-Host 'Installer path, or Enter to cancel').Trim('"')
    }
    $confirm = { (Read-Host 'Install this Microsoft runtime with UAC? No automatic restart. Type INSTALL to continue') -ceq 'INSTALL' }
    $validate = { param($file,$c) Assert-VcInstaller $file $c }
    $install = {
        param($file)
        try {
            $process = Start-Process -FilePath ([IO.Path]::GetFullPath($file)) -ArgumentList '/install','/passive','/norestart' -Verb RunAs -Wait -PassThru -WindowStyle Hidden
            return $process.ExitCode
        } catch {
            $errorToCheck = $_.Exception
            for ($i=0; $i -lt 4 -and $errorToCheck; $i++) {
                if ($errorToCheck -is [ComponentModel.Win32Exception] -and $errorToCheck.NativeErrorCode -eq 1223) { return 1223 }
                $errorToCheck = $errorToCheck.InnerException
            }
            throw 'runtime-installer-launch-failed'
        }
    }
    $setup = {
        $start = Get-VcStartInfo $bundle -Portable:$LaunchThisBuild
        $process = [Diagnostics.Process]::Start($start)
        try {
            if ($LaunchThisBuild) { return 0 }
            $process.WaitForExit()
            return $process.ExitCode
        } finally { $process.Dispose() }
    }
    return Invoke-VcFlow $contract $inspect $choose $confirm $validate $install $setup -InspectOnly:$CheckOnly
}
if ($MyInvocation.InvocationName -ne '.') {
    try {
        $result = Invoke-VcBootstrap
        $result | ConvertTo-Json -Depth 5
        if (-not $CheckOnly) {
            if ($result.state -eq 'unknown-build') { Write-Host 'This older build has no verified runtime record. Use a newer original ZIP; no version was inferred.' }
            if ($result.state -eq 'restart-required') { Write-Host 'Windows restart is required. Restart when convenient, then run this original ZIP entry again.' }
            if (-not ($LaunchThisBuild -and $result.state -eq 'setup-returned' -and $result.exitCode -eq 0)) { $null = Read-Host 'Press Enter to close' }
        }
        if ($result.state -eq 'ready' -or ($result.state -eq 'setup-returned' -and $result.exitCode -eq 0)) { exit 0 }
        exit 2
    } catch {
        [Console]::Error.WriteLine('runtime-preflight-failed: verify the original ZIP and the Microsoft installer; FUZE setup or launch was not completed.')
        if (-not $CheckOnly) { $null = Read-Host 'Press Enter to close' }
        exit 1
    }
}
