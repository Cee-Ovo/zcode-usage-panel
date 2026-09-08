[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [switch]$Clean,
    [string]$ConfirmCleanup = ''
)

$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$debugRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'src-tauri\target\debug'))

# Fixed project-local boundary. Never follow junctions or symbolic links.
foreach ($relative in @('', 'src-tauri', 'src-tauri\target', 'src-tauri\target\debug', 'src-tauri\target\debug\deps')) {
    $candidate = if ($relative) { Join-Path $repoRoot $relative } else { $repoRoot }
    if (Test-Path -LiteralPath $candidate) {
        if ((Get-Item -LiteralPath $candidate -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "Refusing reparse-point boundary: $candidate"
        }
    }
}

$targets = @()
$incremental = Join-Path $debugRoot 'incremental'
if (Test-Path -LiteralPath $incremental -PathType Container) { $targets += Get-Item -LiteralPath $incremental -Force }
$depsRoot = Join-Path $debugRoot 'deps'
if (Test-Path -LiteralPath $depsRoot -PathType Container) {
    # Only old compiler temp archives, not deps itself or any compiled library.
    $targets += @(Get-ChildItem -LiteralPath $depsRoot -Directory -Force |
        Where-Object { $_.Name -like '.tmp*.temp-archive' -and $_.LastWriteTime -lt (Get-Date).AddDays(-7) })
}

$report = @(foreach ($target in $targets) {
    $fullPath = [IO.Path]::GetFullPath($target.FullName)
    if (-not $fullPath.StartsWith($debugRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Outside permitted debug boundary: $fullPath"
    }
    $entries = @(Get-ChildItem -LiteralPath $fullPath -Force -Recurse)
    if (($target.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        @($entries | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count) {
        throw "Refusing target containing reparse points: $fullPath"
    }
    $bytes = ($entries | Where-Object { -not $_.PSIsContainer } | Measure-Object Length -Sum).Sum
    [pscustomobject]@{ Path = $fullPath; Bytes = [long]$bytes; MiB = [math]::Round($bytes / 1MB, 2) }
})
$report | Format-Table Path, MiB -AutoSize
$totalBytes = ($report | Measure-Object Bytes -Sum).Sum
Write-Output ('Eligible total: {0:N3} GiB' -f ($totalBytes / 1GB))

if (-not $Clean) {
    Write-Output 'Report only. To delete these rebuildable caches, use -Clean -ConfirmCleanup DELETE_BUILD_CACHE (supports -WhatIf).'
    return
}
if ($ConfirmCleanup -cne 'DELETE_BUILD_CACHE') { throw 'Explicit confirmation missing: -ConfirmCleanup DELETE_BUILD_CACHE' }
if (Get-Process -Name cargo,rustc,link -ErrorAction SilentlyContinue) { throw 'Build process detected. Stop the build before cleanup.' }
foreach ($item in $report) {
    if ($PSCmdlet.ShouldProcess($item.Path, 'Permanently delete rebuildable compiler cache')) {
        Remove-Item -LiteralPath $item.Path -Recurse -Force
        Write-Output "Removed: $($item.Path)"
    }
}
Write-Output 'Source files, release artifacts, node_modules, global caches and user data are outside this script scope.'
