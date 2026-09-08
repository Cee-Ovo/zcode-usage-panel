$ErrorActionPreference = "Stop"
$dir = "D:\Program Files\ZCode Usage Panel"
$src = "D:\linux_project\zcode-usage-panel\src-tauri\target\release\zcode-usage-panel.exe"
$dst = "$dir\zcode-usage-panel.exe"
$log = "D:\linux_project\zcode-usage-panel\.elevated-swap.log"

function Out-Log($msg) { $msg | Out-File $log -Encoding utf8 -Append }

try {
  Remove-Item $log -ErrorAction SilentlyContinue

  # running exe cannot be opened for write, stop it first
  $procs = Get-Process -Name "zcode-usage-panel" -ErrorAction SilentlyContinue
  if ($procs) {
    Out-Log ("stopping running instance(s): " + (($procs | ForEach-Object { $_.Id }) -join ", "))
    $procs | Stop-Process -Force
    Start-Sleep -Milliseconds 800
  }

  $srcHash = (Get-FileHash $src -Algorithm MD5).Hash
  $dstHash = (Get-FileHash $dst -Algorithm MD5).Hash

  if ($srcHash -eq $dstHash) {
    Out-Log "target exe identical to installed one, skip copy"
  } else {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $backup = "$dir\zcode-usage-panel.pre-$stamp.exe"
    Copy-Item $dst $backup -Force
    Copy-Item $src $dst -Force
    Out-Log "OK swapped (backup: $(Split-Path -Leaf $backup))"
  }

  # restart de-elevated via shell
  Start-Sleep -Milliseconds 300
  explorer.exe "$dst"
  Out-Log "restart requested via explorer"
} catch {
  Out-Log ("FAILED: " + $_.ToString())
  exit 1
}
