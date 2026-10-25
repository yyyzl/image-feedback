# Build-time tool bootstrap, never part of the application runtime.
$ErrorActionPreference='Stop'
$root=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$downloads=Join-Path $root '.tools\downloads';New-Item -ItemType Directory -Force -Path $downloads|Out-Null
$file=Join-Path $downloads 'innosetup-6.7.3.exe'
if(-not(Test-Path -LiteralPath $file)){
    Invoke-WebRequest -Uri 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe' -OutFile $file
}
if((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne '9C73C3BAE7ED48D44112A0F48E66742C00090BDB5BEF71D9D3C056C66E97B732'){throw 'Inno download hash mismatch'}
$sig=Get-AuthenticodeSignature -LiteralPath $file
if($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'Pyrsys B.V.'){throw 'Inno download publisher signature mismatch'}
$dest=Join-Path $root '.tools\InnoSetup6';$compiler=Join-Path $dest 'ISCC.exe'
if(-not(Test-Path -LiteralPath $compiler)){
    $p=Start-Process -FilePath $file -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/CURRENTUSER','/NOICONS',"/DIR=`"$dest`"") -Wait -PassThru
    if($p.ExitCode -ne 0){throw "Inno tool installation failed: $($p.ExitCode)"}
}
if(-not(Test-Path -LiteralPath $compiler)){throw 'Inno compiler missing'}
Write-Output $compiler
