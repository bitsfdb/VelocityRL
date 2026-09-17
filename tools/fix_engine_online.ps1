param(
    [string]$CookedDir = "",
    [switch]$Fix
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "diagnose_engine.ps1")

function ConvertTo-LogGuid([string]$FileGuidHex) {
    if ($FileGuidHex.Length -ne 32) { return $FileGuidHex }
    $bytes = New-Object byte[] 16
    for ($i = 0; $i -lt 16; $i++) {
        $bytes[$i] = [Convert]::ToByte($FileGuidHex.Substring($i * 2, 2), 16)
    }
    $out = New-Object byte[] 16
    for ($w = 0; $w -lt 4; $w++) {
        for ($j = 0; $j -lt 4; $j++) {
            $out[$w * 4 + $j] = $bytes[$w * 4 + (3 - $j)]
        }
    }
    return (Format-GuidHex $out)
}

function ConvertFrom-LogGuid([string]$LogGuidHex) {
    return (ConvertTo-LogGuid $LogGuidHex)
}

function Get-LastMismatchFromLog() {
    $log = Join-Path $env:USERPROFILE "Documents\My Games\Rocket League\TAGame\Logs\Launch.log"
    if (-not (Test-Path -LiteralPath $log)) { return $null }
    $line = Select-String -LiteralPath $log -Pattern "Package 'Engine' version mismatch" | Select-Object -Last 1
    if (-not $line) { return $null }
    if ($line.Line -match "package ([0-9A-F]+), info ([0-9A-F]+)") {
        return [PSCustomObject]@{
            PackageLogGuid  = $Matches[1]
            EngineLogGuid   = $Matches[2]
            PackageFileGuid = (ConvertFrom-LogGuid $Matches[1])
            Line            = $line.Line.Trim()
        }
    }
    return [PSCustomObject]@{ Line = $line.Line.Trim() }
}

function Scan-Upks([string[]]$Paths) {
    $out = @()
    foreach ($p in $Paths) {
        if (-not (Test-Path -LiteralPath $p)) { continue }
        try { $out += Get-UpkSummary $p }
        catch { }
    }
    return $out
}

Write-Host ""
Write-Host "VelocityRL Engine online fix" -ForegroundColor Cyan
Write-Host "============================" -ForegroundColor Cyan
Write-Host ""

if (Get-Process -Name "RocketLeague" -ErrorAction SilentlyContinue) {
    Write-Host "Close Rocket League first, then run this again." -ForegroundColor Red
    exit 1
}

$cooked = Resolve-CookedDir $CookedDir
Write-Host "CookedPCConsole: $cooked"
Write-Host ""

$engine = Get-UpkSummary (Join-Path $cooked "Engine.upk")
$tagamePath = Join-Path $cooked "TAGame.upk"
$tagame = if (Test-Path -LiteralPath $tagamePath) { Get-UpkSummary $tagamePath } else { $null }

Write-Host ("Engine.upk  guid={0}  version={1}/{2}" -f $engine.Guid, $engine.EngineVersion, $engine.CookerVersion)
Write-Host ("              log guid={0}" -f (ConvertTo-LogGuid $engine.Guid))
if ($tagame) {
    Write-Host ("TAGame.upk  guid={0}  version={1}/{2}" -f $tagame.Guid, $tagame.EngineVersion, $tagame.CookerVersion)
    if ($engine.EngineVersion -eq $tagame.EngineVersion -and $engine.CookerVersion -eq $tagame.CookerVersion) {
        Write-Host "TAGame + Engine prefix versions match." -ForegroundColor Green
    }
    else {
        Write-Host "TAGame + Engine prefix versions DO NOT match - verify game files." -ForegroundColor Red
    }
}

$logHit = Get-LastMismatchFromLog
$badPackage = @()
if ($logHit -and $logHit.PackageFileGuid) {
    Write-Host ""
    Write-Host "Last Launch.log error:" -ForegroundColor Yellow
    Write-Host ("  bad package (log)  {0}" -f $logHit.PackageLogGuid)
    Write-Host ("  bad package (file) {0}" -f $logHit.PackageFileGuid)
    Write-Host ("  expected Engine    {0}" -f $logHit.EngineLogGuid)
    if ($logHit.EngineLogGuid -eq (ConvertTo-LogGuid $engine.Guid)) {
        Write-Host "  expected Engine matches your live Engine.upk." -ForegroundColor Green
    }

    $scanPaths = @(
        (Get-ChildItem -LiteralPath $cooked -Filter "*.upk" -File).FullName
    )
    $mods = Join-Path $cooked "mods"
    if (Test-Path -LiteralPath $mods) {
        $scanPaths += (Get-ChildItem -LiteralPath $mods -Filter "*.upk" -Recurse -File -ErrorAction SilentlyContinue).FullName
    }
    Write-Host ""
    Write-Host "Scanning $($scanPaths.Count) packages for the bad GUID..." -ForegroundColor Cyan
    $badPackage = Scan-Upks $scanPaths | Where-Object {
        $_.Guid -eq $logHit.PackageFileGuid -or (ConvertTo-LogGuid $_.Guid) -eq $logHit.PackageLogGuid
    }
    if ($badPackage) {
        Write-Host "Found the package that fails online:" -ForegroundColor Red
        $badPackage | Format-Table File, Size, Guid, EngineVersion, CookerVersion -AutoSize
    }
    else {
        Write-Host "No local package summary matches the bad GUID from the log." -ForegroundColor Yellow
    }
}

Write-Host ""
Write-Host "Loaded right before the error (from Launch.log):" -ForegroundColor Cyan
foreach ($name in @("GFX_Hud_SF.upk", "GFX_HudMatchInfo_SF.upk", "GameInfo_Soccar_SF.upk")) {
    $p = Join-Path $cooked $name
    if (-not (Test-Path -LiteralPath $p)) { continue }
    $s = Get-UpkSummary $p
    $ok = ($s.EngineVersion -eq $engine.EngineVersion -and $s.CookerVersion -eq $engine.CookerVersion)
    $color = if ($ok) { "DarkGray" } else { "Red" }
    Write-Host ("  {0,-24} eng {1}/{2}" -f $s.File, $s.EngineVersion, $s.CookerVersion) -ForegroundColor $color
}

$modsDir = Join-Path $cooked "mods"
if (Test-Path -LiteralPath $modsDir) {
    Write-Host ""
    Write-Host "mods folder (can break online):" -ForegroundColor Yellow
    Get-ChildItem -LiteralPath $modsDir -Recurse -File | ForEach-Object { Write-Host ("  {0}" -f $_.Name) }
}

$swapBaks = @(Get-ChildItem -LiteralPath $cooked -Filter "*.upk.bak" -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -ne "TAGame.upk.bak" })
if ($swapBaks.Count) {
    Write-Host ""
    Write-Host "Swap backups (restore with -Fix):" -ForegroundColor Yellow
    $swapBaks | ForEach-Object { Write-Host ("  {0}" -f $_.Name) }
}

if (-not $Fix) {
    Write-Host ""
    Write-Host "Diagnosis only. To apply local fixes:" -ForegroundColor Cyan
    Write-Host "  .\fix_engine_online.ps1 -Fix" -ForegroundColor White
    Write-Host ""
    Write-Host "Or run: .\diagnose_engine.ps1" -ForegroundColor DarkGray
    exit 0
}

Write-Host ""
Write-Host "Applying fixes..." -ForegroundColor Cyan
$changed = $false

if (Test-Path -LiteralPath $modsDir) {
    $disabled = Join-Path $cooked "mods.disabled"
    if (Test-Path -LiteralPath $disabled) {
        $disabled = Join-Path $cooked ("mods.disabled." + (Get-Date -Format "yyyyMMdd-HHmmss"))
    }
    Move-Item -LiteralPath $modsDir -Destination $disabled
    Write-Host "Moved mods -> $(Split-Path -Leaf $disabled)" -ForegroundColor Green
    $changed = $true
}

foreach ($bak in $swapBaks) {
    $live = $bak.FullName.Substring(0, $bak.FullName.Length - 4)
    Copy-Item -LiteralPath $bak.FullName -Destination $live -Force
    Write-Host "Restored $($bak.Name)" -ForegroundColor Green
    $changed = $true
}

foreach ($bp in $badPackage) {
    Write-Host ""
    Write-Host "Bad package: $($bp.File) at $($bp.FullPath)" -ForegroundColor Red
    $ans = Read-Host "Delete it so verify can re-download? [y/N]"
    if ($ans -match '^[yY]') {
        Remove-Item -LiteralPath $bp.FullPath -Force
        Write-Host "Deleted. Verify game files in Epic/Steam." -ForegroundColor Green
        $changed = $true
    }
}

if (-not $changed) {
    Write-Host "Nothing changed. Verify game files in Epic/Steam." -ForegroundColor Yellow
}
else {
    Write-Host ""
    Write-Host "Done. Verify game files if needed, then try online." -ForegroundColor Green
}
