# Offline fixtures only: no FUZE, installer, browser, registry writes, or elevation.
param([Parameter(Mandatory=$true)][string] $FixtureRoot)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../scripts/install-rust-prerequisites.ps1')
$script:checks = 0
function Assert($Condition, [string] $Name) { if (-not $Condition) { throw "fixture-failed:$Name" }; $script:checks++ }
$contract = [pscustomobject]@{minimumVersion='14.51.36247.0';requiredDlls=@('vcruntime140.dll')}
$script:inventory = @([pscustomobject]@{name='vcruntime140.dll';present=$true;architecture='x64';version='14.51.36247.0'})
Assert ((Get-VcDecision $null @()).state -eq 'unknown-build') 'legacy-unknown'
Assert ((Get-VcDecision $contract @()).state -eq 'runtime-required') 'missing'
Assert ((Get-VcDecision $contract $script:inventory).state -eq 'ready') 'equal'
$script:inventory[0].version='14.52.1.0'
Assert ((Get-VcDecision $contract $script:inventory).state -eq 'ready') 'newer'
$script:inventory[0].architecture='x86'
Assert ((Get-VcDecision $contract $script:inventory).state -eq 'runtime-required') 'x86'
$script:inventory[0].architecture='x64'; $script:inventory[0].version='14.50.35719.0'
Assert ((Get-VcDecision $contract $script:inventory).state -eq 'runtime-required') 'old'
Assert (-not (Test-VcVersion 'garbage' $contract.minimumVersion)) 'invalid-version'
Assert (-not (Test-VcVersion '14.9.999.0' '14.51.1.0')) 'numeric-version'
$inspect = { $script:inventory }
$forbidden = { throw 'unexpected-effect' }
$script:setupCount=0
$setup = { $script:setupCount++; 0 }
$choose = { 'fixture-only.exe' }
$confirm = { $true }
$validate = { }
Assert ((Invoke-VcFlow $contract $inspect $forbidden $forbidden $forbidden $forbidden $forbidden -InspectOnly).state -eq 'runtime-required') 'check-only'
Assert ((Invoke-VcFlow $null $inspect $forbidden $forbidden $forbidden $forbidden $forbidden).state -eq 'unknown-build') 'legacy-no-launch'
Assert ((Invoke-VcFlow $contract $inspect {''} $forbidden $forbidden $forbidden $forbidden).state -eq 'cancelled') 'no-installer'
Assert ((Invoke-VcFlow $contract $inspect $choose {$false} $forbidden $forbidden $forbidden).state -eq 'cancelled') 'declined'
foreach ($code in @(3010,1641,1223,1603)) {
    $install = { $code }.GetNewClosure()
    $result = Invoke-VcFlow $contract $inspect $choose $confirm $validate $install $forbidden
    Assert ($result.exitCode -eq $code) "exit-$code"
    Assert ($result.state -eq $(if($code -in @(3010,1641)) {'restart-required'} elseif($code -eq 1223) {'cancelled'} else {'runtime-install-failed'})) "state-$code"
}
Assert ((Invoke-VcFlow $contract $inspect $choose $confirm $validate {0} $forbidden).state -eq 'runtime-required') 'success-must-recheck'
foreach ($code in @(0,1638)) {
    $script:inventory[0].version='14.50.35719.0'
    $install = { $script:inventory[0].version='14.52.1.0'; $code }.GetNewClosure()
    Assert ((Invoke-VcFlow $contract $inspect $choose $confirm $validate $install $setup).state -eq 'setup-returned') "postcheck-$code"
}
Assert ((Invoke-VcFlow $contract $inspect $forbidden $forbidden $forbidden $forbidden $setup).state -eq 'setup-returned') 'ready-skip-installer'
Assert ($script:setupCount -eq 3) 'exact-setup-count'

New-Item -ItemType Directory -Path $FixtureRoot | Out-Null
$build = @{revision='123456789abc';uiFrontend='slint';runtime='1.26.0'}
$buildFile=Join-Path $FixtureRoot 'build.json'
$build | ConvertTo-Json | Set-Content -LiteralPath $buildFile -Encoding UTF8
Assert ($null -eq (Read-VcContract $FixtureRoot)) 'legacy-file'
$c = @{schema=1;architecture='x64';minimumVersion='14.51.36247.0';basis='supported-baseline-not-lowest-abi-version';compilerVersion='19.44.35229.0';linkerVersion='14.44.35229.0';policySha256=('a'*64);requiredDlls=@('vcruntime140.dll');redistributable=@{version='14.51.36247.0';sha256=('b'*64);url='https://download.visualstudio.microsoft.com/download/pr/abc/VC_redist.x64.exe'};binaries=@{}}
$bytes = New-Object byte[] 256
[BitConverter]::GetBytes([uint16]0x5a4d).CopyTo($bytes,0)
[BitConverter]::GetBytes([uint32]128).CopyTo($bytes,0x3c)
[BitConverter]::GetBytes([uint32]0x4550).CopyTo($bytes,128)
[BitConverter]::GetBytes([uint16]0x8664).CopyTo($bytes,132)
[BitConverter]::GetBytes([uint16]0x20b).CopyTo($bytes,152)
foreach ($name in @('fuze-rust-host.exe','fuze-ui-lab.exe','fuze-ocr-worker.exe','fuze-update.exe','onnxruntime.dll','DirectML.dll','dxcompiler.dll','dxil.dll')) {
    $file=Join-Path $FixtureRoot $name
    [IO.File]::WriteAllBytes($file,$bytes)
    $c.binaries[$name]=(Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
}
$build.vcRuntime=$c
$build | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $buildFile -Encoding UTF8
$loaded=Read-VcContract $FixtureRoot
Assert ($loaded.minimumVersion -eq $c.minimumVersion) 'recorded-contract'
$start = Get-VcStartInfo $FixtureRoot -Portable
Assert ($start.FileName -eq (Join-Path $FixtureRoot 'fuze-ui-lab.exe') -and $start.Arguments -eq '--desktop preview.json') 'portable-this-slint-build'
Assert ($start.WorkingDirectory -eq $FixtureRoot -and -not $start.UseShellExecute -and $start.CreateNoWindow) 'portable-path-with-spaces'
$start = Get-VcStartInfo $FixtureRoot
Assert ($start.FileName -eq (Join-Path $FixtureRoot 'fuze-update.exe') -and $start.Arguments -eq '--install-local') 'install-unchanged'
$failed=$false
try { Assert-VcInstaller (Join-Path $FixtureRoot 'fuze-update.exe') $loaded } catch { $failed=$true }
Assert $failed 'installer-hash-rejected'
$c.requiredDlls=@('../private.dll')
$build | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $buildFile -Encoding UTF8
$failed=$false
try { $null=Read-VcContract $FixtureRoot } catch { $failed=$true }
Assert $failed 'no-path-dependency'
$c.requiredDlls=@('vcruntime140.dll'); $c.binaries['fuze-update.exe']='c'*64
$build | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $buildFile -Encoding UTF8
$failed=$false
try { $null=Read-VcContract $FixtureRoot } catch { $failed=$true }
Assert $failed 'changed-payload'
$failed=$false
try { $null=Get-VcStartInfo $FixtureRoot -Portable } catch { $failed=$true }
Assert $failed 'portable-rechecks-payload-before-start'
$c.binaries['fuze-update.exe']=(Get-FileHash -LiteralPath (Join-Path $FixtureRoot 'fuze-update.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
$build | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $buildFile -Encoding UTF8
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../../scripts/install-rust-prerequisites.ps1') -Destination (Join-Path $FixtureRoot 'Install-local.ps1')
[pscustomobject]@{checks=$script:checks;passed=$true;appLaunched=$false;runtimeInstalled=$false} | ConvertTo-Json -Compress
