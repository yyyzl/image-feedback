param([string]$Out=(Join-Path $PSScriptRoot '..\artifacts\lifecycle-report.json'))
$ErrorActionPreference='Stop'
$exe=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\target\release\image-feedback-slint.exe')).Path
if(@(Get-Process -Name 'image-feedback-slint' -ErrorAction SilentlyContinue | Where-Object {$_.Path -eq $exe}).Count){throw 'Existing prototype instance found; it will not be altered'}
$tray=Start-Process -FilePath $exe -PassThru
try{
    Start-Sleep -Seconds 4
    $tray.Refresh();if($tray.HasExited){throw 'Tray process exited before creating an editor'}
    $children=@(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($tray.Id) AND Name = 'image-feedback-slint.exe'")
    if($children.Count -ne 1){throw "Expected exactly one editor child, found $($children.Count)"}
    $editor=Get-Process -Id $children[0].ProcessId
    if($editor.Path -ne $exe -or $children[0].CommandLine -notmatch '--editor'){throw 'Unexpected editor executable/command'}
    $before=[ordered]@{tray_pid=$tray.Id;editor_pid=$editor.Id;tray_working_set_bytes=$tray.WorkingSet64;editor_working_set_bytes=$editor.WorkingSet64}
    if(-not $editor.CloseMainWindow()){throw 'Could not request a normal editor close'}
    if(-not $editor.WaitForExit(5000)){throw 'Editor retained its window, possibly due to unsaved changes; it was not force-killed'}
    Start-Sleep -Seconds 1
    $tray.Refresh()
    $remaining=@(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($tray.Id) AND Name = 'image-feedback-slint.exe'")
    if($tray.HasExited -or $remaining.Count -ne 0){throw 'Expected only the surviving lightweight tray'}
    $report=[ordered]@{result='passed';sampled_at=(Get-Date).ToString('o');method='actual default launch -> native editor child -> normal WM_CLOSE -> child exited, tray survived';before=$before;after=[ordered]@{tray_alive=$true;editor_children=$remaining.Count;tray_working_set_bytes=$tray.WorkingSet64;tray_private_bytes=$tray.PrivateMemorySize64}}
    $report|ConvertTo-Json -Depth 6|Set-Content -LiteralPath $Out -Encoding utf8
    Write-Output ("Lifecycle passed: editor exited; tray WS={0:N1} MiB remains" -f ($tray.WorkingSet64/1MB))
}finally{
    $tray.Refresh()
    if(-not $tray.HasExited){
        $children=@(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($tray.Id) AND Name = 'image-feedback-slint.exe'")
        if($children.Count){throw 'An editor remains; preserving both processes rather than killing user data'}
        if($tray.Path -ne $exe){throw 'Tray executable identity changed; refusing cleanup'}
        Stop-Process -Id $tray.Id
        $tray.WaitForExit()
    }
}
