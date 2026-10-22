@echo off
chcp 65001 >nul
rem Automated build script for rustBiliPC.
rem Default profile is release: gpui's Windows backend embeds HLSL bytecode only in release,
rem so a debug exe built on another machine cannot start (see docs/architecture.md section 2).
rem NOTE: keep this file ASCII-only; cmd.exe reads .bat in the OEM codepage.
setlocal
cd /d "%~dp0"

set "PROFILE=release"
if "%~1"=="" goto :profile_ok
if /I "%~1"=="release" goto :profile_ok
if /I "%~1"=="debug" (
  set "PROFILE=debug"
  goto :profile_ok
)
echo Unknown profile: %~1
echo Usage: build.bat [release^|debug]   (default: release)
exit /b 2

:profile_ok
echo [1/3] cargo build (--%PROFILE%)
if exist Cargo.lock (
  cargo build --%PROFILE% --locked
) else (
  echo Cargo.lock missing, resolving dependencies now.
  cargo build --%PROFILE%
)
if errorlevel 1 (
  echo BUILD FAILED
  exit /b 1
)

echo [2/3] cargo test (--%PROFILE%)
cargo test --%PROFILE%
if errorlevel 1 (
  echo TEST FAILED
  exit /b 1
)

echo [3/3] copy artifacts to publish\%PROFILE%
if not exist "publish\%PROFILE%" mkdir "publish\%PROFILE%"
copy /Y "target\%PROFILE%\rustbilipc.exe" "publish\%PROFILE%\" >nul 2>&1
copy /Y "target\%PROFILE%\*.dll" "publish\%PROFILE%\" >nul 2>&1

echo Done. Artifacts in publish\%PROFILE%
echo Note: a debug build is for local testing only; it will not run on another machine.
endlocal
