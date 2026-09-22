$sig = @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class EnumW {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EP f, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr h);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  public delegate bool EP(IntPtr h, IntPtr l);
}
"@
Add-Type -TypeDefinition $sig

$procs = Get-Process -Name "zcode-usage-panel" -ErrorAction SilentlyContinue
if (-not $procs) { Write-Output "app not running"; exit 1 }
$pids = @($procs | ForEach-Object { $_.Id })
Write-Output ("pids: " + ($pids -join ","))

$rows = New-Object System.Collections.ArrayList
$cb = [EnumW+EP]{
  param($h, $l)
  $wp = 0
  [EnumW]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
  if ($pids -contains [int]$wp) {
    $tl = [EnumW]::GetWindowTextLength($h)
    $sb = New-Object System.Text.StringBuilder ($tl + 2)
    [EnumW]::GetWindowText($h, $sb, $sb.Capacity) | Out-Null
    $cn = New-Object System.Text.StringBuilder 256
    [EnumW]::GetClassName($h, $cn, $cn.Capacity) | Out-Null
    $r = New-Object EnumW+RECT
    [EnumW]::GetWindowRect($h, [ref]$r) | Out-Null
    $vis = [EnumW]::IsWindowVisible($h)
    $par = [EnumW]::GetParent($h)
    [void]$rows.Add([pscustomobject]@{
      Hwnd = $h; Vis = $vis; Top = ($par -eq [IntPtr]::Zero)
      W = ($r.Right - $r.Left); Hh = ($r.Bottom - $r.Top)
      Cls = $cn.ToString(); Title = $sb.ToString()
    })
  }
  return $true
}
[EnumW]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null

$rows | Sort-Object -Property @{Expression={$_.W * $_.Hh}} -Descending | Format-Table -AutoSize Hwnd, Vis, Top, W, Hh, Cls, Title | Out-String -Width 200