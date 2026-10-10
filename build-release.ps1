

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$versionFile = Join-Path $PSScriptRoot "VERSION"
if (-not (Test-Path $versionFile)) {
    throw "VERSION file not found at repo root. It is the single source of truth for the app version."
}
$version = (Get-Content $versionFile -Raw).Trim()
if ($version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') {
    throw "VERSION file contains '$version' - expected a valid semver like 2.0.3 or 2.0.3-alpha.1."
}
Write-Host "==> Version: $version"

$confPath = Join-Path $PSScriptRoot "src-tauri\tauri.conf.json"
$conf = Get-Content $confPath -Raw
if ($conf -notmatch '([\r\n ]+"version": ")(\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?)(")') {
    throw "Could not find version field in tauri.conf.json."
}
if ($Matches[2] -ne $version) {
    $conf = $conf -replace '([\r\n ]+"version": ")(\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?)(")', ('${1}' + $version + '${4}')
    [System.IO.File]::WriteAllText($confPath, $conf, [System.Text.UTF8Encoding]::new($false))
    Write-Host "    tauri.conf.json: $($Matches[2]) -> $version"
}

$cargoPath = Join-Path $PSScriptRoot "src-tauri\Cargo.toml"
$cargo = Get-Content $cargoPath -Raw
if ($cargo -notmatch '(?m)^(version = ")(\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?)(")') {
    throw "Could not find version field in Cargo.toml."
}
if ($Matches[2] -ne $version) {
    $cargo = $cargo -replace '(?m)^(version = ")(\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?)(")', ('${1}' + $version + '${4}')
    [System.IO.File]::WriteAllText($cargoPath, $cargo, [System.Text.UTF8Encoding]::new($false))
    Write-Host "    Cargo.toml: $($Matches[2]) -> $version"
}

$pkgPath = Join-Path $PSScriptRoot "package.json"
if (Test-Path $pkgPath) {
    $pkg = Get-Content $pkgPath -Raw
    if ($pkg -match '("version": ")(\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?)(")') {
        if ($Matches[2] -ne $version) {
            $pkg = $pkg -replace '("version": ")(\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?)(")', ('${1}' + $version + '${4}')
            [System.IO.File]::WriteAllText($pkgPath, $pkg, [System.Text.UTF8Encoding]::new($false))
            Write-Host "    package.json: $($Matches[2]) -> $version"
        }
    }
}
# ---------------------------------------------------------------------------
# Deterministic Source Tree Hash, Build ID & Secret Generation
# ---------------------------------------------------------------------------
Write-Host "==> Computing deterministic source tree hash..."
$sha256 = [System.Security.Cryptography.SHA256]::Create()
$files = Get-ChildItem -Path (Join-Path $PSScriptRoot "src-tauri\src"), (Join-Path $PSScriptRoot "ui") -Recurse -File |
    Where-Object { $_.FullName -notmatch '[\\/](\.git|target|node_modules)[\\/]' } |
    Sort-Object FullName

$rootLen = $PSScriptRoot.Length
$hashStream = [System.IO.MemoryStream]::new()
foreach ($f in $files) {
    $rel = $f.FullName.Substring($rootLen).TrimStart('\', '/').Replace('\', '/')
    $relBytes = [System.Text.Encoding]::UTF8.GetBytes($rel)
    $hashStream.Write($relBytes, 0, $relBytes.Length)
    $fBytes = [System.IO.File]::ReadAllBytes($f.FullName)
    $hashStream.Write($fBytes, 0, $fBytes.Length)
}
$sourceHashBytes = $sha256.ComputeHash($hashStream.ToArray())
$sourceHashHex = [BitConverter]::ToString($sourceHashBytes).Replace('-', '').ToLowerInvariant()
$buildIdInt = [BitConverter]::ToInt32($sourceHashBytes, 0)
$env:VRL_BUILD_ID = "$buildIdInt"
$env:VRL_BUILD_HASH = $sourceHashHex.Substring(0, 8)

$masterSecret = "18667c8a510a5a0eb3ea0124d23f372b7387b6383a328ca4136e30f1f633997a"
$env:VRL_BUILD_SECRET = $masterSecret

Write-Host "    Source Tree Hash: $sourceHashHex"
Write-Host "    Deterministic Build ID: $env:VRL_BUILD_ID"
Write-Host "    Build Secret derived successfully."

if (Test-Path ".\.signing\build-release.ps1") {
    & .\.signing\build-release.ps1
} else {
    Write-Host "==> Running npm run tauri build..."
    npm run tauri build
}
