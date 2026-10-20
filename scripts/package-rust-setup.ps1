# Repackage the immutable public v0.7.0 ZIP. Never compiles or launches FUZE.
param(
    [Parameter(Mandatory)] [string] $Archive,
    [Parameter(Mandatory)] [string] $Output,
    [string] $Compiler = "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe"
)
$ErrorActionPreference = 'Stop'
$expectedHash = '35ab61daad95f128310ab77b111ea9db4284264f8e930bb8d120faa2c110e892'
if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHash) { throw 'original-v070-zip-mismatch' }
$Output = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $Output) { throw 'use-new-isolated-output' }
New-Item -ItemType Directory -Path $Output | Out-Null
$payload = Join-Path $Output 'payload'
Expand-Archive -LiteralPath $Archive -DestinationPath $payload
$build = Get-Content -LiteralPath (Join-Path $payload 'build.json') -Raw | ConvertFrom-Json
if ($build.revision -ne 'dc0957fd7617') { throw 'original-v070-revision-mismatch' }
$repo = Split-Path $PSScriptRoot -Parent
& $Compiler "/DPayload=$payload" "/DOutput=$Output" (Join-Path $repo 'installer/rust-setup.iss')
if ($LASTEXITCODE -ne 0) { throw 'setup-compile-failed' }
$exe = Join-Path $Output 'FUZE-Setup-0.7.0-windows-x64.exe'
$hash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
if ((Get-AuthenticodeSignature -LiteralPath $exe).Status -ne 'NotSigned') { throw 'unexpected-signature-state' }
[IO.File]::WriteAllText("$exe.sha256", "$hash  FUZE-Setup-0.7.0-windows-x64.exe`n")
@{
    version='0.7.0'; packageRevision=$build.revision
    installerSource=(& git -C $repo rev-parse HEAD).Trim()
    sourceZipSha256=$expectedHash; installerSha256=$hash; signed=$false
    applicationRebuilt=$false; applicationLaunched=$false; installerExecuted=$false
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Output 'setup-build.json') -Encoding utf8
