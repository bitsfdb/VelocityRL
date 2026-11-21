@echo off
setlocal
cd /d "%~dp0"
title VelocityRL Prepatched UPK Updater

echo =======================================================================
echo               VelocityRL Pre-patched UPK VPS Deployer
echo =======================================================================
echo.
echo Place new pre-patched UPK files (TAGame.upk / ProjectX.upk) into:
echo   prepatched_incoming\
echo.
echo The script will auto-detect game version from your local Rocket League
echo install, compute hashes, upload to VPS, and publish live to users.
echo.

python scripts\update_prepatched_vps.py %*
if %ERRORLEVEL% neq 0 (
    echo.
    echo [ERROR] Process exited with error code %ERRORLEVEL%.
) else (
    echo.
    echo [SUCCESS] VPS updated successfully!
)

echo.
pause
