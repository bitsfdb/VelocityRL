param(
    [string]$CookedDir = ""
)

$ErrorActionPreference = "Stop"
$PackageTag = [BitConverter]::ToUInt32([byte[]](0xC1, 0x83, 0x2A, 0x9E), 0)

function Read-I32([IO.BinaryReader]$Reader) {
    $b = $Reader.ReadBytes(4)
    if ($b.Length -lt 4) { throw "unexpected EOF" }
    return [BitConverter]::ToInt32($b, 0)
}

function Read-U32([IO.BinaryReader]$Reader) {
    $b = $Reader.ReadBytes(4)
    if ($b.Length -lt 4) { throw "unexpected EOF" }
    return [BitConverter]::ToUInt32($b, 0)
}

function Read-U16([IO.BinaryReader]$Reader) {
    $b = $Reader.ReadBytes(2)
    if ($b.Length -lt 2) { throw "unexpected EOF" }
    return [BitConverter]::ToUInt16($b, 0)
}

function Skip-FString([IO.BinaryReader]$Reader) {
    $len = Read-I32 $Reader
    if ($len -gt 0) {
        [void]$Reader.ReadBytes($len)
    }
    elseif ($len -lt 0) {
        [void]$Reader.ReadBytes((-$len) * 2)
    }
}

function Format-GuidHex([byte[]]$Bytes) {
    return (($Bytes | ForEach-Object { $_.ToString("X2") }) -join "")
}

function Get-UpkSummary([string]$Path) {
    $fs = [System.IO.File]::OpenRead($Path)
    $reader = New-Object System.IO.BinaryReader($fs)
    try {
        $tag = Read-U32 $reader
        if ($tag -ne $PackageTag) {
            throw "not a UPK (bad tag 0x{0:X8})" -f $tag
        }
        $fileVer = Read-U16 $reader
        $licVer = Read-U16 $reader
        [void](Read-I32 $reader)
        Skip-FString $reader
        [void](Read-U32 $reader)
        $nameCount = Read-I32 $reader
        [void](Read-I32 $reader)
        [void](Read-I32 $reader)
        [void](Read-I32 $reader)
        $importCount = Read-I32 $reader
        [void](Read-I32 $reader)
        [void](Read-I32 $reader)
        [void](Read-I32 $reader)
        $importGuids = Read-I32 $reader
        [void](Read-I32 $reader)
        [void](Read-I32 $reader)
        $guidBytes = $reader.ReadBytes(16)
        $genCount = Read-I32 $reader
        if ($genCount -lt 0 -or $genCount -gt 16384) {
            throw "implausible generation count $genCount"
        }
        if ($genCount -gt 0) {
            [void]$reader.ReadBytes($genCount * 12)
        }
        $engineVer = Read-U32 $reader
        $cookerVer = Read-U32 $reader

        return [PSCustomObject]@{
            File          = [IO.Path]::GetFileName($Path)
            FullPath      = $Path
            Size          = $fs.Length
            FileVersion   = $fileVer
            Licensee      = $licVer
            Guid          = Format-GuidHex $guidBytes
            EngineVersion = $engineVer
            CookerVersion = $cookerVer
            ImportCount   = $importCount
            ImportGuids   = $importGuids
            Modified      = (Get-Item -LiteralPath $Path).LastWriteTime
        }
    }
    finally {
        $reader.Close()
        $fs.Close()
    }
}

function Resolve-CookedDir([string]$Hint) {
    if ($Hint -and (Test-Path -LiteralPath $Hint)) {
        $p = (Resolve-Path -LiteralPath $Hint).Path
        if (Test-Path -LiteralPath (Join-Path $p "Engine.upk")) { return $p }
        $nested = Join-Path $p "TAGame\CookedPCConsole"
        if (Test-Path -LiteralPath (Join-Path $nested "Engine.upk")) { return $nested }
        throw "Engine.upk not found under $p"
    }

    $cfgPath = Join-Path $env:APPDATA "com.velocityrl.app\config.json"
    if (Test-Path -LiteralPath $cfgPath) {
        try {
            $cfg = Get-Content -LiteralPath $cfgPath -Raw | ConvertFrom-Json
            if ($cfg.game_dir -and (Test-Path -LiteralPath $cfg.game_dir)) {
                return Resolve-CookedDir $cfg.game_dir
            }
        }
        catch { }
    }

    $candidates = @(
        "E:\games\rocketleague\TAGame\CookedPCConsole",
        "E:\SteamLibrary\steamapps\common\rocketleague\TAGame\CookedPCConsole",
        "C:\Program Files\Epic Games\rocketleague\TAGame\CookedPCConsole"
    )
    foreach ($c in $candidates) {
        if (Test-Path -LiteralPath (Join-Path $c "Engine.upk")) { return $c }
    }
    throw "Could not find CookedPCConsole. Pass -CookedDir path."
}

function Find-GuidOwners([string]$Root, [string[]]$Targets) {
    $hits = @()
    $files = Get-ChildItem -LiteralPath $Root -Filter "*.upk" -Recurse -File -ErrorAction SilentlyContinue
    foreach ($f in $files) {
        try {
            $info = Get-UpkSummary $f.FullName
        }
        catch {
            continue
        }
        foreach ($t in $Targets) {
            if ($info.Guid -eq $t) {
                $hits += [PSCustomObject]@{
                    Guid = $t
                    File = $info.File
                    Path = $info.FullPath
                    Size = $info.Size
                }
            }
        }
    }
    return $hits
}

function Get-LastEngineMismatchFromLog() {
    $log = Join-Path $env:USERPROFILE "Documents\My Games\Rocket League\TAGame\Logs\Launch.log"
    if (-not (Test-Path -LiteralPath $log)) { return $null }
    $line = Select-String -LiteralPath $log -Pattern "Package 'Engine' version mismatch" |
        Select-Object -Last 1
    if (-not $line) { return $null }
    if ($line.Line -match 'package ([0-9A-F]+), info ([0-9A-F]+)') {
        return [PSCustomObject]@{
            PackageGuid = $Matches[1]
            InfoGuid    = $Matches[2]
            Line        = $line.Line.Trim()
        }
    }
    return [PSCustomObject]@{ Line = $line.Line.Trim() }
}

if ($MyInvocation.InvocationName -eq '.') {
    return
}

$cooked = Resolve-CookedDir $CookedDir
Write-Host "CookedPCConsole: $cooked" -ForegroundColor Cyan
Write-Host ""

$core = @("Engine.upk", "TAGame.upk", "Core.upk")
foreach ($name in $core) {
    $path = Join-Path $cooked $name
    if (-not (Test-Path -LiteralPath $path)) {
        Write-Host "$name  (missing)" -ForegroundColor Yellow
        continue
    }
    $s = Get-UpkSummary $path
    Write-Host ("{0,-14} {1,12} bytes  modified {2:yyyy-MM-dd HH:mm}" -f $s.File, $s.Size, $s.Modified)
    Write-Host ("  guid={0}" -f $s.Guid)
    Write-Host ("  engine_version={0} cooker_version={1} import_guids={2}" -f $s.EngineVersion, $s.CookerVersion, $s.ImportGuids)
}

$engine = Get-UpkSummary (Join-Path $cooked "Engine.upk")
$tagamePath = Join-Path $cooked "TAGame.upk"
if (Test-Path -LiteralPath $tagamePath) {
    $tagame = Get-UpkSummary $tagamePath
    Write-Host ""
    if ($engine.EngineVersion -ne $tagame.EngineVersion -or $engine.CookerVersion -ne $tagame.CookerVersion) {
        Write-Host "PREFIX VERSION MISMATCH" -ForegroundColor Red
        Write-Host ("  Engine.upk  eng={0} cook={1}" -f $engine.EngineVersion, $engine.CookerVersion)
        Write-Host ("  TAGame.upk  eng={0} cook={1}" -f $tagame.EngineVersion, $tagame.CookerVersion)
    }
    elseif ($engine.Modified -gt $tagame.Modified) {
        Write-Host "Engine.upk is newer than TAGame.upk - verify game files if online fails." -ForegroundColor Yellow
    }
    else {
        Write-Host "Prefix engine/cooker versions match between Engine.upk and TAGame.upk." -ForegroundColor Green
    }
}

$logHit = Get-LastEngineMismatchFromLog
if ($logHit) {
    Write-Host ""
    Write-Host "Last Launch.log mismatch:" -ForegroundColor Cyan
    if ($logHit.PackageGuid) {
        Write-Host ("  package {0}" -f $logHit.PackageGuid)
        Write-Host ("  info    {0}" -f $logHit.InfoGuid)
    }
    else {
        Write-Host ("  {0}" -f $logHit.Line)
    }

    if ($logHit.PackageGuid) {
        $targets = @($logHit.PackageGuid, $logHit.InfoGuid, $engine.Guid) | Select-Object -Unique
        Write-Host ""
        Write-Host "Scanning all .upk under CookedPCConsole for those GUIDs..." -ForegroundColor Cyan
        $owners = Find-GuidOwners $cooked $targets
        if ($owners.Count -eq 0) {
            Write-Host "  No local .upk package summary matches the log GUIDs." -ForegroundColor Yellow
            Write-Host "  The mismatch may be inside a swapped/modded file body, or from the server during join."
        }
        else {
            $owners | Sort-Object Guid, File | Format-Table -AutoSize
        }
    }
}

Write-Host ""
Write-Host "Backups / extras:" -ForegroundColor DarkGray
Get-ChildItem -LiteralPath $cooked -Filter "TAGame*" -File |
    Sort-Object Name |
    ForEach-Object { Write-Host ("  {0,-32} {1,12}" -f $_.Name, $_.Length) }
