param([string]$Iscc='', [string]$Readobj='', [string]$Python='python', [switch]$SkipBuild)
$ErrorActionPreference='Stop'
$root=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$target='x86_64-pc-windows-msvc'
if(-not $SkipBuild){
    $old=$env:RUSTFLAGS; $encoded=$env:CARGO_ENCODED_RUSTFLAGS; $embed=$env:SLINT_EMBED_RESOURCES
    try {
        $env:RUSTFLAGS='-C target-feature=+crt-static'; $env:CARGO_ENCODED_RUSTFLAGS=$null; $env:SLINT_EMBED_RESOURCES='true'
        cargo build --manifest-path "$root\Cargo.toml" --release --locked --target $target --target-dir "$root\target"
        if($LASTEXITCODE -ne 0){throw 'Static Windows build failed'}
    } finally {$env:RUSTFLAGS=$old;$env:CARGO_ENCODED_RUSTFLAGS=$encoded;$env:SLINT_EMBED_RESOURCES=$embed}
}
$meta=cargo metadata --manifest-path "$root\Cargo.toml" --no-deps --format-version 1 --locked | ConvertFrom-Json
if($LASTEXITCODE -ne 0){throw 'Cargo metadata failed'}
$version=($meta.packages | Where-Object name -eq 'image-feedback-slint').version
if(-not $version){throw 'Missing package version'}
$binary=Join-Path $root "target\$target\release\image-feedback-slint.exe"
if(-not(Test-Path -LiteralPath $binary)){throw 'Expected target binary missing'}
if(-not $Readobj){$command=Get-Command llvm-readobj -ErrorAction SilentlyContinue;if($command){$Readobj=$command.Source}else{$Readobj="$env:ProgramFiles\LLVM\bin\llvm-readobj.exe"}}
if(-not(Test-Path -LiteralPath $Readobj)){throw 'Install LLVM build tools or pass -Readobj. This is not an application runtime dependency.'}
$report=@(& $Readobj --file-headers --coff-imports $binary)
if($LASTEXITCODE -ne 0){throw 'PE import audit failed'}
if(-not($report -match 'COFF-x86-64')){throw 'Expected an x64 Windows PE'}
$dlls=@($report | ForEach-Object {if($_ -match '^\s*Name:\s*(\S+\.dll)\s*$'){$Matches[1].ToLowerInvariant()}} | Sort-Object -Unique)
if(-not $dlls.Count){throw 'No PE imports found; audit cannot pass'}
$system=('user32 shlwapi kernel32 kernelbase oleaut32 gdi32 ole32 comctl32 combase shell32 ntdll uxtheme dwrite opengl32 dwmapi bcryptprimitives bcrypt advapi32 imm32 ucrtbase msvcrt version crypt32 secur32 ws2_32 winmm comdlg32 shcore userenv propsys d3d11 dxgi d2d1') -split ' ' | ForEach-Object {"$_.dll"}
$unknown=@($dlls | Where-Object {$_ -notin $system -and $_ -notmatch '^(api|ext)-ms-win-'})
if($unknown.Count){throw "Non-system DLL imports need bundling/review: $($unknown -join ', ')"}
if($dlls -match '^(vcruntime|msvcp|msvcr\d|concrt|vcomp)'){throw 'Redistributable VC runtime is still dynamically imported'}
$dist=Join-Path $root 'dist\windows'; New-Item -ItemType Directory -Force -Path $dist | Out-Null
$stage=Join-Path $root ('release-staging\windows-'+[guid]::NewGuid().ToString('N'));New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'ImageFeedback.exe')
foreach($file in @('LICENSE','THIRD_PARTY_NOTICES.md','README.md','Cargo.lock')){Copy-Item -LiteralPath (Join-Path $root $file) -Destination $stage}
Copy-Item -LiteralPath (Join-Path $root 'licenses') -Destination $stage -Recurse
& $Python "$root\scripts\collect-licenses.py" --target $target --output "$stage\THIRD_PARTY_LICENSES.html"
if($LASTEXITCODE -ne 0){throw 'Dependency license collection failed'}
@{architecture='x86_64';static_crt=$true;imports=$dlls}|ConvertTo-Json -Depth 4|Set-Content -LiteralPath "$stage\runtime-dependencies.json" -Encoding utf8
if(-not $Iscc){foreach($candidate in @("$root\.tools\InnoSetup6\ISCC.exe","${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe","$env:ProgramFiles\Inno Setup 6\ISCC.exe")){if(Test-Path -LiteralPath $candidate){$Iscc=$candidate;break}}}
if(-not $Iscc -or -not(Test-Path -LiteralPath $Iscc)){throw 'Install Inno Setup 6 or pass -Iscc'}
& $Iscc "/DAppVersion=$version" "/DStageDir=$stage" "/DOutputDir=$dist" "$root\packaging\windows\ImageFeedback.iss"
if($LASTEXITCODE -ne 0){throw 'Installer compilation failed'}
$installer=Join-Path $dist "ImageFeedback-$version-windows-x64-setup.exe"
if(-not(Test-Path -LiteralPath $installer)){throw 'Installer output missing'}
$hash=(Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$installer.sha256", "$hash  $([IO.Path]::GetFileName($installer))`n", [Text.UTF8Encoding]::new($false))
Write-Output "Installer ready: $installer"
