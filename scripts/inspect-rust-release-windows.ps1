# Read only fixed package binaries and Windows runtime metadata. No certificate-store enumeration.
$ErrorActionPreference = 'Stop'
function Get-RuntimeArchitecture([string] $File) {
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
try {
    if (-not [Environment]::Is64BitProcess) { throw 'x64-inspector-required' }
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $names = @('fuze-rust-host.exe','fuze-ui-lab.exe','fuze-ocr-worker.exe','fuze-update.exe','onnxruntime.dll','DirectML.dll','dxcompiler.dll','dxil.dll')
    $binaries = foreach ($name in $names) {
        $file = Join-Path $request.bundle $(if ($name -eq 'fuze-ui-lab.exe' -and $request.desktopExecutable -eq 'FUZE.exe') { 'FUZE.exe' } else { $name })
        $signature = Get-AuthenticodeSignature -LiteralPath $file
        $signer = $null
        if ($signature.SignerCertificate) {
            $sha = [Security.Cryptography.SHA256]::Create()
            try { $signer = ([BitConverter]::ToString($sha.ComputeHash($signature.SignerCertificate.RawData))).Replace('-','').ToLowerInvariant() }
            finally { $sha.Dispose() }
        }
        [pscustomobject]@{ name=$name; sha256=(Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant(); signatureStatus=$signature.Status.ToString(); signerSha256=$signer; timestampPresent=($null -ne $signature.TimeStamperCertificate) }
    }
    $runtime = foreach ($name in @('vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll','msvcp140_1.dll','msvcp140_2.dll','msvcp140_atomic_wait.dll','concrt140.dll')) {
        $file = Join-Path ([Environment]::SystemDirectory) $name
        $present = [IO.File]::Exists($file)
        $version = $null
        $architecture = $null
        if ($present) {
            $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($file)
            $version = '{0}.{1}.{2}.{3}' -f $info.FileMajorPart,$info.FileMinorPart,$info.FileBuildPart,$info.FilePrivatePart
            $architecture = Get-RuntimeArchitecture $file
        }
        [pscustomobject]@{name=$name;present=$present;version=$version;architecture=$architecture}
    }
    [pscustomobject]@{x64Process=$true;binaries=@($binaries);runtime=@($runtime)} | ConvertTo-Json -Depth 5 -Compress
} catch {
    # Do not emit exception text, paths, certificate subject names, or user environment values.
    [Console]::Error.WriteLine('release-windows-inspection-failed')
    exit 1
}
