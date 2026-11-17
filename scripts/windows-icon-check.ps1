# Checks the icon and version resources of OCW's Windows executables
# (ocw-winres, spec §11.3): extracts the exe's icon in several sizes as PNG
# files and prints the version info. Fails when an exe has no icon.
#
#   powershell -ExecutionPolicy Bypass -File scripts\windows-icon-check.ps1 `
#     -Exe target\debug\ocw-desktop.exe -Out test-output\windows-icon
param(
  [Parameter(Mandatory = $true)][string[]]$Exe,
  [string]$Out = "test-output\windows-icon"
)
$ErrorActionPreference = "Stop"
# -File passes "a.exe,b.exe" as one string.
$Exe = $Exe | ForEach-Object { $_ -split "," } | Where-Object { $_ }
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class OcwIcons {
  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  public static extern uint PrivateExtractIcons(string file, int index, int cx, int cy,
    IntPtr[] icons, uint[] ids, uint count, uint flags);
  [DllImport("user32.dll")] public static extern bool DestroyIcon(IntPtr icon);
  [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
  public static extern uint ExtractIconEx(string file, int index, IntPtr[] large, IntPtr[] small, uint count);
}
"@
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$failed = $false
foreach ($e in $Exe) {
  $path = (Resolve-Path $e).Path
  $name = [System.IO.Path]::GetFileNameWithoutExtension($path)
  $count = [OcwIcons]::ExtractIconEx($path, -1, $null, $null, 0)
  Write-Output "$name.exe: $count icon group(s)"
  if ($count -lt 1) { $failed = $true; continue }
  # The shell's associated icon (what Explorer and shortcuts show), 32 px.
  $assoc = [System.Drawing.Icon]::ExtractAssociatedIcon($path)
  $assoc.ToBitmap().Save((Join-Path $Out "$name-associated-32.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  foreach ($size in 16, 24, 32, 48, 64, 128, 256) {
    $h = New-Object IntPtr[] 1
    $ids = New-Object uint32[] 1
    $n = [OcwIcons]::PrivateExtractIcons($path, 0, $size, $size, $h, $ids, 1, 0)
    if ($n -lt 1 -or $h[0] -eq [IntPtr]::Zero) { Write-Output "  $size px: missing"; $failed = $true; continue }
    $icon = [System.Drawing.Icon]::FromHandle($h[0])
    $bmp = $icon.ToBitmap()
    $file = Join-Path $Out "$name-$size.png"
    $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Output ("  {0} px: {1}x{2} -> {3}" -f $size, $bmp.Width, $bmp.Height, $file)
    $bmp.Dispose(); $icon.Dispose(); [OcwIcons]::DestroyIcon($h[0]) | Out-Null
  }
  $v = (Get-Item $path).VersionInfo
  Write-Output ("  ProductName={0}; FileDescription={1}; FileVersion={2}; ProductVersion={3}; OriginalFilename={4}" -f `
    $v.ProductName, $v.FileDescription, $v.FileVersion, $v.ProductVersion, $v.OriginalFilename)
  if ($v.ProductName -ne "Open Cycling World") { $failed = $true }
}
if ($failed) { Write-Output "FAILED"; exit 1 }
Write-Output "OK"
