param([switch]$EditorOnly)
$ErrorActionPreference='Stop'
$exe=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\target\release\image-feedback-slint.exe')).Path
$old=$env:SLINT_BACKEND
try{
    # Only this new process and its editor children inherit the software-renderer request.
    $env:SLINT_BACKEND='winit-software'
    if($EditorOnly){$app=Start-Process -FilePath $exe -ArgumentList '--editor' -PassThru}
    else{$app=Start-Process -FilePath $exe -PassThru}
    Write-Output "Started low-memory prototype, process $($app.Id). Existing instances are not changed."
}finally{$env:SLINT_BACKEND=$old}
