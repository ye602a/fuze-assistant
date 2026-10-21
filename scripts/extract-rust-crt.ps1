# Extract the pinned Microsoft redistributable without running its installer.
param([Parameter(Mandatory)][string] $Output)
$ErrorActionPreference = 'Stop'
$policy = Get-Content (Join-Path $PSScriptRoot '../rust/vc-runtime-policy.json') -Raw | ConvertFrom-Json
New-Item -ItemType Directory -Path $Output | Out-Null
$archive = Join-Path $Output 'VC_redist.x64.exe'
Invoke-WebRequest $policy.redistributable.url -OutFile $archive
if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $policy.redistributable.sha256) { throw 'redist-hash' }
$expanded = Join-Path $Output 'expanded'
New-Item -ItemType Directory -Path $expanded | Out-Null
# The EXE contains separate bootstrap and payload CABs; ordinary EXE extraction only sees the first.
$bytes = [IO.File]::ReadAllBytes($archive)
$text = [Text.Encoding]::ASCII.GetString($bytes)
$offset = 0
while (($offset = $text.IndexOf('MSCF', $offset, [StringComparison]::Ordinal)) -ge 0) {
    $size = [BitConverter]::ToUInt32($bytes, $offset + 8)
    if ($size -gt 36 -and $offset + $size -le $bytes.Length -and $bytes[$offset+24] -eq 3 -and $bytes[$offset+25] -eq 1) {
        $cab = Join-Path $Output "container-$offset.cab"
        $slice = New-Object byte[] $size
        [Array]::Copy($bytes, $offset, $slice, 0, $size)
        [IO.File]::WriteAllBytes($cab, $slice)
        & expand.exe '-F:*' $cab $expanded | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'redist-container-extract' }
    }
    $offset += 4
}
foreach ($file in @(Get-ChildItem $expanded -File)) {
    $head = [IO.File]::ReadAllBytes($file.FullName)
    if ($head.Length -ge 4 -and [Text.Encoding]::ASCII.GetString($head,0,4) -eq 'MSCF') {
        $folder = Join-Path $expanded ($file.Name + '-files')
        New-Item -ItemType Directory -Path $folder | Out-Null
        & expand.exe '-F:*' $file.FullName $folder | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'redist-cab-extract' }
    }
}
$dllDir = Join-Path $Output 'dlls'
New-Item -ItemType Directory -Path $dllDir | Out-Null
foreach ($file in Get-ChildItem $expanded -Recurse -File) {
    $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($file.FullName)
    $name = $info.OriginalFilename
    if ($name -and $name.ToLowerInvariant() -in @('vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll','msvcp140_1.dll','msvcp140_2.dll','msvcp140_atomic_wait.dll','concrt140.dll')) {
        $pe = [IO.File]::ReadAllBytes($file.FullName)
        if ([BitConverter]::ToUInt16($pe, [BitConverter]::ToInt32($pe,60)+4) -ne 0x8664) { continue }
        $version = [version]('{0}.{1}.{2}.{3}' -f $info.FileMajorPart,$info.FileMinorPart,$info.FileBuildPart,$info.FilePrivatePart)
        if ($version -lt [version]$policy.minimumVersion) { throw 'redist-version' }
        Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $dllDir $name.ToLowerInvariant())
    }
}
if (-not (Test-Path (Join-Path $dllDir 'vcruntime140.dll'))) { throw 'redist-dlls-not-found' }
Get-ChildItem $dllDir | Select-Object Name,Length
