<#
.SYNOPSIS
  Renders the app window (only that window) to a PNG for development.

.DESCRIPTION
  Uses PrintWindow with PW_RENDERFULLCONTENT: the window is rendered on its
  own, so other applications are never captured and the window does not need
  to be (or become) the foreground window. The largest visible top-level
  window of the process is used, so tooltips and popups are skipped.

.EXAMPLE
  ./capture-window.ps1 -Output shot.png
#>
param(
    [string]$ProcessName = "SJTUCanvasDownloader",
    [Parameter(Mandatory)] [string]$Output
)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class WindowCapture {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();

    public static IntPtr Largest(uint processId) {
        IntPtr best = IntPtr.Zero;
        long bestArea = 0;
        EnumWindows((hWnd, _) => {
            uint pid;
            GetWindowThreadProcessId(hWnd, out pid);
            RECT r;
            if (pid == processId && IsWindowVisible(hWnd) && GetWindowRect(hWnd, out r)) {
                long area = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
                if (area > bestArea) { bestArea = area; best = hWnd; }
            }
            return true;
        }, IntPtr.Zero);
        return best;
    }
}
"@
[WindowCapture]::SetProcessDPIAware() | Out-Null
$process = Get-Process -Name $ProcessName -ErrorAction Stop | Select-Object -First 1
$handle = [WindowCapture]::Largest([uint32]$process.Id)
if ($handle -eq [IntPtr]::Zero) { throw "No window for $ProcessName" }
$rect = New-Object WindowCapture+RECT
[WindowCapture]::GetWindowRect($handle, [ref]$rect) | Out-Null
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$hdc = $graphics.GetHdc()
[WindowCapture]::PrintWindow($handle, $hdc, 2) | Out-Null
$graphics.ReleaseHdc($hdc)
$bitmap.Save($Output, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$bitmap.Dispose()
"{0}x{1} -> {2}" -f $width, $height, $Output
