# Build psynet_proxy.exe + MITM certs required by src-tauri/tauri.conf.json bundle.resources.
# Run before `tauri build` / release packaging. CI invokes the same steps.
#
# Why this exists:
#   server.crt/key, leaf_*.crt/key, velocityrl_ca.key, and psynet_proxy.exe are
#   gitignored. Developer machines that generated them locally work; tester
#   installs from CI without this step ship without a working MITM stack. Boot
#   then leaves config.psynet.gg -> 127.0.0.1 with nothing listening, and
#   Rocket League fails Epic Online Services / PsyNet login.
$ErrorActionPreference = "Stop"
$here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
Set-Location $here

$py = $null
foreach ($name in @("py", "python", "python3")) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if ($cmd) { $py = $cmd.Source; break }
}
if (-not $py) { throw "Python required to run gen_certs.py (pip install cryptography)" }

Write-Host "gen_certs.py via $py"
& $py .\gen_certs.py
if ($LASTEXITCODE -ne 0) { throw "gen_certs.py failed" }

$go = Get-Command go -ErrorAction SilentlyContinue
if (-not $go) { throw "Go toolchain required to build psynet_proxy.exe" }
Write-Host "go build -o psynet_proxy.exe ."
& go build -o psynet_proxy.exe .
if ($LASTEXITCODE -ne 0) { throw "go build failed" }

$required = @(
    "psynet_proxy.exe",
    "start_from_app.ps1",
    "stop_proxy.ps1",
    "velocityrl_ca.crt",
    "velocityrl_ca.key",
    "server.crt",
    "server.key",
    "leaf_api.rlpp.psynet.gg.crt",
    "leaf_api.rlpp.psynet.gg.key",
    "leaf_config.psynet.gg.crt",
    "leaf_config.psynet.gg.key",
    "leaf_ws.rlpp.psynet.gg.crt",
    "leaf_ws.rlpp.psynet.gg.key"
)
foreach ($f in $required) {
    if (-not (Test-Path -LiteralPath $f)) { throw "missing ship asset: $f" }
}
Write-Host "PsyNet ship assets OK ($($required.Count) files) in $here"
