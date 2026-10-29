$ErrorActionPreference='Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class OSProbe {
 public delegate bool EnumCallback(IntPtr h, IntPtr p);
 [DllImport("user32.dll")] public static extern bool EnumWindows(EnumCallback callback, IntPtr value);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr process);
 [DllImport("user32.dll",EntryPoint="GetWindowThreadProcessId")] public static extern uint GetWindowPid(IntPtr h, out uint pid);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h,System.Text.StringBuilder name,int count);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,System.Text.StringBuilder name,int count);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out RECT rect);
 [StructLayout(LayoutKind.Sequential)] public struct RECT {public int left,top,right,bottom;}
 [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
 [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a,uint b,bool attach);
 [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int cx,int cy,uint flags);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref POINT p);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
 [DllImport("user32.dll")] public static extern bool IsChild(IntPtr parent, IntPtr child);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint dx,uint dy,uint data,UIntPtr extra);
 [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
 [DllImport("user32.dll")] public static extern bool GetCursorInfo(ref CURSORINFO info);
 [StructLayout(LayoutKind.Sequential)] public struct POINT {public int x,y;}
 [StructLayout(LayoutKind.Sequential)] public struct CURSORINFO {public int size,flags;public IntPtr handle; public POINT position;}
}
'@
[OSProbe]::SetProcessDPIAware()|Out-Null
$root=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path;Set-Location $root
if(-not (Test-Path 'target/debug/image-feedback-slint.exe')){throw 'Run cargo build first; this acceptance test uses the debug-only probe.'}
$dir=Join-Path $root 'artifacts/os-transform-probe';New-Item -Force -ItemType Directory $dir|Out-Null
Remove-Item "$dir/state.json","$dir/window.png","$dir/snapshot-request" -ErrorAction SilentlyContinue
$env:LOCALAPPDATA=Join-Path $root 'artifacts/os-transform-user';$env:SLINT_BACKEND='winit-software'
$previous=[OSProbe]::GetForegroundWindow(); $original=New-Object OSProbe+POINT;[OSProbe]::GetCursorPos([ref]$original)|Out-Null
$p=Start-Process -FilePath 'target/debug/image-feedback-slint.exe' -ArgumentList @('--editor','--demo','--native-pointer-probe',"`"$dir`"") -PassThru
function State { for($i=0;$i -lt 40;$i++){try { $s=Get-Content "$dir/state.json" -Raw|ConvertFrom-Json;if($s){return $s}}catch{};Start-Sleep -Milliseconds 100}; throw 'probe state unavailable' }
function RequireForeground {if([OSProbe]::GetForegroundWindow() -ne $script:h){throw 'test window is not foreground; refusing input'}}
function XY($s,[double]$x,[double]$y) { $o=New-Object OSProbe+POINT;[OSProbe]::ClientToScreen($script:h,[ref]$o)|Out-Null; return @{x=[int][math]::Round($o.x+($s.canvas_window_x+$s.view.x+$x*$s.view.scale)*$s.window_scale);y=[int][math]::Round($o.y+($s.canvas_window_y+$s.view.y+$y*$s.view.scale)*$s.window_scale)} }
function CheckPoint($pt){$p=New-Object OSProbe+POINT;$p.x=$pt.x;$p.y=$pt.y;$w=[OSProbe]::WindowFromPoint($p);if($w -ne $script:h -and -not [OSProbe]::IsChild($script:h,$w)){$pidFound=0;[OSProbe]::GetWindowPid($w,[ref]$pidFound)|Out-Null;if($pidFound -ne $script:p.Id){$class=[System.Text.StringBuilder]::new(256);[OSProbe]::GetClassName($w,$class,256)|Out-Null;throw "point is not inside test process: $($pt.x),$($pt.y), hwnd=$w pid=$pidFound class=$class expectedPid=$($script:p.Id)"}}}
function PointerMove($pt){RequireForeground;CheckPoint $pt;[OSProbe]::SetCursorPos($pt.x,$pt.y)|Out-Null;Start-Sleep -Milliseconds 220}
function Drag($a,$b){PointerMove $a;RequireForeground;[OSProbe]::mouse_event(2,0,0,0,[UIntPtr]::Zero); try {for($i=1;$i -le 12;$i++){RequireForeground;$pt=@{x=[int]($a.x+($b.x-$a.x)*$i/12);y=[int]($a.y+($b.y-$a.y)*$i/12)}; CheckPoint $pt;[OSProbe]::SetCursorPos($pt.x,$pt.y)|Out-Null;Start-Sleep -Milliseconds 25}}finally{[OSProbe]::mouse_event(4,0,0,0,[UIntPtr]::Zero)};Start-Sleep -Milliseconds 600}
function KeyDown([byte]$k){RequireForeground;[OSProbe]::keybd_event($k,0,0,[UIntPtr]::Zero)}
function KeyUp([byte]$k){[OSProbe]::keybd_event($k,0,2,[UIntPtr]::Zero)}
function Must($condition,$message){if(-not $condition){throw $message};Write-Output "PASS $message"}
try {
 $script:h=[IntPtr]::Zero
 for($i=0;$i -lt 100;$i++){Start-Sleep -Milliseconds 100;$p.Refresh();if($p.HasExited){throw "test process exited $($p.ExitCode)"};if($p.MainWindowHandle -ne 0){$script:h=$p.MainWindowHandle;break}}
 if($script:h -eq 0){throw 'test window missing'}
 # winit can recreate its native window during startup; choose the loaded editor,
 # not the temporary initial handle cached by Process.MainWindowHandle.
 $null=State; $script:editorHandle=[IntPtr]::Zero; $script:probePid=$p.Id
 $cb=[OSProbe+EnumCallback]{param($candidate,$unused) $candidatePid=0;[OSProbe]::GetWindowPid($candidate,[ref]$candidatePid)|Out-Null;if($candidatePid -eq $script:probePid -and [OSProbe]::IsWindowVisible($candidate)){$name=[System.Text.StringBuilder]::new(256);[OSProbe]::GetWindowText($candidate,$name,256)|Out-Null;if($name.ToString().Contains('首页设计.png')){$script:editorHandle=$candidate}};return $true}
 [OSProbe]::EnumWindows($cb,[IntPtr]::Zero)|Out-Null
 if($script:editorHandle -eq 0){throw 'loaded editor window missing'}
 $script:h=$script:editorHandle
 [OSProbe]::SetWindowPos($script:h,[IntPtr]::Zero,30,30,1920,1272,0x40)|Out-Null
 $fg=[OSProbe]::GetForegroundWindow();$fgThread=[OSProbe]::GetWindowThreadProcessId($fg,[IntPtr]::Zero);$ourThread=[OSProbe]::GetCurrentThreadId();
 if($fgThread -ne 0 -and $fgThread -ne $ourThread){[OSProbe]::AttachThreadInput($ourThread,$fgThread,$true)|Out-Null}
 try {[OSProbe]::BringWindowToTop($script:h)|Out-Null;[OSProbe]::SetForegroundWindow($script:h)|Out-Null}finally{if($fgThread -ne 0 -and $fgThread -ne $ourThread){[OSProbe]::AttachThreadInput($ourThread,$fgThread,$false)|Out-Null}}
 Start-Sleep -Seconds 3;RequireForeground
 $s=State;$t=$s.target;$source=$s.source; Write-Output "pid=$($p.Id) hwnd=$script:h state=$($s | ConvertTo-Json -Compress)"; Must ($s.rotation -eq 0) 'selected region starts unrotated'
 $center=XY $s ($t.x+$t.width/2) ($t.y+$t.height/2);PointerMove $center
 $ci=New-Object OSProbe+CURSORINFO;$ci.size=[System.Runtime.InteropServices.Marshal]::SizeOf($ci);[OSProbe]::GetCursorInfo([ref]$ci)|Out-Null;$defaultCursor=$ci.handle
 $corner=XY $s ($t.x+$t.width) ($t.y+$t.height);PointerMove $corner;$hover=State; $at=New-Object OSProbe+POINT;$at.x=$corner.x;$at.y=$corner.y;$wh=[OSProbe]::WindowFromPoint($at);$r=New-Object OSProbe+RECT;[OSProbe]::GetWindowRect($wh,[ref]$r)|Out-Null;$ti=[System.Text.StringBuilder]::new(256);[OSProbe]::GetWindowText($wh,$ti,256)|Out-Null; Write-Output "hover=$($hover.cursor) point=$($corner.x),$($corner.y) under=$wh title=$ti rect=$($r.left),$($r.top),$($r.right),$($r.bottom)"; Must (@(1,2) -contains $hover.cursor) 'actual OS mouse hovers resize handle'
 $end=XY $s ($t.x+$t.width*1.15) ($t.y+$t.height*1.15);Drag $corner $end
 $after=State;Must ($after.target.width -gt $t.width*1.05 -and [math]::Abs($after.rotation) -lt 1) 'actual OS corner drag resizes without rotation'
 $t=$after.target;$cx=$t.x+$t.width/2;$cy=$t.y+$t.height/2;$rx=$cx;$ry=$t.y-30/$after.view.scale
 $start=XY $after $rx $ry;PointerMove $start;$hover=State;Must ($hover.cursor -eq 3) 'actual OS mouse hovers top rotation control'
 $ci=New-Object OSProbe+CURSORINFO;$ci.size=[System.Runtime.InteropServices.Marshal]::SizeOf($ci);[OSProbe]::GetCursorInfo([ref]$ci)|Out-Null;Must ($ci.handle -ne [IntPtr]::Zero -and $ci.handle -ne $defaultCursor) 'OS actually switches to a distinct rotation cursor'
 $a=20*[math]::PI/180;$vx=$rx-$cx;$vy=$ry-$cy;$end=XY $after ($cx+[math]::Cos($a)*$vx-[math]::Sin($a)*$vy) ($cy+[math]::Sin($a)*$vx+[math]::Cos($a)*$vy);Drag $start $end
 $rot=State;Must ([math]::Abs($rot.rotation) -gt 12) 'actual OS drag rotates using top control'
 $t=$rot.target;$cx=$t.x+$t.width/2;$cy=$t.y+$t.height/2;$a=$rot.rotation*[math]::PI/180;$lx=$t.width/2+18/$rot.view.scale;$ly=-$t.height/2-18/$rot.view.scale;$rx=$cx+[math]::Cos($a)*$lx-[math]::Sin($a)*$ly;$ry=$cy+[math]::Sin($a)*$lx+[math]::Cos($a)*$ly
 $start=XY $rot $rx $ry;PointerMove $start;$hover=State;Must ($hover.cursor -eq 3) 'actual OS mouse detects corner-outside rotation'
 $a=10*[math]::PI/180;$vx=$rx-$cx;$vy=$ry-$cy;$end=XY $rot ($cx+[math]::Cos($a)*$vx-[math]::Sin($a)*$vy) ($cy+[math]::Sin($a)*$vx+[math]::Cos($a)*$vy);Drag $start $end
 $rot2=State;Must ([math]::Abs($rot2.rotation-$rot.rotation) -gt 6) 'actual OS drag rotates from corner exterior'
 # Hold Shift during an actual OS drag and verify angular snapping.
 $t=$rot2.target;$cx=$t.x+$t.width/2;$cy=$t.y+$t.height/2;$r=$rot2.rotation*[math]::PI/180;$distance=$t.height/2+30/$rot2.view.scale
 $rx=$cx+[math]::Sin($r)*$distance;$ry=$cy-[math]::Cos($r)*$distance;$start=XY $rot2 $rx $ry;$a=12*[math]::PI/180;$vx=$rx-$cx;$vy=$ry-$cy
 $end=XY $rot2 ($cx+[math]::Cos($a)*$vx-[math]::Sin($a)*$vy) ($cy+[math]::Sin($a)*$vx+[math]::Cos($a)*$vy)
 KeyDown 16;try{Drag $start $end}finally{KeyUp 16};$snapped=State
 Must ([math]::Abs($snapped.rotation/15-[math]::Round($snapped.rotation/15)) -lt 0.01 -and [math]::Abs($snapped.rotation-$rot2.rotation) -gt 5) 'actual OS Shift-drag snaps rotation'
 $t=$snapped.target;$center=XY $snapped ($t.x+$t.width/2) ($t.y+$t.height/2);$end=@{x=$center.x+50;y=$center.y+25};Drag $center $end
 $moved=State;Must ($moved.target.x -gt $t.x+10 -and [math]::Abs($moved.rotation-$snapped.rotation) -lt 0.1) 'actual OS drag moves rotated and scaled block'
 New-Item -ItemType File "$dir/snapshot-request"|Out-Null;for($i=0;$i -lt 50 -and -not (Test-Path "$dir/window.png");$i++){Start-Sleep -Milliseconds 100};Must (Test-Path "$dir/window.png") 'native window snapshot captured by test process'
 $t=$moved.target;$center=XY $moved ($t.x+$t.width/2) ($t.y+$t.height/2);KeyDown 32;try {Drag $center @{x=$center.x+45;y=$center.y+20}}finally{KeyUp 32};$pan=State;Must ([math]::Abs($pan.view.x-$moved.view.x) -gt 10 -and [math]::Abs($pan.target.x-$moved.target.x) -lt 0.01) 'actual OS space-drag pans without changing block'
 KeyDown 17;try{KeyDown 90;KeyUp 90}finally{KeyUp 17};Start-Sleep -Milliseconds 500;$undo=State;Must ([math]::Abs($undo.target.x-$moved.target.x) -gt 10 -and [math]::Abs($undo.rotation-$moved.rotation) -lt 0.1) 'actual OS Ctrl+Z undoes move'
 @{result='passed';method='Windows OS cursor positioning, mouse button and keyboard events; isolated debug app';checks=@('resize','top rotate','corner rotate','Shift snap','move after transform','space pan','undo');source_unchanged=($source.x -eq $undo.source.x -and $source.width -eq $undo.source.width)}|ConvertTo-Json|Set-Content "$dir/os-report.json"
} finally {
 if(-not $p.HasExited){Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue}
 [OSProbe]::SetCursorPos($original.x,$original.y)|Out-Null
 if($previous -ne [IntPtr]::Zero){[OSProbe]::SetForegroundWindow($previous)|Out-Null}
}
