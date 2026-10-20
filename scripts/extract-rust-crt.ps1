# Extract the pinned Microsoft redistributable without running its installer.
param([Parameter(Mandatory)][string] $Output)
$ErrorActionPreference = 'Stop'
$policy = Get-Content (Join-Path $PSScriptRoot '../rust/vc-runtime-policy.json') -Raw | ConvertFrom-Json
New-Item -ItemType Directory -Path $Output | Out-Null
$archive = Join-Path $Output 'VC_redist.x64.exe'
Invoke-WebRequest $policy.redistributable.url -OutFile $archive
if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $policy.redistributable.sha256) { throw 'redist-hash' }
$sevenZip = Join-Path $env:ProgramFiles '7-Zip/7z.exe'
$expanded = Join-Path $Output 'expanded'
& $sevenZip x $archive "-o$expanded" -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'redist-extract' }
# Burn containers are CABs, often named a0/a1 rather than *.cab.
$containers = @(Get-ChildItem $expanded -Recurse -File | Where-Object {
    $stream = [IO.File]::OpenRead($_.FullName)
    try { $head = New-Object byte[] 4; $null = $stream.Read($head,0,4); [Text.Encoding]::ASCII.GetString($head) -eq 'MSCF' }
    finally { $stream.Dispose() }
})
$index = 0
foreach ($cab in $containers) {
    $folder = Join-Path $expanded ("cab-" + $index++)
    New-Item -ItemType Directory -Path $folder | Out-Null
    & $sevenZip x $cab.FullName "-o$folder" -y | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'redist-cab-extract' }
}
$dllDir = Join-Path $Output 'dlls'
New-Item -ItemType Directory -Path $dllDir | Out-Null
foreach ($file in Get-ChildItem $expanded -Recurse -File) {
    $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($file.FullName)
    $name = $info.OriginalFilename
    if ($name -and $name.ToLowerInvariant() -in @('vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll','msvcp140_1.dll','msvcp140_2.dll','msvcp140_atomic_wait.dll','concrt140.dll')) {
        $version = [version]('{0}.{1}.{2}.{3}' -f $info.FileMajorPart,$info.FileMinorPart,$info.FileBuildPart,$info.FilePrivatePart)
        if ($version -lt [version]$policy.minimumVersion) { throw 'redist-version' }
        Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $dllDir $name.ToLowerInvariant())
    }
}
if (-not (Test-Path (Join-Path $dllDir 'vcruntime140.dll'))) { throw 'redist-dlls-not-found' }
Get-ChildItem $dllDir | Select-Object Name,Length
