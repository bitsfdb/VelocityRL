# Stop stale VelocityRL psynet_proxy.exe and wait for :443 to release.
# .\stop_proxy.ps1 [-WaitMs 8000] [-Quiet] [-RevertHosts]
#
# Used by start_from_app.ps1 and the desktop app (Rust elevated setup / stop).
# -RevertHosts strips config/api/ws.psynet hosts redirects (former revert_hosts.ps1).
param(
    [int]$WaitMs = 8000,
    [switch]$Quiet,
    [switch]$RevertHosts
)

$ErrorActionPreference = "SilentlyContinue"
$here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }

function Stop-Log([string]$Message) {
    if (-not $Quiet) {
        Write-Host $Message
    }
}

function Stop-ProxyByName {
    # taskkill /T handles elevated orphans better than Stop-Process alone.
    & taskkill.exe /F /IM psynet_proxy.exe /T 2>$null | Out-Null
    Get-Process -Name psynet_proxy -ErrorAction SilentlyContinue |
        ForEach-Object {
            Stop-Log "stop_proxy: killing psynet_proxy pid=$($_.Id)"
            Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
            & taskkill.exe /PID $_.Id /F /T 2>$null | Out-Null
        }
    # Second pass: image name kill can miss very fresh respawns.
    Start-Sleep -Milliseconds 100
    & taskkill.exe /F /IM psynet_proxy.exe /T 2>$null | Out-Null
}

function Stop-ProxyListenersOnPort {
    param([int]$Port)
    $conns = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
    foreach ($c in $conns) {
        $ownerPid = [int]$c.OwningProcess
        if ($ownerPid -le 0) { continue }
        $owner = Get-Process -Id $ownerPid -ErrorAction SilentlyContinue
        $name = if ($owner) { $owner.ProcessName } else { "?" }
        if ($name -ne "psynet_proxy") { continue }
        Stop-Log "stop_proxy: killing psynet_proxy pid=$ownerPid listening on :$Port"
        Stop-Process -Id $ownerPid -Force -ErrorAction SilentlyContinue
        & taskkill.exe /PID $ownerPid /F /T 2>$null | Out-Null
    }
}

function Test-ProxyPortsHeld {
    $procs = @(Get-Process -Name psynet_proxy -ErrorAction SilentlyContinue)
    $on443 = @()
    $conns = @(Get-NetTCPConnection -LocalPort 443 -State Listen -ErrorAction SilentlyContinue)
    foreach ($c in $conns) {
        $owner = Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
        if ($owner -and $owner.ProcessName -eq "psynet_proxy") {
            $on443 += $c.OwningProcess
        }
    }
    return @{
        Processes = $procs
        Port443   = $on443
    }
}

Stop-Log "stop_proxy: stopping psynet_proxy.exe (wait up to ${WaitMs}ms for :443)..."
Stop-ProxyByName
Stop-ProxyListenersOnPort -Port 443

function Read-PidFile([string]$Path) {
    for ($attempt = 1; $attempt -le 5; $attempt++) {
        try {
            $fs = [System.IO.FileStream]::new(
                $Path,
                [System.IO.FileMode]::Open,
                [System.IO.FileAccess]::Read,
                ([System.IO.FileShare]"ReadWrite, Delete")
            )
            try {
                $reader = New-Object System.IO.StreamReader($fs)
                $text = $reader.ReadToEnd().Trim()
                $reader.Dispose()
                if ($text -match '^\d+$') { return [int]$text }
                return 0
            } finally { $fs.Dispose() }
        } catch [System.IO.IOException] {
            if ($attempt -eq 5) { return 0 }
            Start-Sleep -Milliseconds (100 * $attempt)
        } catch { return 0 }
    }
    return 0
}

function Remove-PidFile([string]$Path) {
    for ($attempt = 1; $attempt -le 5; $attempt++) {
        try {
            if (Test-Path -LiteralPath $Path) {
                Remove-Item -LiteralPath $Path -Force -ErrorAction Stop
            }
            return
        } catch {
            if ($attempt -eq 5) { return }
            Start-Sleep -Milliseconds (100 * $attempt)
        }
    }
}

$pidFile = Join-Path $here "proxy.pid"
if (Test-Path -LiteralPath $pidFile) {
    $oldPid = Read-PidFile -Path $pidFile
    if ($oldPid -gt 0) {
        $stale = Get-Process -Id $oldPid -ErrorAction SilentlyContinue
        if ($stale -and $stale.ProcessName -eq 'psynet_proxy') {
            Stop-Log "stop_proxy: killing stale proxy.pid=$oldPid"
            Stop-Process -Id $oldPid -Force -ErrorAction SilentlyContinue
            & taskkill.exe /PID $oldPid /F /T 2>$null | Out-Null
        }
    }
    Remove-PidFile -Path $pidFile
}

$deadline = [Environment]::TickCount + $WaitMs
$released = $false
while ([Environment]::TickCount -lt $deadline) {
    Stop-ProxyByName
    Stop-ProxyListenersOnPort -Port 443
    $held = Test-ProxyPortsHeld
    if ($held.Processes.Count -eq 0 -and $held.Port443.Count -eq 0) {
        $released = $true
        break
    }
    Start-Sleep -Milliseconds 250
}

if (-not $released) {
    $held = Test-ProxyPortsHeld
    $msg = "stop_proxy: psynet_proxy still running or holding ports after ${WaitMs}ms"
    if ($held.Processes.Count -gt 0) {
        $msg += " (pids=$($held.Processes.Id -join ','))"
    }
    if ($held.Port443.Count -gt 0) {
        $msg += " (:443 pids=$($held.Port443 -join ','))"
    }
    $msg += ". Run as Admin: taskkill /F /IM psynet_proxy.exe /T"
    Stop-Log $msg
    if (-not $RevertHosts) { exit 1 }
} else {
    Stop-Log "stop_proxy: ports released"
}

# Warn if another process reclaimed loopback :443 (RL would hit that process, not us).
$foreign443 = @(Get-NetTCPConnection -LocalPort 443 -State Listen -ErrorAction SilentlyContinue |
    Where-Object { $_.LocalAddress -eq '127.0.0.1' } |
    ForEach-Object {
        $o = Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue
        if ($o -and $o.ProcessName -ne 'psynet_proxy') { $_ }
    })
if ($foreign443.Count -gt 0) {
    Stop-Log "stop_proxy: note another process owns 127.0.0.1:443 - quit it before starting the VelocityRL proxy"
}

if ($RevertHosts) {
    $hostsPath = "$env:SystemRoot\System32\drivers\etc\hosts"
    try {
        $lines = @(Get-Content -LiteralPath $hostsPath -ErrorAction Stop)
        $filtered = @($lines | Where-Object {
            $_ -notmatch 'config\.psynet\.gg' -and
            $_ -notmatch 'api\.rlpp\.psynet\.gg' -and
            $_ -notmatch 'ws\.rlpp\.psynet\.gg'
        })
        if ($filtered.Count -eq $lines.Count) {
            Stop-Log "stop_proxy: hosts already clean (no psynet redirects)"
        } else {
            $bytes = [System.Text.Encoding]::ASCII.GetBytes(($filtered -join "`r`n") + "`r`n")
            $fs = [System.IO.FileStream]::new(
                $hostsPath,
                [System.IO.FileMode]::Create,
                [System.IO.FileAccess]::Write,
                ([System.IO.FileShare]"ReadWrite, Delete")
            )
            try { $fs.Write($bytes, 0, $bytes.Length) } finally { $fs.Dispose() }
            Stop-Log "stop_proxy: hosts removed config/api/ws.psynet redirects"
        }
        ipconfig /flushdns | Out-Null
        Stop-Log "stop_proxy: DNS flushed"
    } catch {
        Stop-Log "stop_proxy: hosts revert failed: $($_.Exception.Message)"
        if (-not $released) { exit 1 }
        exit 1
    }
}

if (-not $released) { exit 1 }
exit 0
