param(
  [string]$Out = "D:\linux_project\zcode-usage-panel\output\px\app-printwindow.png",
  [string]$TitleMatch = "ZCode Usage"
)

Add-Type -AssemblyName System.Drawing

$sig = @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinCap {
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, uint nFlags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr hWnd);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc lpEnumFunc, IntPtr lParam);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
  public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
Add-Type -TypeDefinition $sig -ReferencedAssemblies System.Drawing

$target = Get-Process -Name "zcode-usage-panel" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $target) { Write-Output "app not running"; exit 1 }
Write-Output ("pid=" + $target.Id)

$found = @()
$cb = [WinCap+EnumProc]{
  param($h, $l)
  $pid2 = 0
  [WinCap]::GetWindowThreadProcessId($h, [ref]$pid2) | Out-Null
  if ($pid2 -eq $target.Id -and [WinCap]::IsWindowVisible($h)) {
    $len = [WinCap]::GetWindowTextLength($h)
    $sb = New-Object System.Text.StringBuilder ($len + 1)
    [WinCap]::GetWindowText($h, $sb, $sb.Capacity) | Out-Null
    $t = $sb.ToString()
    $r = New-Object WinCap+RECT
    [WinCap]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
    if ($w -gt 200 -and $hh -gt 200) {
      Write-Output ("  hwnd=" + $h + " title='" + $t + "' size=" + $w + "x" + $hh)
      $script:found += [pscustomobject]@{ H = $h; T = $t; W = $w; Hh = $hh }
    }
  }
  return $true
}
[WinCap]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null

$pick = $found | Where-Object { $_.T -match $TitleMatch } | Select-Object -First 1
if (-not $pick) { $pick = $found | Sort-Object -Property @{Expression={$_.W * $_.Hh}} -Descending | Select-Object -First 1 }
if (-not $pick) { Write-Output "no visible window found"; exit 1 }
Write-Output ("capturing hwnd=" + $pick.H + " title='" + $pick.T + "' " + $pick.W + "x" + $pick.Hh)

$bmp = New-Object System.Drawing.Bitmap($pick.W, $pick.Hh)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
$ok = [WinCap]::PrintWindow($pick.H, $hdc, 2)   # 2 = PW_RENDERFULLCONTENT
$g.ReleaseHdc($hdc)
$g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output ("printwindow=" + $ok + " saved=" + $Out)