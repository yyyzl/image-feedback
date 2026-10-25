param([Parameter(Mandatory=$true)][string]$Installer,[string]$OutputDirectory='')
$ErrorActionPreference='Stop'
$installer=(Resolve-Path -LiteralPath $Installer).Path
$root=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if(-not $OutputDirectory){$OutputDirectory=Join-Path $root 'artifacts\windows-install-verification'}
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$output=(Resolve-Path -LiteralPath $OutputDirectory).Path
$key='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{A57E2C63-8619-4B9D-BC75-9B7DE154A621}_is1'
$appPath='HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\ImageFeedback.exe'
if(Test-Path -LiteralPath $key){throw 'An existing Image Feedback installation is registered; it was preserved'}
$install=Join-Path $output 'installed app'
if(Test-Path -LiteralPath $install){throw 'Test installation directory already exists; choose a fresh output directory'}
function Install-App {
    $p=Start-Process -FilePath $installer -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-',"/DIR=`"$install`"","/LOG=`"$output\install.log`"") -Wait -PassThru
    if($p.ExitCode -ne 0){throw "Installation failed: $($p.ExitCode)"}
}
Install-App
$exe=Join-Path $install 'ImageFeedback.exe'
foreach($path in @($exe,(Join-Path $install 'unins000.exe'),(Join-Path $install 'THIRD_PARTY_LICENSES.html'))){if(-not(Test-Path -LiteralPath $path)){throw "Installed file missing: $path"}}
if(-not(Test-Path -LiteralPath $key) -or -not(Test-Path -LiteralPath $appPath)){throw 'Installation registration missing'}
$shortcut=Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Image Feedback\Image Feedback.lnk'
if(-not(Test-Path -LiteralPath $shortcut)){throw 'Start Menu shortcut missing'}
$before=(Get-FileHash -LiteralPath $exe).Hash
# A running tray-only test instance contains no document and is safe to stop after the guard test.
$tray=Start-Process -FilePath $exe -ArgumentList '--tray-only' -WorkingDirectory $install -PassThru
try {
    Start-Sleep -Seconds 2
    if($tray.HasExited){throw 'Installed tray did not stay alive'}
    $blocked=Start-Process -FilePath $installer -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-',"/DIR=`"$install`"") -Wait -PassThru
    if($blocked.ExitCode -eq 0){throw 'Installer did not refuse a running application'}
    if($tray.HasExited -or (Get-FileHash -LiteralPath $exe).Hash -ne $before){throw 'Running application was terminated or replaced'}
} finally {
    if(-not $tray.HasExited){
        $children=@(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($tray.Id)")
        if($children.Count){throw 'Unexpected child; refusing to stop a potentially editable window'}
        Stop-Process -Id $tray.Id -ErrorAction Stop
        $tray.WaitForExit()
    }
}
# Repair/reinstall uses the same stable AppId and preserves the installation location.
Install-App
$oldBackend=$env:SLINT_BACKEND
try {
    $env:SLINT_BACKEND='winit-software'
    foreach($mode in @('smoke-test','delivery-smoke','dialog-smoke')){
        $dir=Join-Path $output $mode
        $p=Start-Process -FilePath $exe -ArgumentList @('--editor',"--$mode","`"$dir`"") -WorkingDirectory $install -Wait -PassThru
        if($p.ExitCode -ne 0){throw "Installed application test failed: $mode ($($p.ExitCode))"}
        $reportName=switch($mode){'smoke-test'{'smoke-report.json'} 'delivery-smoke'{'delivery-report.json'} 'dialog-smoke'{'dialog-report.json'}}
        $report=Get-Content -Raw -LiteralPath (Join-Path $dir $reportName)|ConvertFrom-Json
        if($report.result -ne 'passed'){throw "Missing completed report: $mode"}
    }
} finally {$env:SLINT_BACKEND=$oldBackend}
$data=Join-Path $env:LOCALAPPDATA 'image-feedback-slint';New-Item -ItemType Directory -Force -Path $data|Out-Null
$marker=Join-Path $data ('installer-preservation-'+[guid]::NewGuid().ToString('N')+'.txt')
Set-Content -LiteralPath $marker -Value 'owned install-test marker; preserve application data'
$markerAbsolute=(Get-Item -LiteralPath $marker).FullName
$markerHash=(Get-FileHash -LiteralPath $markerAbsolute).Hash
$uninstall=Join-Path $install 'unins000.exe'
$p=Start-Process -FilePath $uninstall -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',"/LOG=`"$output\uninstall.log`"") -Wait -PassThru
if($p.ExitCode -ne 0){throw "Uninstall failed: $($p.ExitCode)"}
if((Get-FileHash -LiteralPath $markerAbsolute).Hash -ne $markerHash){throw 'Uninstaller changed application data'}
if(Test-Path -LiteralPath $exe){throw 'Executable remains after uninstall'}
if(Test-Path -LiteralPath $key){throw 'Uninstall registration remains'}
if(Test-Path -LiteralPath $appPath){throw 'App Paths remains'}
if(Test-Path -LiteralPath $shortcut){throw 'Shortcut remains'}
if((Get-Item -LiteralPath $markerAbsolute).FullName -ne $markerAbsolute){throw 'Unexpected cleanup path'}
Remove-Item -LiteralPath $markerAbsolute
@{result='passed';installation='real per-user Inno installation';app_mutex='blocked replacement without killing the tray';repair='same-version reinstall';uninstall='program/shortcuts/registry removed; application data preserved';gui_renderer='software';limitations='Not a clean Windows VM test or a signed installer trust assessment'}|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $output 'installation-report.json') -Encoding utf8
Write-Output "Install/repair/guard/uninstall verification passed: $output"
