

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

& .\.signing\build-release.ps1
