param([ValidateRange(2,30)][int]$Seconds=5,[string]$Out=(Join-Path $PSScriptRoot '..\artifacts\runtime-metrics.json'))
$ErrorActionPreference='Stop'
$exe=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\target\release\image-feedback-slint.exe')).Path
$existing=@(Get-Process -Name 'image-feedback-slint' -ErrorAction SilentlyContinue | Where-Object {$_.Path -eq $exe})
if($existing.Count -gt 0){throw 'Close existing prototype processes before measuring. No existing process will be stopped.'}
$logical=[Environment]::ProcessorCount
$rows=@()
$cases=@(@{name='tray-only';argv=@('--tray-only')},@{name='blank-editor';argv=@('--editor')},@{name='demo-editor';argv=@('--editor','--demo')})
$large=Join-Path $PSScriptRoot '..\artifacts\perf-large.png'
if(Test-Path -LiteralPath $large){$cases+=@{name='large-image-editor';argv=@('--editor','--open',('"'+(Resolve-Path -LiteralPath $large).Path+'"'))}}
foreach($case in $cases){
    $watch=[Diagnostics.Stopwatch]::StartNew()
    $process=Start-Process -FilePath $exe -ArgumentList $case.argv -PassThru
    try{
        $inputIdle=$false
        try{$inputIdle=$process.WaitForInputIdle(5000)}catch{}
        $inputIdleMs=$watch.Elapsed.TotalMilliseconds
        # This is a defined measurement interval, not waiting on a harness background job.
        Start-Sleep -Seconds 4
        $process.Refresh();if($process.HasExited){throw "Prototype exited during $($case.name)"}
        if($case.name -ne 'tray-only' -and $process.MainWindowHandle -eq [IntPtr]::Zero){throw 'Expected editor window was not created'}
        $cpuStart=$process.TotalProcessorTime.TotalSeconds
        $sample=[Diagnostics.Stopwatch]::StartNew()
        Start-Sleep -Seconds $Seconds
        $process.Refresh();if($process.HasExited){throw "Prototype exited during idle sample $($case.name)"}
        $cpu=$process.TotalProcessorTime.TotalSeconds-$cpuStart
        $elapsed=$sample.Elapsed.TotalSeconds
        $children=@(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($process.Id)" | Select-Object ProcessId,Name)
        $row=[ordered]@{state=$case.name;pid=$process.Id;sample_seconds=$elapsed;idle_cpu_seconds=$cpu;idle_cpu_percent_one_core=100*$cpu/$elapsed;idle_cpu_percent_normalized=100*$cpu/$elapsed/$logical;working_set_bytes=$process.WorkingSet64;private_bytes=$process.PrivateMemorySize64;peak_working_set_bytes=$process.PeakWorkingSet64;threads=$process.Threads.Count;input_idle_returned=$inputIdle;input_idle_ms=$inputIdleMs;children=$children}
        $rows+=$row
        Write-Output ("{0}: WS={1:N1} MiB private={2:N1} MiB CPU={3:N3}s/{4}s" -f $case.name,($process.WorkingSet64/1MB),($process.PrivateMemorySize64/1MB),$cpu,$Seconds)
    }finally{
        $process.Refresh()
        if(-not $process.HasExited){
            if($process.Path -ne $exe){throw 'Executable identity changed; refusing cleanup'}
            if($case.name -eq 'tray-only'){
                # This owned test instance never opens an editor and contains no user document.
                Stop-Process -Id $process.Id -ErrorAction Stop
                $process.WaitForExit()
            }else{
                if(-not $process.CloseMainWindow()){throw "Could not request a normal close for test editor $($process.Id)"}
                if(-not $process.WaitForExit(5000)){throw "Test editor $($process.Id) kept open (possibly unsaved changes); it was NOT force-killed"}
            }
        }
    }
}
$backendRequest=if([string]::IsNullOrWhiteSpace($env:SLINT_BACKEND)){'default'}else{$env:SLINT_BACKEND}
$report=[ordered]@{sampled_at=(Get-Date).ToString('o');os=[Environment]::OSVersion.VersionString;logical_processors=$logical;executable=$exe;executable_bytes=(Get-Item -LiteralPath $exe).Length;sha256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash;renderer_request=$backendRequest;gpu_memory_measured=$false;scope='single Windows host observations, not cross-framework benchmarks or power measurements';states=$rows}
$report|ConvertTo-Json -Depth 8|Set-Content -LiteralPath $Out -Encoding utf8
Write-Output "Saved runtime metrics: $Out"
