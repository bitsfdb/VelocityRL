

param(
    [string]$Tag,
    [string]$Out = "latest.json"
)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$versionFile = Join-Path $PSScriptRoot "VERSION"
if (-not (Test-Path $versionFile)) {
    throw "VERSION file not found at repo root."
}
$version = (Get-Content $versionFile -Raw).Trim()
if (-not $Tag) { $Tag = "v$version" }

$nsisDir = Join-Path $PSScriptRoot "src-tauri\target\release\bundle\nsis"

$setup = Get-ChildItem -Path $nsisDir -Filter "*$version*_x64-setup.exe" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) {
    $setup = Get-ChildItem -Path $nsisDir -Filter "*_x64-setup.exe" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}
if (-not $setup) {
    throw "No NSIS setup exe found in $nsisDir. Run build-release.ps1 first."
}

$sigPath = "$($setup.FullName).sig"
if (-not (Test-Path $sigPath)) {
    throw "Updater signature missing: $sigPath. The build must run with TAURI_SIGNING_PRIVATE_KEY set (build-release.ps1 handles this)."
}
$signature = (Get-Content $sigPath -Raw).Trim()

$url = "https://github.com/bitsfdb/VelocityRL/releases/download/$Tag/$($setup.Name)"

$json = [ordered]@{
    version  = $version
    notes    = "VelocityRL v$version`n`nSee the in-app What's New for the full changelog."
    pub_date = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            url       = $url
            signature = $signature
        }
    }
}
$jsonText = $json | ConvertTo-Json -Depth 5

$outPath = Join-Path $PSScriptRoot $Out
[System.IO.File]::WriteAllText($outPath, $jsonText, [System.Text.UTF8Encoding]::new($false))

Write-Host ""
Write-Host "==> Wrote $outPath"
Write-Host "    version: $version"
Write-Host "    url:     $url"
Write-Host "    sig:     $sigPath"
Write-Host ""
Write-Host "    Edit 'notes' if you want custom release notes, then upload this file"
Write-Host "    so https://api.velocityrl.tech/latest.json serves it."
