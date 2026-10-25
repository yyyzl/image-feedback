param([string]$Executable=(Join-Path $PSScriptRoot '..\target\release\image-feedback-slint.exe'),[string]$Out=(Join-Path $PSScriptRoot '..\artifacts\app-icon-from-exe.png'))
$ErrorActionPreference='Stop'
$exe=(Resolve-Path -LiteralPath $Executable).Path
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class IconResources {
 public delegate bool EnumName(IntPtr module, IntPtr type, IntPtr name, IntPtr param);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)]static extern IntPtr LoadLibraryExW(string path,IntPtr file,uint flags);
 [DllImport("kernel32.dll",SetLastError=true)]static extern bool FreeLibrary(IntPtr module);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)]static extern bool EnumResourceNamesW(IntPtr module,IntPtr type,EnumName callback,IntPtr param);
 public static int[] Counts(string path){
  IntPtr module=LoadLibraryExW(path,IntPtr.Zero,0x2|0x20);
  if(module==IntPtr.Zero)throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
  try{int images=0,groups=0;EnumName image=(m,t,n,p)=>{images++;return true;};EnumName group=(m,t,n,p)=>{groups++;return true;};EnumResourceNamesW(module,(IntPtr)3,image,IntPtr.Zero);EnumResourceNamesW(module,(IntPtr)14,group,IntPtr.Zero);return new[]{images,groups};}finally{FreeLibrary(module);}
 }
}
'@
$counts=[IconResources]::Counts($exe)
if($counts[0] -lt 9 -or $counts[1] -lt 1){throw 'Executable does not contain the expected multi-size application icon'}
$icon=[System.Drawing.Icon]::ExtractAssociatedIcon($exe)
if($null -eq $icon){throw 'The executable icon could not be extracted'}
$bitmap=$icon.ToBitmap()
try{$bitmap.Save($Out,[System.Drawing.Imaging.ImageFormat]::Png)}finally{$bitmap.Dispose();$icon.Dispose()}
Write-Output "Embedded icon verified: $($counts[0]) image resources, $($counts[1]) icon group; extracted to $Out"
