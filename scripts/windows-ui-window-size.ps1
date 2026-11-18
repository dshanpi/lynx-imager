param([Parameter(Mandatory=$true)][int]$Width,[Parameter(Mandatory=$true)][int]$Height,[Parameter(Mandatory=$true)][string]$Executable)
$ErrorActionPreference='Stop'
if ($Width -lt 1090 -or $Height -lt 720 -or $Width -gt 3840 -or $Height -gt 2160) {throw 'Unexpected isolated test window size'}
function Get-OwnedHash([string]$File) { $stream=[System.IO.File]::OpenRead($File);try { $sha=[System.Security.Cryptography.SHA256]::Create();try { return [System.BitConverter]::ToString($sha.ComputeHash($stream)) } finally {$sha.Dispose()} } finally {$stream.Dispose()} }
$expected=Get-OwnedHash $Executable
$candidates=@(Get-Process -Name lynx-imager -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle -eq 'LYNX isolated UI acceptance' -and $_.MainWindowHandle -ne 0 })
if ($candidates.Count -ne 1) {throw 'Expected one isolated test application window'}
$owned=$candidates[0]
if ((Get-OwnedHash $owned.Path) -ne $expected) {throw 'Isolated test executable mismatch'}
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class LynxTestWindow {
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window,out RECT rect);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window,out RECT rect);
 [DllImport("user32.dll",SetLastError=true)] public static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
}
"@
$oldContext=[LynxTestWindow]::SetThreadDpiAwarenessContext([IntPtr](-4))
try {
 $outer=New-Object LynxTestWindow+RECT;$client=New-Object LynxTestWindow+RECT
 if (-not [LynxTestWindow]::GetWindowRect($owned.MainWindowHandle,[ref]$outer)) {throw 'Cannot read owned window geometry'}
 if (-not [LynxTestWindow]::GetClientRect($owned.MainWindowHandle,[ref]$client)) {throw 'Cannot read owned client geometry'}
 $scale=[LynxTestWindow]::GetDpiForWindow($owned.MainWindowHandle)/96.0
 $physicalWidth=[int][Math]::Round($Width*$scale)+($outer.Right-$outer.Left)-($client.Right-$client.Left)
 $physicalHeight=[int][Math]::Round($Height*$scale)+($outer.Bottom-$outer.Top)-($client.Bottom-$client.Top)
 # Normal window API; preserve activation and Z-order. No synthetic input/focus.
 if (-not [LynxTestWindow]::SetWindowPos($owned.MainWindowHandle,[IntPtr]::Zero,0,0,$physicalWidth,$physicalHeight,0x0014)) {throw 'Owned window resize failed'}
 [pscustomobject]@{pid=$owned.Id;scale=$scale;logicalWidth=$Width;logicalHeight=$Height;physicalWidth=$physicalWidth;physicalHeight=$physicalHeight}|ConvertTo-Json -Compress
} finally { [void][LynxTestWindow]::SetThreadDpiAwarenessContext($oldContext) }
