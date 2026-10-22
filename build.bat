@echo off
chcp 65001 >nul
rem Automated build script: cargo debug build + collect artifacts into publish\
rem NOTE: keep this file (and its echoes) ASCII-only; cmd.exe reads .bat in the OEM codepage.
setlocal
cd /d "%~dp0"

set "PROFILE=debug"
set "FEATURES="

if "%1"=="release" set "PROFILE=release"
if not "%1"=="" if not "%1"=="release" if not "%1"=="debug" (
  echo Unknown profile: %1
  echo Usage: build.bat [debug^|release]
  exit /b 2
)

echo [1/3] cargo build (%PROFILE%)
if "%FEATURES%"=="" (
  cargo build --%PROFILE%
) else (
  cargo build --%PROFILE% --features "%FEATURES%"
)
if errorlevel 1 (
  echo BUILD FAILED
  exit /b 1
)

echo [2/3] run tests
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
endlocal
