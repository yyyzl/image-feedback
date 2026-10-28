param(
    [string]$Executable = "target/debug/image-feedback-slint.exe",
    [string]$Output = (Join-Path "artifacts" ("resident-quit-" + (Get-Date -Format "yyyyMMdd-HHmmss")))
)
$ErrorActionPreference = "Stop"
$exe = (Resolve-Path -LiteralPath $Executable).Path
if (Test-Path -LiteralPath $Output) { throw "Refusing to overwrite existing output: $Output" }
$root = (New-Item -ItemType Directory -Path $Output).FullName
$oldLocalAppData = $env:LOCALAPPDATA
$oldBackend = $env:SLINT_BACKEND
$env:SLINT_BACKEND = "winit-software"

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ResidentQuitSmokeKeys {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
}
'@

function Start-TestProcess([string[]]$Arguments, [string]$Directory, [string]$Name) {
    Start-Process -FilePath $exe -ArgumentList $Arguments -PassThru `
        -RedirectStandardOutput (Join-Path $Directory "$Name.stdout.log") `
        -RedirectStandardError (Join-Path $Directory "$Name.stderr.log")
}
function Wait-EditorWindow($Process, [int]$Seconds = 30) {
    $until = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $until) {
        $Process.Refresh()
        if ($Process.HasExited) { throw "Editor exited before the test was ready" }
        if ($Process.MainWindowHandle -ne 0 -and $Process.MainWindowTitle -like "*Image Feedback") { return }
        Start-Sleep -Milliseconds 300
    }
    throw "Timed out waiting for the editor window"
}
function Send-TestKey([IntPtr]$Window, [int]$Key) {
    if (-not [ResidentQuitSmokeKeys]::PostMessage($Window, 0x100, [IntPtr]$Key, [IntPtr]1)) {
        throw "Could not post key-down to the test window"
    }
    if (-not [ResidentQuitSmokeKeys]::PostMessage($Window, 0x101, [IntPtr]$Key, [IntPtr]0xC0000001)) {
        throw "Could not post key-up to the test window"
    }
}
function Wait-Success($Process, [string]$Report) {
    if (-not $Process.WaitForExit(30000)) { throw "Resident quit timed out" }
    $Process.Refresh()
    if ($Process.ExitCode -ne 0) { throw "Resident quit returned exit code $($Process.ExitCode)" }
    if (-not (Test-Path -LiteralPath (Join-Path $Report "quit-report.json"))) {
        throw "Resident quit did not produce a report"
    }
}
function Stop-TestProcess($Process) {
    if ($null -ne $Process) {
        $Process.Refresh()
        if (-not $Process.HasExited -and $Process.Path -eq $exe) {
            Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
        }
    }
}

try {
    foreach ($case in @("no-editor", "startup-race", "independent-editor", "owned-cancel")) {
        $directory = (New-Item -ItemType Directory -Path (Join-Path $root $case)).FullName
        $env:LOCALAPPDATA = Join-Path $directory "appdata"
        $report = Join-Path $directory "report"
        $tray = $null
        $editor = $null
        try {
            if ($case -eq "independent-editor") {
                $editor = Start-TestProcess @("--editor") $directory "editor"
                Wait-EditorWindow $editor
                $endpoint = Join-Path $env:LOCALAPPDATA "image-feedback-slint/editor-control.json"
                if (-not (Test-Path -LiteralPath $endpoint)) { throw "Editor endpoint was not created" }
                Start-Sleep -Seconds 2
            }
            $arguments = @("--tray-only", "--tray-quit-smoke", ('"' + $report + '"'))
            if ($case -eq "startup-race") { $arguments += "--quit-smoke-open-editor" }
            if ($case -eq "owned-cancel") {
                $arguments += @("--expect-quit-cancel", "--quit-smoke-open-annotated")
            }
            $tray = Start-TestProcess $arguments $directory "tray"
            if ($case -eq "owned-cancel") {
                $until = (Get-Date).AddSeconds(30)
                while ((Get-Date) -lt $until) {
                    $child = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($tray.Id)" |
                        Where-Object { $_.ExecutablePath -eq $exe -and $_.CommandLine -match "--editor" } |
                        Select-Object -First 1
                    if ($child) {
                        $editor = Get-Process -Id $child.ProcessId -ErrorAction SilentlyContinue
                        if ($editor) { break }
                    }
                    $tray.Refresh()
                    if ($tray.HasExited) { throw "Resident exited before starting its editor" }
                    Start-Sleep -Milliseconds 300
                }
                if (-not $editor) { throw "Could not find the test editor child" }
                Wait-EditorWindow $editor
                $window = $editor.MainWindowHandle
                # The smoke scenario sends the menu event five seconds after spawning the demo.
                Start-Sleep -Seconds 7
                Send-TestKey $window 27 # Escape: cancel the first exit confirmation.
                $until = (Get-Date).AddSeconds(10)
                while (-not (Test-Path -LiteralPath (Join-Path $report "cancelled.txt"))) {
                    if ((Get-Date) -gt $until) { throw "Exit cancellation was not observed" }
                    Start-Sleep -Milliseconds 200
                }
                $editor.Refresh()
                $tray.Refresh()
                if ($editor.HasExited -or $tray.HasExited) { throw "Cancel did not preserve both processes" }
                # The scenario retries after cancellation. Default focus is Cancel; Tab chooses Exit.
                Start-Sleep -Seconds 2
                Send-TestKey $window 9
                Start-Sleep -Milliseconds 250
                Send-TestKey $window 13
            }
            Wait-Success $tray $report
            if ($editor -and -not $editor.WaitForExit(10000)) { throw "Editor survived confirmed exit" }
            Write-Output "$case passed"
        } finally {
            # Only processes created by this isolated smoke scenario may be terminated on failure.
            if ($tray -and -not $tray.HasExited) {
                Get-CimInstance Win32_Process -Filter "ParentProcessId=$($tray.Id)" |
                    Where-Object { $_.ExecutablePath -eq $exe -and $_.CommandLine -match "--editor" } |
                    ForEach-Object { Stop-TestProcess (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue) }
            }
            Stop-TestProcess $editor
            Stop-TestProcess $tray
        }
    }
    Write-Output "Reports: $root"
} finally {
    $env:LOCALAPPDATA = $oldLocalAppData
    $env:SLINT_BACKEND = $oldBackend
}
