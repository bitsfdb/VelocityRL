@echo off
REM Build VelocityReplaySync.dll (x64 BakkesMod plugin)
REM Requires: VS 2022 Build Tools + BakkesMod SDK
REM Matches tools\bm-product-dumper\build.bat structure (two-phase cl + link).

setlocal

set MSVC=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207
set WINSDK_INC=C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0
set WINSDK_LIB=C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0
set BM_SDK=%APPDATA%\bakkesmod\bakkesmod\bakkesmodsdk

set CL_EXE="%MSVC%\bin\Hostx64\x64\cl.exe"
set LINK_EXE="%MSVC%\bin\Hostx64\x64\link.exe"

set INCLUDE_DIRS=/I"%MSVC%\include" /I"%WINSDK_INC%\ucrt" /I"%WINSDK_INC%\um" /I"%WINSDK_INC%\shared" /I"%BM_SDK%\include" 

set LIB_DIRS=/LIBPATH:"%MSVC%\lib\x64" /LIBPATH:"%WINSDK_LIB%\ucrt\x64" /LIBPATH:"%WINSDK_LIB%\um\x64" /LIBPATH:"%BM_SDK%\lib"

set SRC_DIR=%~dp0
set OUT_DIR=%SRC_DIR%build

if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"

echo === Compiling pch.cpp (x64) ===
%CL_EXE% /nologo /c /EHsc /std:c++17 /MD /O2 /DWIN32 /D_WINDOWS /D_USRDLL %INCLUDE_DIRS% /Yc"pch.h" /Fp"%OUT_DIR%\pch.pch" /Fo"%OUT_DIR%\pch.obj" "%SRC_DIR%pch.cpp"
if errorlevel 1 goto :error

echo === Compiling VelocityReplaySync.cpp (x64) ===
%CL_EXE% /nologo /c /EHsc /std:c++17 /MD /O2 /DWIN32 /D_WINDOWS /D_USRDLL %INCLUDE_DIRS% /Yu"pch.h" /Fp"%OUT_DIR%\pch.pch" /Fo"%OUT_DIR%\VelocityReplaySync.obj" "%SRC_DIR%VelocityReplaySync.cpp"
if errorlevel 1 goto :error

echo === Linking VelocityReplaySync.dll (x64) ===
%LINK_EXE% /nologo /DLL /MACHINE:X64 /OUT:"%OUT_DIR%\VelocityReplaySync.dll" %LIB_DIRS% "%OUT_DIR%\pch.obj" "%OUT_DIR%\VelocityReplaySync.obj" pluginsdk.lib ws2_32.lib shell32.lib ole32.lib
if errorlevel 1 goto :error

echo.
echo === SUCCESS! ===
echo DLL: %OUT_DIR%\VelocityReplaySync.dll
echo.
echo To install, copy to: %APPDATA%\bakkesmod\bakkesmod\plugins\
exit /b 0

:error
echo.
echo Build FAILED.
exit /b 1
