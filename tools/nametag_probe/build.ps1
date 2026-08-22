$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$out = Join-Path $here "nametag_probe.dll"

function Try-BuildWithCl {
    $cl = Get-Command cl.exe -ErrorAction SilentlyContinue
    if (-not $cl) { return $false }
    Push-Location $here
    try {
        cmd /c "cl /nologo /LD /O2 /EHsc nametag_probe.cpp /Fe:nametag_probe.dll /link /DEF:nametag_probe.def"
        return $LASTEXITCODE -eq 0
    } finally {
        Pop-Location
    }
}

function Try-BuildWithGpp {
    $gpp = Get-Command g++.exe -ErrorAction SilentlyContinue
    if (-not $gpp) { return $false }
    Push-Location $here
    try {
        & g++.exe -shared -O2 -o nametag_probe.dll nametag_probe.cpp -static-libgcc -static-libstdc++
        return $LASTEXITCODE -eq 0
    } finally {
        Pop-Location
    }
}

function Try-BuildWithClang {
    $clang = Get-Command clang++.exe -ErrorAction SilentlyContinue
    if (-not $clang) { return $false }
    Push-Location $here
    try {
        & clang++.exe -shared -O2 -o nametag_probe.dll nametag_probe.cpp
        return $LASTEXITCODE -eq 0
    } finally {
        Pop-Location
    }
}

if (Try-BuildWithCl) {
    Write-Host "Built $out with MSVC"
    exit 0
}
if (Try-BuildWithGpp) {
    Write-Host "Built $out with g++"
    exit 0
}
if (Try-BuildWithClang) {
    Write-Host "Built $out with clang++"
    exit 0
}

Write-Error "Could not find cl.exe or g++.exe. Install Visual Studio Build Tools or MinGW, then rerun build.ps1."
exit 1
