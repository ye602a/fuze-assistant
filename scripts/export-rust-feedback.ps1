param([string]$Source, [string]$Destination)
# Explicit post-game export only. Never reads credentials, preferences, history or raw logs.
$ErrorActionPreference = 'Stop'
function SafeNumber($Value, [double]$Max, [bool]$Integer = $false) {
    if ($null -eq $Value -or $Value -is [string] -or $Value -is [bool]) { return $null }
    if ($Value -isnot [ValueType]) { return $null }
    $number = [double]$Value
    if ([double]::IsNaN($number) -or [double]::IsInfinity($number) -or $number -lt 0 -or $number -gt $Max) { return $null }
    if ($Integer -and [math]::Floor($number) -ne $number) { return $null }
    return $number
}
try {
    if (!$Source) { $Source = Join-Path $env:LOCALAPPDATA 'FuzeRustPreview/observations-v2.json' }
    if (!$Destination) { $Destination = Join-Path $PSScriptRoot ('Fuze-feedback-' + [guid]::NewGuid().ToString('N').Substring(0,8) + '.json') }
    $status = 'missing-observations'
    $summaries = @()
    if (Test-Path -LiteralPath $Source -PathType Leaf) {
        $item = Get-Item -LiteralPath $Source
        if ($item.Length -gt 2MB -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'invalid-input' }
        $saved = [IO.File]::ReadAllText($item.FullName) | ConvertFrom-Json
        if ($null -eq $saved.segments -or $saved.segments -isnot [array] -or $saved.segments.Count -gt 100) { throw 'invalid-input' }
        foreach ($segment in @($saved.segments | Select-Object -Last 5)) {
            $count = SafeNumber $segment.recognition.analyses 1000000 $true
            $partial = SafeNumber $segment.recognition.partial 1000000 $true
            $total = SafeNumber $segment.recognition.total_ms 1e12
            $ocr = SafeNumber $segment.recognition.ocr_ms 1e12
            $events = @()
            if (@($segment.events).Count -gt 80) { throw 'invalid-input' }
            foreach ($event in $segment.events) {
                $slots = @($event.cards)
                if ($slots.Count -ne 3) { throw 'invalid-input' }
                $present = @($slots | ForEach-Object { $n = SafeNumber $_ 4294967295 $true; $null -ne $n -and $n -gt 0 })
                $kind = 'unknown'
                if ($event.kind -is [string] -and $event.kind -cin @('first','changed','reappeared')) { $kind = $event.kind }
                $level = SafeNumber $event.level 30 $true
                if ($level -eq 0) { $level = $null }
                $events += [ordered]@{ kind=$kind; level=$level; readableSlots=$present }
            }
            $validMetrics = $null -ne $count -and $null -ne $partial -and $partial -le $count -and $null -ne $total -and $null -ne $ocr
            $meanTotal = $null; $meanOcr = $null
            if ($validMetrics -and $count -gt 0) {
                $meanTotal = [math]::Round($total / $count, 2)
                $meanOcr = [math]::Round($ocr / $count, 2)
            }
            $summaries += [ordered]@{
                segmentNumber=$summaries.Count + 1; interrupted=($segment.interrupted -ceq $true)
                capped=($segment.capped -ceq $true); metricsValid=$validMetrics
                analyses=$(if ($validMetrics) { $count } else { $null })
                partialAnalyses=$(if ($validMetrics) { $partial } else { $null })
                meanCompletedAnalysisMs=$meanTotal; meanCompletedOcrMs=$meanOcr; events=$events
            }
        }
        $status = 'available'
    }
    $revision = $null
    $buildPath = Join-Path $PSScriptRoot 'build.json'
    if (Test-Path -LiteralPath $buildPath -PathType Leaf) {
        if ((Get-Item -LiteralPath $buildPath).Length -le 16384) {
            $build = [IO.File]::ReadAllText($buildPath) | ConvertFrom-Json
            if ($build.revision -is [string] -and $build.revision -cmatch '^[a-f0-9]{12}$') { $revision = $build.revision }
        }
    }
    $report = [ordered]@{
        schema=1; kind='fuze-private-safe-feedback'; packageRevision=$revision
        observationStatus=$status; scope='last-five-segments-not-match-count'; segments=$summaries
        recommendationScope='overall'; actualClaimRound=$null
        limitations=@('completed-analysis-only','not-popup-latency','no-connection-or-capture-failure-log','no-cpu-gpu-measurement','no-claim-inference')
    }
    $bytes = [Text.Encoding]::UTF8.GetBytes(($report | ConvertTo-Json -Depth 8))
    $file = [IO.File]::Open([IO.Path]::GetFullPath($Destination), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $file.Write($bytes,0,$bytes.Length) } finally { $file.Dispose() }
    Write-Output ('Feedback saved. Observation status: ' + $status + '; segments: ' + $summaries.Count + '. Attach the new Fuze-feedback JSON only.')
} catch {
    # Never print source content, paths, identities or exception messages.
    Write-Output 'Feedback export failed. No raw files were exported. Close FUZE and retry with a new output filename.'
    exit 1
}
