# Elevated one-shot: install CA + hosts for config.psynet.gg MITM, then start proxy.
# Called by VelocityRL. Logs to start_from_app.log next to this script.
#
# Trust model (DO NOT CHANGE):
#   - VelocityRL CA -> LocalMachine\Root only (Schannel)
#   - hosts: config.psynet.gg always; NEVER api.rlpp (Auth uses PsyNetUrl broker rewrite)
#   - NEVER add ws.rlpp hosts for shipping spoofs — PerConURL is rewritten to local broker WS
#   - AuthPlayer request pass-through; response PerConURL/PerConURLv2 -> http://127.0.0.1:27505/ws/...
#     ALWAYS while proxy is running (2.0)
#   - Broker forced ON always (persisted into config)
#   - inventory_spoof is force-cleared (feature removed)
#   - ping_spoof is force-cleared (feature removed)
#   - openssl_trust APPEND when openssl_trust=true (needed for ws OpenSSL leaf)
#   - NEVER write/overwrite OpenSSL bundles:
#       C:\Program Files\Common Files\SSL\cert.pem
#       C:\Windows\cert.pem
#       game cacert.pem / curl-ca-bundle
#       CAPATH hash dirs under Common Files\SSL\certs or C:\Windows\certs
#     Singleton VelocityRL-only cert.pem previously broke EAC/EOS TLS.
$ErrorActionPreference = "Stop"
$here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$logPath = Join-Path $here "start_from_app.log"
$script:startLockStream = $null

# Append with FileShare so tailing editors / concurrent starts do not fail on lock.
function Write-LogBytes([string]$Path, [byte[]]$Bytes, [int]$MaxAttempts = 5) {
    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        try {
            $fs = [System.IO.FileStream]::new(
                $Path,
                [System.IO.FileMode]::OpenOrCreate,
                [System.IO.FileAccess]::Write,
                ([System.IO.FileShare]"ReadWrite, Delete")
            )
            try {
                $fs.Seek(0, [System.IO.SeekOrigin]::End) | Out-Null
                $fs.Write($Bytes, 0, $Bytes.Length)
                return $true
            } finally { $fs.Dispose() }
        } catch [System.IO.IOException] {
            if ($attempt -eq $MaxAttempts) { return $false }
            Start-Sleep -Milliseconds (150 * $attempt)
        }
    }
    return $false
}

function Initialize-LogSession {
    $stamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $header = "`r`n========== session $stamp pid=$PID ==========`r`n"
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($header)
    if (-not (Write-LogBytes -Path $logPath -Bytes $bytes)) {
        Write-Warning "start_from_app: log file locked ($logPath) - continuing without file log"
    }
}

function Log([string]$Message) {
    $line = "[{0}] {1}" -f (Get-Date -Format "HH:mm:ss"), $Message
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($line + "`r`n")
    if (-not (Write-LogBytes -Path $logPath -Bytes $bytes -MaxAttempts 3)) {
        Write-Warning "start_from_app: could not append to log (locked): $line"
    }
    Write-Host $line
}

# Windows PowerShell 5.1 Set-Content -Encoding UTF8 writes a BOM that breaks Go encoding/json.
function Write-PsyNetConfigJson([object]$Cfg, [string]$Path) {
    $json = $Cfg | ConvertTo-Json -Depth 20
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($Path, $json, $utf8NoBom)
}

# Atomic text write with shared read — avoids proxy.pid / log lock races.
function Write-AtomicTextFile {
    param(
        [string]$Path,
        [string]$Text,
        [int]$MaxAttempts = 8
    )
    $bytes = [System.Text.Encoding]::ASCII.GetBytes($Text)
    $tmp = "$Path.tmp.$PID"
    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        try {
            $fs = [System.IO.FileStream]::new(
                $tmp,
                [System.IO.FileMode]::Create,
                [System.IO.FileAccess]::Write,
                ([System.IO.FileShare]"ReadWrite, Delete")
            )
            try {
                $fs.Write($bytes, 0, $bytes.Length)
            } finally { $fs.Dispose() }
            if ([System.IO.File]::Exists($Path)) {
                [System.IO.File]::Delete($Path)
            }
            [System.IO.File]::Move($tmp, $Path)
            return $true
        } catch [System.IO.IOException] {
            if ($attempt -eq $MaxAttempts) { return $false }
            Start-Sleep -Milliseconds (100 * $attempt)
        } finally {
            Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
        }
    }
    return $false
}

# CAPATH / OpenSSL repair that used to live in fix_eac_trust.ps1.
# Never creates cert.pem if missing; only strips plants or restores a destroyed singleton.
function Repair-EacHygiene {
    Log "EAC/OpenSSL hygiene"
    $caPathLocal = Join-Path $here "velocityrl_ca.crt"
    $caBytes = $null
    if (Test-Path -LiteralPath $caPathLocal) {
        $caBytes = [System.IO.File]::ReadAllBytes($caPathLocal)
    }

    foreach ($dir in @("C:\Windows\certs", "${env:ProgramFiles}\Common Files\SSL\certs")) {
        if (-not (Test-Path -LiteralPath $dir)) { continue }
        Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue |
            Where-Object { -not $_.PSIsContainer } |
            ForEach-Object {
                try {
                    $b = [System.IO.File]::ReadAllBytes($_.FullName)
                    $txt = [System.Text.Encoding]::ASCII.GetString($b)
                    $tinyPem = ($b.Length -lt 2500) -and $txt.Contains("BEGIN CERTIFICATE")
                    $sameCa = ($null -ne $caBytes -and $b.Length -eq $caBytes.Length)
                    if ($tinyPem -or $sameCa) {
                        Remove-Item -LiteralPath $_.FullName -Force
                        Log "hygiene: removed CAPATH plant $($_.FullName) (len=$($b.Length))"
                    }
                } catch { Log "warn: CAPATH $($_.FullName): $_" }
            }
        $left = @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue)
        if ((Test-Path -LiteralPath $dir) -and $left.Count -eq 0) {
            Remove-Item -LiteralPath $dir -Force -ErrorAction SilentlyContinue
        }
    }

    function Test-MozillaPem([string]$PemPath) {
        if (-not (Test-Path -LiteralPath $PemPath)) { return $false }
        if ((Get-Item -LiteralPath $PemPath).Length -lt 50000) { return $false }
        $raw = Get-Content -LiteralPath $PemPath -Raw -ErrorAction SilentlyContinue
        if (-not $raw) { return $false }
        $n = ([regex]::Matches($raw, "-----BEGIN CERTIFICATE-----")).Count
        return ($n -ge 50) -and ($raw -match "Mozilla|DigiCert|ISRG Root|GlobalSign")
    }

    $pemPaths = @(
        "C:\Windows\cert.pem",
        "${env:ProgramFiles}\Common Files\SSL\cert.pem"
    )
    $needRestore = $false
    foreach ($pem in $pemPaths) {
        if (-not (Test-Path -LiteralPath $pem)) { continue }
        $raw = Get-Content -LiteralPath $pem -Raw -ErrorAction SilentlyContinue
        if (-not $raw) { continue }
        if (Test-MozillaPem $pem) {
            if ($raw -match "VelocityRL") {
                $cleaned = [regex]::Replace($raw, '(?s)\r?\n?# VelocityRL CA[\s\S]*\z', "")
                $utf8 = New-Object System.Text.UTF8Encoding $false
                [System.IO.File]::WriteAllText($pem, $cleaned.TrimEnd() + "`n", $utf8)
                Log "hygiene: stripped VelocityRL append from $pem"
            }
        } else {
            $needRestore = $true
            Log "hygiene: bad/singleton OpenSSL bundle $pem (len=$((Get-Item -LiteralPath $pem).Length))"
        }
    }

    if ($needRestore) {
        $tmp = Join-Path $env:TEMP "velocityrl_cacert_mozilla.pem"
        try {
            Log "hygiene: downloading Mozilla CA bundle from curl.se"
            Invoke-WebRequest -Uri "https://curl.se/ca/cacert.pem" -OutFile $tmp -UseBasicParsing
            if (-not (Test-MozillaPem $tmp)) { throw "downloaded cacert.pem failed Mozilla check" }
            foreach ($pem in $pemPaths) {
                if (-not (Test-Path -LiteralPath $pem)) { continue }
                Copy-Item -LiteralPath $tmp -Destination $pem -Force
                Log "hygiene: restored Mozilla bundle $pem"
            }
        } catch {
            Log "warn: Mozilla restore failed: $($_.Exception.Message)"
        }
    }

    $marker = Join-Path $here "openssl_trust\INSTALLED_APPEND.txt"
    if (Test-Path -LiteralPath $marker) {
        Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
    }
}

try {
    Initialize-LogSession
    Log "start_from_app.ps1 beginning in $here"
    Push-Location $here

    # Single-flight: another elevated start must not kill/race this one.
    $startLockPath = Join-Path $here "start_from_app.lock"
    $script:startLockStream = $null
    $gotLock = $false
    for ($lockTry = 0; $lockTry -lt 60; $lockTry++) {
        try {
            $script:startLockStream = [System.IO.File]::Open(
                $startLockPath,
                [System.IO.FileMode]::OpenOrCreate,
                [System.IO.FileAccess]::ReadWrite,
                [System.IO.FileShare]::None
            )
            $gotLock = $true
            break
        } catch {
            # Another start in progress — if proxy already healthy, we are done.
            $healthyWait = $false
            try {
                $listeners = @(Get-NetTCPConnection -LocalPort 443 -State Listen -ErrorAction SilentlyContinue |
                    Where-Object { $_.LocalAddress -in @('127.0.0.1', '::1') })
                foreach ($c in $listeners) {
                    $owner = Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
                    if ($owner -and $owner.ProcessName -eq 'psynet_proxy') { $healthyWait = $true; break }
                }
            } catch { }
            if ($healthyWait) {
                Log "another start in progress and psynet_proxy already owns :443 — skip"
                exit 0
            }
            Start-Sleep -Milliseconds 500
        }
    }
    if (-not $gotLock) {
        throw "Another proxy start is already in progress (could not acquire start_from_app.lock). Wait a few seconds and retry."
    }
    try {
        $lockBytes = [System.Text.Encoding]::UTF8.GetBytes("$PID $(Get-Date -Format o)`n")
        $script:startLockStream.SetLength(0)
        $script:startLockStream.Write($lockBytes, 0, $lockBytes.Length)
        $script:startLockStream.Flush()
    } catch { }

    function Test-PsyNetOwns443 {
        try {
            $listeners = @(Get-NetTCPConnection -LocalPort 443 -State Listen -ErrorAction SilentlyContinue |
                Where-Object { $_.LocalAddress -in @('127.0.0.1', '::1') })
            if ($listeners.Count -eq 0) { return $false }
            foreach ($c in $listeners) {
                $owner = Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
                if (-not $owner -or $owner.ProcessName -ne 'psynet_proxy') { return $false }
            }
            return $true
        } catch { return $false }
    }

    if (Test-PsyNetOwns443) {
        Log "psynet_proxy already listening on loopback :443 — skip stop/restart (config hot-reloads)"
        exit 0
    }

    $exe = Join-Path $here "psynet_proxy.exe"
    if (-not (Test-Path $exe)) {
        throw "psynet_proxy.exe not found in $here - build it first (go build -o psynet_proxy.exe)."
    }
    $exeInfo = Get-Item -LiteralPath $exe
    Log "psynet_proxy.exe path=$exe mtime=$($exeInfo.LastWriteTime) size=$($exeInfo.Length)"

    $caCert = Join-Path $here "velocityrl_ca.crt"
    $serverCrt = Join-Path $here "server.crt"
    $serverKey = Join-Path $here "server.key"

    $needCerts = -not (Test-Path $caCert) -or -not (Test-Path $serverCrt) -or -not (Test-Path $serverKey)
    if ($needCerts) {
        Log "certs missing - running gen_certs.py"
        $py = $null
        foreach ($name in @("py", "python", "python3")) {
            $cmd = Get-Command $name -ErrorAction SilentlyContinue
            if ($cmd) { $py = $cmd.Source; break }
        }
        foreach ($candidate in @(
            "$env:LOCALAPPDATA\Programs\Python\Python312\python.exe",
            "$env:LOCALAPPDATA\Programs\Python\Python311\python.exe",
            "$env:LOCALAPPDATA\Programs\Python\Python310\python.exe",
            "C:\Python311\python.exe",
            "C:\Python312\python.exe"
        )) {
            if (-not $py -and (Test-Path $candidate)) { $py = $candidate }
        }
        if (-not $py) {
            throw "Python not found in elevated PATH and certs are missing. Run gen_certs.py once as your user, or install Python."
        }
        Log "using python: $py"
        & $py (Join-Path $here "gen_certs.py")
        if ($LASTEXITCODE -ne 0) { throw "gen_certs.py failed (exit $LASTEXITCODE)" }
    } else {
        Log "certs present - skipping gen_certs.py"
    }

    if (-not (Test-Path $caCert)) { throw "missing velocityrl_ca.crt" }

    Log "Installing VelocityRL root CA..."
    # Remove old VelocityRL CA + any leaf/host/mitmproxy certs accidentally imported into Root.
    # Never import leaf_*.crt / server.crt into Root - CA only.
    # Never touch OpenSSL cert.pem / CAPATH (that previously broke EAC/EOS).
    Get-ChildItem Cert:\LocalMachine\Root, Cert:\CurrentUser\Root -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Subject -like "*VelocityRL*" -or
            $_.Issuer -like "*VelocityRL*" -or
            $_.Subject -like "*mitmproxy*" -or
            $_.Issuer -like "*mitmproxy*" -or
            $_.Subject -match 'CN=(config\.psynet\.gg|api\.rlpp\.psynet\.gg|ws\.rlpp\.psynet\.gg)' -or
            $_.Subject -like '*config.psynet.gg*' -or
            $_.Subject -like '*api.rlpp.psynet.gg*' -or
            $_.Subject -like '*ws.rlpp.psynet.gg*'
        } |
        ForEach-Object {
            try {
                Log "removing old root cert: $($_.Subject) $($_.Thumbprint)"
                Remove-Item $_.PSPath -Force -ErrorAction Stop
            } catch { Log "warn: could not remove old cert: $_" }
        }
    Import-Certificate -FilePath $caCert -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
    Log "CA installed (LocalMachine\\Root)"

    # One-shot hygiene each start (idempotent): CAPATH/Mozilla cleanup.
    # Strips VelocityRL from OpenSSL cert.pem. If openssl_trust is on we
    # re-append after hygiene (append-only, never singleton replace).
    Repair-EacHygiene

    # Safety: refuse to continue if a prior bad plant is still present.
    # This script must NEVER create those paths from scratch.
    foreach ($pem in @(
        "${env:ProgramFiles}\Common Files\SSL\cert.pem",
        "C:\Windows\cert.pem"
    )) {
        if (Test-Path -LiteralPath $pem) {
            $len = (Get-Item -LiteralPath $pem).Length
            if ($len -lt 5000) {
                throw "Refusing to start: $pem is only $len bytes (likely VelocityRL-only). Restore a Mozilla CA bundle to this path — overwriting it broke EAC/EOS."
            }
        }
    }
    foreach ($capath in @(
        "C:\Windows\certs",
        "${env:ProgramFiles}\Common Files\SSL\certs"
    )) {
        if (Test-Path -LiteralPath $capath) {
            $plants = @(Get-ChildItem -LiteralPath $capath -Force -ErrorAction SilentlyContinue |
                Where-Object { -not $_.PSIsContainer -and $_.Length -lt 2500 })
            if ($plants.Count -gt 0) {
                throw "Refusing to start: CAPATH plants still present under $capath."
            }
        }
    }

    $crl = Join-Path $here "velocityrl.crl"
    if (Test-Path $crl) {
        try { certutil -addstore -f CA $crl | Out-Null } catch { Log "warn: CRL install: $_" }
    }

    function Add-HostsEntry {
        param([string]$Path, [string]$Line)
        for ($attempt = 1; $attempt -le 5; $attempt++) {
            try {
                $fs = [System.IO.FileStream]::new(
                    $Path,
                    [System.IO.FileMode]::Append,
                    [System.IO.FileAccess]::Write,
                    ([System.IO.FileShare]"ReadWrite, Delete")
                )
                try {
                    $bytes = [System.Text.Encoding]::ASCII.GetBytes("`r`n$Line")
                    $fs.Write($bytes, 0, $bytes.Length)
                    return $true
                } finally { $fs.Dispose() }
            } catch [System.IO.IOException] {
                if ($attempt -eq 5) { throw }
                Start-Sleep -Milliseconds 500
            }
        }
        return $false
    }

    $hostsPath = "$env:SystemRoot\System32\drivers\etc\hosts"

    # rewrite_ws_url legacy flag — PerCon rewrite follows broker / WS spoofs.
    $nameSpoofOn = $false  # legacy api.rlpp TLS MITM - never add api hosts
    $wsSpoofOn = $false
    $opensslTrustOn = $false
    $brokerRpcOn = $false
    $cfgPath = Join-Path $here "psynet_config.json"
    if (Test-Path $cfgPath) {
        try {
            $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
            # Drop stale observe_only flags from older builds (they disabled all spoofs).
            if ($null -ne $cfg.PSObject.Properties['observe_only']) {
                $cfg.PSObject.Properties.Remove('observe_only')
                Write-PsyNetConfigJson $cfg $cfgPath
                Log "stripped stale observe_only from psynet_config.json"
            }
            if ($cfg.name_spoof -and $cfg.name_spoof.enabled) {
                Log "WARN: name_spoof.enabled is legacy - ignored; broker path is used instead"
            }
            # Broker ON / fake_ranks ⇒ local WS rewrite (no ws hosts).
            # Titles/camera are config.psynet.gg only.
            $fakeRanksOn = $false
            if ($cfg.fake_ranks -and $cfg.fake_ranks.enabled -eq $true) { $fakeRanksOn = $true }
            $pingSpoofOn = $false
            if ($cfg.ping_spoof -and $cfg.ping_spoof.enabled -eq $true) { $pingSpoofOn = $true }
            # Inventory spawn removed — strip any stale enabled flag from disk.
            if ($cfg.inventory_spoof) {
                $hadInv = ($cfg.inventory_spoof.enabled -eq $true) -or ($cfg.inventory_spoof.items -and @($cfg.inventory_spoof.items).Count -gt 0)
                $cfg.inventory_spoof = [pscustomobject]@{ enabled = $false; items = @() }
                if ($hadInv) {
                    Write-PsyNetConfigJson $cfg $cfgPath
                    Log "inventory_spoof forced OFF (feature removed)"
                }
            }
            # ping_spoof removed — never leave enabled on disk.
            if ($cfg.ping_spoof -and $cfg.ping_spoof.enabled -eq $true) {
                $cfg.ping_spoof = [pscustomobject]@{ enabled = $false; ms = 0 }
                Write-PsyNetConfigJson $cfg $cfgPath
                Log "ping_spoof forced OFF (feature removed)"
                $pingSpoofOn = $false
            }
            $logoOn = $false
            if ($cfg.logo_spoof -and $cfg.logo_spoof.enabled -eq $true) { $logoOn = $true }
            $blogOn = $false
            if ($cfg.blog_spoof -and $cfg.blog_spoof.enabled -eq $true) { $blogOn = $true }
            $cameraOn = $false
            if ($cfg.camera_spoof -and $cfg.camera_spoof.enabled -eq $true) { $cameraOn = $true }
            $titlesOn = $false
            if ($cfg.enabled -eq $true -and (($cfg.swaps -and @($cfg.swaps).Count -gt 0) -or $cfg.equip_title_id)) { $titlesOn = $true }

            # 2.0: local broker + WS rewrite always on while proxy runs (config hosts only).
            $brokerRpcOn = $true
            if (-not $cfg.name_spoof) {
                $cfg | Add-Member -NotePropertyName name_spoof -NotePropertyValue ([pscustomobject]@{}) -Force
            }
            $ns = $cfg.name_spoof
            $ns | Add-Member -NotePropertyName broker -NotePropertyValue $true -Force
            $ns | Add-Member -NotePropertyName rewrite_ws_url -NotePropertyValue $false -Force
            $ns | Add-Member -NotePropertyName enabled -NotePropertyValue $false -Force
            $ns | Add-Member -NotePropertyName websocket -NotePropertyValue $false -Force
            $cfg.name_spoof = $ns
            Write-PsyNetConfigJson $cfg $cfgPath
            Log "broker ON (local RPC + WS rewrite; config hosts only)"
            if ($fakeRanksOn) { Log "fake_ranks ON" }

            # Never add ws.rlpp hosts for shipping spoofs.
            if ($fakeRanksOn) {
                Log "WS spoofs ON (via local broker)"
            }

            Log ("spoofs: titles={0} logo={1} blog={2} camera={3} fake_ranks={4} ping={5} broker={6}" -f `
                $titlesOn, $logoOn, $blogOn, $cameraOn, $fakeRanksOn, $pingSpoofOn, $brokerRpcOn)
            if ($cameraOn) { Log "camera_spoof ON" }
            if ($pingSpoofOn) {
                Log "WARN: ping_spoof still marked on after force-off"
            }

            if ($cfg.name_spoof -and ($cfg.name_spoof.classprop_name -or $cfg.name_spoof.broker -or $brokerRpcOn)) {
                $disp = [string]$cfg.name_spoof.display_name
                if ([string]::IsNullOrWhiteSpace($disp) -and $cfg.custom_name) {
                    $disp = [string]$cfg.custom_name
                }
                if ($cfg.name_spoof.websocket -eq $true -or $cfg.name_spoof.ws_enabled -eq $true) {
                    if (-not $brokerRpcOn) {
                        $wsSpoofOn = $true
                    }
                }
                if ($cfg.name_spoof.openssl_trust -eq $true -and $wsSpoofOn) {
                    $opensslTrustOn = $true
                }
                if ($cfg.name_spoof.broker -or $brokerRpcOn) {
                    if ($wsSpoofOn) {
                        Log "name_spoof broker ON display=$disp (websocket path active)"
                    } else {
                        Log "name_spoof broker ON display=$disp"
                    }
                } else {
                    Log "name_spoof classprop ON display=$disp"
                }
            } elseif (-not $wsSpoofOn) {
                Log "name_spoof off"
            }
            if (-not $cfg.method -or $cfg.method -match 'json') {
                $cfg | Add-Member -NotePropertyName method -NotePropertyValue "raw" -Force
                Write-PsyNetConfigJson $cfg $cfgPath
                Log "psynet_config.json method forced to raw"
            } else {
                Log "psynet_config.json method=$($cfg.method)"
            }
        } catch { Log "warn: could not validate psynet_config.json: $_" }
    }

    # Strip hosts that are not allowed for this run.
    # - ws: only keep when wsSpoofOn
    # - api: always strip (RPC via PsyNetUrl -> local broker, not api.rlpp hosts)
    if ($opensslTrustOn) {
        $installOt = Join-Path $here "install_openssl_trust.ps1"
        if (-not (Test-Path -LiteralPath $installOt)) {
            Log "WARN: openssl_trust=true but install_openssl_trust.ps1 removed - skipping OpenSSL append"
        } else {
            Log "openssl_trust=true - re-APPEND after OpenSSL hygiene"
            & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $installOt 2>&1 |
                ForEach-Object { Log "openssl_trust: $_" }
            if ($LASTEXITCODE -ne 0) {
                Log "WARN: install_openssl_trust.ps1 FAILED exit=$LASTEXITCODE"
            } else {
                foreach ($pem in @(
                    "${env:ProgramFiles}\Common Files\SSL\cert.pem",
                    "C:\Windows\cert.pem"
                )) {
                    if (-not (Test-Path -LiteralPath $pem)) {
                        Log "WARN: openssl_trust verify: missing $pem"
                        continue
                    }
                    $raw = Get-Content -LiteralPath $pem -Raw
                    if ($raw -notmatch "# VelocityRL CA") {
                        Log "WARN: openssl_trust verify: $pem missing # VelocityRL CA"
                    } else {
                        Log "openssl_trust verified: $pem"
                    }
                }
            }
        }
    } elseif ($wsSpoofOn) {
        Log "WARN: websocket=ON openssl_trust=OFF - expect tls: unknown certificate authority on leaf_ws"
    }

    $hostsLines = @(Get-Content $hostsPath -ErrorAction Stop)
    $filtered = @($hostsLines | Where-Object {
        $line = $_
        $drop = $false
        if ($line -match 'ws\.rlpp\.psynet\.gg' -and -not $wsSpoofOn) { $drop = $true }
        if ($line -match 'api\.rlpp\.psynet\.gg') { $drop = $true }
        -not $drop
    })
    if ($filtered.Count -ne $hostsLines.Count) {
        Log "hosts: removing api.rlpp (always) and ws.rlpp unless WS spoof on"
        $bytes = [System.Text.Encoding]::ASCII.GetBytes(($filtered -join "`r`n") + "`r`n")
        $fs = [System.IO.FileStream]::new(
            $hostsPath,
            [System.IO.FileMode]::Create,
            [System.IO.FileAccess]::Write,
            ([System.IO.FileShare]"ReadWrite, Delete")
        )
        try { $fs.Write($bytes, 0, $bytes.Length) } finally { $fs.Dispose() }
    }

    $hosts = (Get-Content $hostsPath -Raw -ErrorAction SilentlyContinue)
    if ($null -eq $hosts) { $hosts = "" }
    $hostPairs = @(
        @{ Ip = "127.0.0.1"; Host = "config.psynet.gg" },
        @{ Ip = "::1"; Host = "config.psynet.gg" }
    )
    if ($wsSpoofOn) {
        $hostPairs += @(
            @{ Ip = "127.0.0.1"; Host = "ws.rlpp.psynet.gg" },
            @{ Ip = "::1"; Host = "ws.rlpp.psynet.gg" }
        )
    }
    foreach ($pair in $hostPairs) {
        $pat = [regex]::Escape($pair.Ip) + "\s+" + [regex]::Escape($pair.Host)
        if ($hosts -notmatch $pat) {
            Add-HostsEntry -Path $hostsPath -Line "$($pair.Ip) $($pair.Host)" | Out-Null
            $hosts += "`r`n$($pair.Ip) $($pair.Host)"
            Log "hosts: added $($pair.Ip) $($pair.Host)"
        } else {
            Log "hosts: $($pair.Ip) $($pair.Host) already present"
        }
    }

    # Always stop stale proxy (survives force-close / prior session).
    $stopProxy = Join-Path $here "stop_proxy.ps1"
    if (-not (Test-Path -LiteralPath $stopProxy)) {
        throw "missing stop_proxy.ps1 next to start_from_app.ps1"
    }
    Log "running stop_proxy.ps1 (kill psynet_proxy + wait for :443)"
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $stopProxy -WaitMs 10000 -Quiet | ForEach-Object { Log "stop: $_" }
    if ($LASTEXITCODE -ne 0) {
        throw "Old psynet_proxy.exe still holds :443. Run as Admin: .\stop_proxy.ps1 then retry. Or: taskkill /F /IM psynet_proxy.exe /T"
    }

    # Refuse to start if something else still owns loopback :443.
    try {
        foreach ($port in @(443)) {
            $busy = @(Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue |
                Where-Object { $_.LocalAddress -in @('127.0.0.1', '::1') })
            foreach ($c in $busy) {
                $owner = Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
                $name = if ($owner) { $owner.ProcessName } else { "?" }
                if ($name -eq "psynet_proxy") {
                    throw "Port $port still held by psynet_proxy pid=$($c.OwningProcess) after stop_proxy.ps1. Run: taskkill /F /IM psynet_proxy.exe /T"
                }
                if ($port -eq 443) {
                    throw "Port 443 still in use by pid=$($c.OwningProcess) name=$name. Quit that app, then retry."
                }
            }
        }
    } catch {
        if ($_.Exception.Message -match 'Port ') { throw }
        Log "warn: port check: $($_.Exception.Message)"
    }

    Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' -Name ProxyEnable -Value 0 -ErrorAction SilentlyContinue

    Log "starting psynet_proxy.exe (hidden, no console)"
    # CREATE_NO_WINDOW so a console-subsystem Go binary never flashes a window.
    # Logging goes to psynet_proxy.log from the Go process itself.
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.WorkingDirectory = $here
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    $proc = [System.Diagnostics.Process]::Start($psi)
    if (-not $proc) { throw "failed to start psynet_proxy.exe" }
    $pidPath = Join-Path $here "proxy.pid"
    if (-not (Write-AtomicTextFile -Path $pidPath -Text "$($proc.Id)`n")) {
        Log "warn: could not write proxy.pid (locked) - stop_proxy uses process name as fallback"
    }
    Log "ok psynet_proxy started pid=$($proc.Id) (log: $(Join-Path $here 'psynet_proxy.log'))"

    Start-Sleep -Seconds 2

    function Test-Loopback443Owner {
        param([string]$WantProcess = "psynet_proxy")
        $listeners = @(Get-NetTCPConnection -LocalPort 443 -State Listen -ErrorAction SilentlyContinue |
            Where-Object { $_.LocalAddress -in @('127.0.0.1', '::1') })
        foreach ($c in $listeners) {
            $owner = Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
            $name = if ($owner) { $owner.ProcessName } else { "?" }
            if ($name -ne $WantProcess) {
                return @{ Ok = $false; Addr = $c.LocalAddress; Pid = $c.OwningProcess; Name = $name }
            }
        }
        if ($listeners.Count -eq 0) {
            return @{ Ok = $false; Addr = "none"; Pid = 0; Name = "none" }
        }
        return @{ Ok = $true }
    }

    $own = $null
    # Proxy can take several seconds to load certs + bind both 127.0.0.1 and ::1.
    for ($i = 0; $i -lt 30; $i++) {
        $own = Test-Loopback443Owner
        if ($own.Ok) { break }
        # Foreign owner of :443 — stop waiting and report generically.
        if ($own.Name -ne "none" -and $own.Name -ne "exited" -and $own.Name -ne "psynet_proxy") { break }
        $alive = $null -ne (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue)
        if (-not $alive) {
            $own = @{ Ok = $false; Addr = "none"; Pid = 0; Name = "exited" }
            break
        }
        Start-Sleep -Milliseconds 400
    }
    if (-not $own.Ok) {
        if ($own.Name -eq "exited") {
            throw "psynet_proxy exited before binding :443 (started pid=$($proc.Id)). Check psynet_proxy.log for listen errors."
        }
        if ($own.Name -eq "none") {
            throw "psynet_proxy pid=$($proc.Id) is running but loopback :443 is not bound yet — check psynet_proxy.log for [ready] listening=127.0.0.1:443 (another process may have taken the port)."
        }
        throw "Another process owns $($own.Addr):443 (pid=$($own.Pid), name=$($own.Name)), not psynet_proxy. Quit that process, then restart the VelocityRL proxy."
    }
    Log "verified: psynet_proxy owns loopback :443 (127.0.0.1 / ::1)"

    exit 0
} catch {
    $msg = "ERROR: $($_.Exception.Message)"
    try { Log $msg } catch { Write-Host $msg }
    exit 1
} finally {
    if ($script:startLockStream) {
        try { $script:startLockStream.Dispose() } catch { }
        $script:startLockStream = $null
        try { Remove-Item -LiteralPath (Join-Path $here "start_from_app.lock") -Force -ErrorAction SilentlyContinue } catch { }
    }
    Pop-Location -ErrorAction SilentlyContinue
}
