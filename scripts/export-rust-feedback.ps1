param([string]$Source, [string]$Destination)
# Explicit post-game export. Reads only observation summaries and the three typed FUZE log files.
$ErrorActionPreference = 'Stop'
function SafeNumber($Value, [double]$Max, [bool]$Integer = $false) {
    if ($null -eq $Value -or $Value -is [string] -or $Value -is [bool]) { return $null }
    if ($Value -isnot [ValueType]) { return $null }
    $number = [double]$Value
    if ([double]::IsNaN($number) -or [double]::IsInfinity($number) -or $number -lt 0 -or $number -gt $Max) { return $null }
    if ($Integer -and [math]::Floor($number) -ne $number) { return $null }
    return $number
}
function ReadRuntimeLogs([string]$Directory) {
    $records = [Collections.Generic.List[object]]::new()
    $skipped = 0; $unavailable = 0; $found = $false
    $choices = @{
        connection=@('waiting','connected','paused','discovery','credentials','multiple','auth','not-found','http','phase','limit','transport')
        phase=@('unknown','lobby','selection','loading','playing','post-game')
        capture=@('waiting','paused','fresh','window-waiting','failed')
        ocr=@('waiting','paused','warming','cpu','directml','failed')
        issue=@('none','window-waiting','capture','window','ocr','pipeline')
        failure=@('none','other','locale-unavailable','champion-missing','lcu-context-missing','lcu-observation','pipeline-missing','window-waiting','capture-phase','capture-start','capture-missing','capture-frame','capture-atlas','worker-result','worker','overlay-reset','overlay-delivery')
        stream=@('waiting','waiting-frame','receiving','resizing','gpu-busy','suspended','recovering','stat-shards','retaining','unknown')
        gate=@('unknown','fallback','selection','pending','preparing','saving')
    }
    try {
        $absolute = [IO.Path]::GetFullPath($Directory)
        if ($absolute.StartsWith('\\')) { throw 'unsafe-log-directory' }
        $parent = [IO.DirectoryInfo]::new($absolute)
        while ($null -ne $parent) {
            if ($parent.Exists -and ($parent.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'unsafe-log-directory' }
            $parent = $parent.Parent
        }
        foreach ($name in @('runtime-v1.2.jsonl','runtime-v1.1.jsonl','runtime-v1.jsonl')) {
            $path = Join-Path $absolute $name
            if (!(Test-Path -LiteralPath $path)) { continue }
            $found = $true
            try {
                $item = Get-Item -LiteralPath $path
                if ($item.PSIsContainer -or $item.Length -gt 256KB -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'unsafe-log-file' }
                $stream = [IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
                try {
                    $buffer = [byte[]]::new(262145); $used = 0
                    while ($used -lt $buffer.Length) {
                        $n = $stream.Read($buffer,$used,$buffer.Length-$used)
                        if ($n -eq 0) { break }; $used += $n
                    }
                    if ($used -gt 262144) { throw 'log-limit' }
                    $text = [Text.Encoding]::UTF8.GetString($buffer,0,$used)
                } finally { $stream.Dispose() }
                foreach ($line in ($text -split "`n")) {
                    if ([string]::IsNullOrWhiteSpace($line)) { continue }
                    try {
                        if ($line.Length -gt 2048) { throw 'line-limit' }
                        $row = $line | ConvertFrom-Json
                        if ((SafeNumber $row.schema 1 $true) -ne 1 -or $row.event -isnot [string] -or $row.event -cnotin @('start','state','metrics','queue-overflow','stop')) { throw 'invalid-log' }
                        $elapsed = SafeNumber $row.elapsed_ms 1e12 $true
                        $dropped = SafeNumber $row.dropped 1e12 $true
                        if ($null -eq $elapsed -or $null -eq $dropped) { throw 'invalid-log' }
                        $safe = [ordered]@{}
                        foreach ($key in $choices.Keys) {
                            $value = $row.state.$key
                            if ($value -isnot [string] -or $value -cnotin $choices[$key]) { throw 'invalid-state' }
                            $safe[$key] = $value
                        }
                        foreach ($key in @('analyses','skipped')) {
                            $safe[$key] = SafeNumber $row.state.$key 1e12 $true
                            if ($null -eq $safe[$key]) { throw 'invalid-metric' }
                        }
                        $safe.analysis_ms = SafeNumber $row.state.analysis_ms 3600000
                        $safe.level = SafeNumber $row.state.level 30 $true
                        if ($safe.level -eq 0) { $safe.level = $null }
                        $safe.interval_ms = SafeNumber $row.state.interval_ms 3600000 $true
                        $safe.readable_slots = $null
                        if ($null -ne $row.state.readable_slots) {
                            $slots = @($row.state.readable_slots)
                            if ($slots.Count -ne 3 -or @($slots | Where-Object { $_ -isnot [bool] }).Count -ne 0) { throw 'invalid-slots' }
                            $safe.readable_slots = $slots
                        }
                        if ($row.state.overlay_prepared -isnot [bool]) { throw 'invalid-overlay' }
                        $safe.overlay_prepared = $row.state.overlay_prepared
                        $records.Add([ordered]@{schema=1;event=$row.event;elapsed_ms=$elapsed;state=$safe;dropped=$dropped})
                    } catch { $skipped++ }
                }
            } catch { $unavailable++ }
        }
    } catch { $unavailable++ }
    return [ordered]@{
        schema=1; status=$(if ($unavailable -gt 0 -or $skipped -gt 0) {'partial'} elseif ($found) {'ok'} else {'missing'})
        records=@($records.ToArray());skippedLines=$skipped;unavailableFiles=$unavailable
        scope='last-three-rotating-files-not-match-count';actualClaimRound=$null
    }
}
try {
    if (!$Source) { $Source = Join-Path $env:LOCALAPPDATA 'FuzeRustPreview/observations-v2.json' }
    if (!$Destination) { $Destination = Join-Path $PSScriptRoot ('Fuze-feedback-' + [guid]::NewGuid().ToString('N').Substring(0,8) + '.json') }
    $status = 'missing-observations'
    $summaries = @()
    try { if (Test-Path -LiteralPath $Source -PathType Leaf) {
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
    } } catch { $status = 'invalid-observations'; $summaries = @() }
    $logs = ReadRuntimeLogs (Join-Path ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Source))) 'logs')
    $revision = $null
    $buildPath = Join-Path $PSScriptRoot 'build.json'
    try { if (Test-Path -LiteralPath $buildPath -PathType Leaf) {
        if ((Get-Item -LiteralPath $buildPath).Length -le 16384) {
            $build = [IO.File]::ReadAllText($buildPath) | ConvertFrom-Json
            if ($build.revision -is [string] -and $build.revision -cmatch '^[a-f0-9]{12}$') { $revision = $build.revision }
        }
    } } catch { $revision = $null }
    $report = [ordered]@{
        schema=1; kind='fuze-private-safe-feedback'; packageRevision=$revision
        observationStatus=$status; scope='last-five-segments-not-match-count'; segments=$summaries
        recommendationScope='overall'; actualClaimRound=$null
        persistentLog=$logs
        limitations=@('observations-completed-analysis-only','bounded-rotating-state-log','not-popup-latency','no-cpu-gpu-measurement','no-claim-inference')
    }
    $bytes = [Text.Encoding]::UTF8.GetBytes(($report | ConvertTo-Json -Depth 8))
    $file = [IO.File]::Open([IO.Path]::GetFullPath($Destination), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $file.Write($bytes,0,$bytes.Length) } finally { $file.Dispose() }
    Write-Output ('Feedback saved. Observation status: ' + $status + '; segments: ' + $summaries.Count + '; log status: ' + $logs.status + '; log records: ' + $logs.records.Count + '. Attach the new Fuze-feedback JSON only.')
} catch {
    # Never print source content, paths, identities or exception messages.
    Write-Output 'Feedback export failed. No raw files were exported. Close FUZE and retry with a new output filename.'
    exit 1
}
