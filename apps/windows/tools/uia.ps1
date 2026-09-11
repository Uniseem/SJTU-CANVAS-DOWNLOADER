<#
.SYNOPSIS
  UI Automation helper for manual testing of the app window.

.DESCRIPTION
  Uses UI Automation patterns only (Invoke, SelectionItem, Toggle, Value), so
  the window is driven without moving the mouse or taking the focus.

.EXAMPLE
  ./uia.ps1 -Dump
  ./uia.ps1 -Click "下载"
  ./uia.ps1 -SetText "搜索课程" -Value "物理"
  ./uia.ps1 -Click "第 01 讲 · 课程导论" -Index 1
#>
param(
    [switch]$Dump,
    [string]$Click,
    [string]$SetText,
    [string]$Value,
    [int]$Index = 0,
    [int]$Depth = 40,
    [string]$ProcessName = "SJTUCanvasDownloader"
)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class AppWindows {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

    // The largest visible top-level window: the main window, not a tooltip.
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

$process = Get-Process -Name $ProcessName -ErrorAction Stop | Select-Object -First 1
$handle = [AppWindows]::Largest([uint32]$process.Id)
if ($handle -eq [IntPtr]::Zero) { throw "No window for $ProcessName" }
$root = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
$walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker

function Walk($element, $level) {
    if ($level -gt $Depth) { return }
    $child = $walker.GetFirstChild($element)
    while ($child -ne $null) {
        $current = $child.Current
        if (-not $current.IsOffscreen -and ($current.Name -or $current.AutomationId)) {
            $type = $current.ControlType.ProgrammaticName -replace 'ControlType\.', ''
            "{0}{1} '{2}' [{3}]" -f ('  ' * $level), $type, $current.Name, $current.AutomationId
        }
        Walk $child ($level + 1)
        $child = $walker.GetNextSibling($child)
    }
}

function Find-ByName($name) {
    $condition = New-Object System.Windows.Automation.PropertyCondition ([System.Windows.Automation.AutomationElement]::NameProperty), $name
    $matches = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition) |
        Where-Object { -not $_.Current.IsOffscreen })
    if ($matches.Count -le $Index) { return $null }
    $matches[$Index]
}

if ($Dump) { Walk $root 0 }

if ($Click) {
    $element = Find-ByName $Click
    if (-not $element) { throw "Element '$Click' not found" }
    # Text inside a button: walk up to the nearest invokable ancestor.
    $candidate = $element
    while ($candidate -and -not ($candidate.GetSupportedPatterns() | Where-Object { $_.ProgrammaticName -match 'Invoke|SelectionItem|Toggle|ExpandCollapse' })) {
        $candidate = $walker.GetParent($candidate)
    }
    if ($candidate) { $element = $candidate }
    $patterns = $element.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }
    if ($patterns -contains 'InvokePatternIdentifiers.Pattern') {
        $element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    } elseif ($patterns -contains 'TogglePatternIdentifiers.Pattern') {
        $element.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle()
    } elseif ($patterns -contains 'SelectionItemPatternIdentifiers.Pattern') {
        $item = $element.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)
        # In a multiple-selection list a click toggles the item.
        if ($item.Current.IsSelected) { $item.RemoveFromSelection() } else { $item.AddToSelection() }
    } elseif ($patterns -contains 'ExpandCollapsePatternIdentifiers.Pattern') {
        $element.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    } else {
        throw "Element '$Click' supports no clickable pattern: $($patterns -join ', ')"
    }
    "clicked '$Click'"
}

if ($SetText) {
    $element = Find-ByName $SetText
    if (-not $element) { throw "Element '$SetText' not found" }
    $element.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($Value)
    "set '$SetText'"
}
