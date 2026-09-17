param(
    [string]$CookedDir = "E:\games\rocketleague\TAGame\CookedPCConsole",
    [switch]$Before,
    [switch]$After,
    [string]$SnapshotDir = "",
    [string]$DumpFile = ""
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
$ProbeDir = Join-Path $RepoRoot "src-tauri"

function Invoke-PaletteDump([string]$Target) {
    Push-Location $ProbeDir
    try {
        $out = & cargo run --example palette_probe -- $Target --dump 2>&1
        if ($LASTEXITCODE -ne 0 -and -not $out) { throw "palette_probe failed for $Target" }
        return ($out | Out-String)
    }
    finally {
        Pop-Location
    }
}

function Get-ColorCounts([string]$DumpText) {
    $rows = @{}
    foreach ($line in ($DumpText -split "`n")) {
        if ($line -match '^\s+(Accent|BlueTeamV?\d*|OrangeTeamV?\d*):\s+HueCount=(\d+)\s+ValueCount=(\d+)\s+Colors=(\d+)') {
            $rows[$Matches[1]] = [pscustomobject]@{
                Hue   = [int]$Matches[2]
                Value = [int]$Matches[3]
                Colors = [int]$Matches[4]
            }
        }
        elseif ($line -match '^\s+(BlueTeam|OrangeTeam|BlueTeamV2|OrangeTeamV2|BlueTeamV3|OrangeTeamV3|Accent)\s+serial_size=(\d+)\s+offset=(\d+)') {
            $key = $Matches[1]
            if (-not $rows.ContainsKey($key)) { $rows[$key] = [pscustomobject]@{ Hue = $null; Value = $null; Colors = $null } }
            $rows[$key] | Add-Member -NotePropertyName SerialSize -NotePropertyValue ([int]$Matches[2]) -Force
            $rows[$key] | Add-Member -NotePropertyName Offset -NotePropertyValue ([int64]$Matches[3]) -Force
        }
    }
    return $rows
}

function Write-CompareReport([string]$Label, [hashtable]$Before, [hashtable]$After) {
    Write-Host ""
    Write-Host "=== $Label ==="
    $names = ($Before.Keys + $After.Keys) | Sort-Object -Unique
    foreach ($name in $names) {
        $b = $Before[$name]
        $a = $After[$name]
        if (-not $b -or -not $a) {
            Write-Host "  $name : only in $(if ($b) { 'before' } else { 'after' })"
            continue
        }
        $parts = @()
        if ($b.Hue -ne $null -and $a.Hue -ne $null -and ($b.Hue -ne $a.Hue -or $b.Value -ne $a.Value -or $b.Colors -ne $a.Colors)) {
            $parts += "grid $($b.Hue)x$($b.Value)/$($b.Colors) -> $($a.Hue)x$($a.Value)/$($a.Colors)"
        }
        if ($b.SerialSize -and $a.SerialSize -and $b.SerialSize -ne $a.SerialSize) {
            $parts += "serial $($b.SerialSize) -> $($a.SerialSize)"
        }
        if ($b.Offset -and $a.Offset -and $b.Offset -ne $a.Offset) {
            $parts += "offset $($b.Offset) -> $($a.Offset)"
        }
        if ($parts.Count -eq 0) { continue }
        Write-Host "  $name : $($parts -join '; ')"
    }
}

if ($DumpFile) {
    $tmp = Join-Path $env:TEMP "vrl-palette-dump"
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    Copy-Item $DumpFile (Join-Path $tmp "TAGame.upk") -Force
    $text = Invoke-PaletteDump $tmp
    Write-Output $text
    exit 0
}

$tagame = Join-Path $CookedDir "TAGame.upk"
if (-not (Test-Path $tagame)) { throw "Missing $tagame" }

$snapRoot = Join-Path $PSScriptRoot "snapshots"
if (-not (Test-Path $snapRoot)) { New-Item -ItemType Directory -Path $snapRoot | Out-Null }

if ($Before) {
    $dir = Join-Path $snapRoot ("shift-observe-{0:yyyyMMdd-HHmmss}" -f (Get-Date))
    New-Item -ItemType Directory -Path $dir | Out-Null
    Copy-Item $tagame (Join-Path $dir "TAGame.upk.before")
    $hash = Get-FileHash $tagame -Algorithm SHA256
    @{
        time = (Get-Date).ToString("o")
        cooked = $CookedDir
        bytes = (Get-Item $tagame).Length
        sha256 = $hash.Hash
    } | ConvertTo-Json | Set-Content (Join-Path $dir "meta.json")
    Invoke-PaletteDump $CookedDir | Set-Content (Join-Path $dir "palette-before.txt") -Encoding utf8
    Write-Host "[ok] Snapshot saved to $dir"
    Write-Host "     sha256=$($hash.Hash) bytes=$((Get-Item $tagame).Length)"
    exit 0
}

if ($After) {
    if (-not $SnapshotDir) {
        $SnapshotDir = Get-ChildItem $snapRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $SnapshotDir -or -not (Test-Path $SnapshotDir)) {
        throw "No snapshot found. Run with -Before first."
    }
    $beforeFile = Join-Path $SnapshotDir "TAGame.upk.before"
    if (-not (Test-Path $beforeFile)) { throw "Missing $beforeFile" }

    $afterDir = Join-Path $SnapshotDir "after"
    New-Item -ItemType Directory -Path $afterDir -Force | Out-Null
    Copy-Item $tagame (Join-Path $afterDir "TAGame.upk.after") -Force
    $hash = Get-FileHash $tagame -Algorithm SHA256
    @{
        time = (Get-Date).ToString("o")
        bytes = (Get-Item $tagame).Length
        sha256 = $hash.Hash
    } | ConvertTo-Json | Set-Content (Join-Path $afterDir "meta-after.json")

    $beforeDump = Get-Content (Join-Path $SnapshotDir "palette-before.txt") -Raw
    $afterDump = Invoke-PaletteDump $CookedDir
    $afterDump | Set-Content (Join-Path $afterDir "palette-after.txt") -Encoding utf8

    $meta = Get-Content (Join-Path $SnapshotDir "meta.json") -Raw | ConvertFrom-Json
    Write-Host "Before: $($meta.sha256) ($($meta.bytes) bytes)"
    Write-Host "After:  $($hash.Hash) ($((Get-Item $tagame).Length) bytes)"
    if ($meta.sha256 -eq $hash.Hash) {
        Write-Host "[warn] File hash unchanged — Shift may not have patched yet, or you need to re-apply."
    }

    $bCounts = Get-ColorCounts $beforeDump
    $aCounts = Get-ColorCounts $afterDump
    Write-CompareReport "Palette diff (team + accent)" $bCounts $aCounts

    Write-Host ""
    Write-Host "=== Sidecar files in CookedPCConsole ==="
    Get-ChildItem $CookedDir -Filter "TAGame.upk*" |
        Sort-Object Name |
        ForEach-Object { Write-Host ("  {0,-28} {1,12}  {2}" -f $_.Name, $_.Length, $_.LastWriteTime.ToString("yyyy-MM-dd HH:mm")) }

    Write-Host ""
    Write-Host "[ok] After dump saved under $afterDir"
    exit 0
}

Write-Host @"
Observe TAGame palette patches.

  .\observe_tagame_palette.ps1 -Before
  # apply rich palette in Shift (RL closed)
  .\observe_tagame_palette.ps1 -After

Optional:
  .\observe_tagame_palette.ps1 -DumpFile 'E:\path\TAGame.upk.shiftpal.orig'
"@
