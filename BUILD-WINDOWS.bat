@echo off
setlocal EnableExtensions
cd /d "%~dp0"

echo ============================================================
echo  AURA-PROJECT Publisher Build
echo ============================================================
echo This tool is for Aura maintainers. End users only need the
echo Aura_*_x64-setup.exe; public installers come from signed CI.
echo.

where cargo >nul 2>nul
if errorlevel 1 (
  echo [ERROR] Rust/Cargo is not installed.
  echo Install Rust from https://rustup.rs/ and reopen this script.
  echo Tauri also requires Microsoft C++ Build Tools and WebView2.
  pause
  exit /b 1
)

cargo tauri --version >nul 2>nul
if errorlevel 1 (
  echo [INFO] Installing the Tauri v2 command-line tool...
  cargo install tauri-cli --version "^2.0.0" --locked
  if errorlevel 1 goto :failed
)

echo [INFO] Verifying pinned replay map assets...
powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\VERIFY-MAP-ASSETS.ps1"
if errorlevel 1 goto :failed

set "CARGO_INCREMENTAL=1"
set "AURA_TARGET=x86_64-pc-windows-msvc"
for /f "usebackq delims=" %%V in (`powershell -NoProfile -Command "(Get-Content -Raw 'src-tauri\tauri.conf.json' | ConvertFrom-Json).version"`) do set "AURA_VERSION=%%V"
if not defined AURA_VERSION (
  echo [ERROR] Could not read the Aura version from tauri.conf.json.
  goto :failed
)
set "AURA_OUTPUT=dist\release\%AURA_VERSION%"
set "AURA_INSTALLER=Aura_%AURA_VERSION%_x64-setup.exe"
set "AURA_STAMP=src-tauri\target\%AURA_TARGET%\.aura-publisher-input.sha256"
set "AURA_INPUT_HASH="
for /f "usebackq delims=" %%H in (`powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\BUILD-FINGERPRINT.ps1"`) do set "AURA_INPUT_HASH=%%H"

if defined AURA_INPUT_HASH if exist "%AURA_STAMP%" if exist "src-tauri\target\%AURA_TARGET%\release\aura.exe" if exist "src-tauri\target\%AURA_TARGET%\release\bundle\nsis\%AURA_INSTALLER%" (
  findstr /I /X /C:"%AURA_INPUT_HASH%" "%AURA_STAMP%" >nul
  if not errorlevel 1 goto :cached_build
)

echo [INFO] Building cached, optimized Windows x64 application and NSIS installer...
cargo tauri build --target %AURA_TARGET% --bundles nsis --ci --no-sign
if errorlevel 1 goto :failed
if defined AURA_INPUT_HASH >"%AURA_STAMP%" echo %AURA_INPUT_HASH%
goto :stage_build

:cached_build
echo [INFO] Build inputs are unchanged; reusing the verified cached %AURA_VERSION% artifacts.

:stage_build

if not exist "%AURA_OUTPUT%" mkdir "%AURA_OUTPUT%"
if not exist "src-tauri\target\%AURA_TARGET%\release\aura.exe" (
  echo [ERROR] Portable executable was not produced.
  goto :failed
)
if not exist "src-tauri\target\%AURA_TARGET%\release\bundle\nsis\%AURA_INSTALLER%" (
  echo [ERROR] Expected installer %AURA_INSTALLER% was not produced.
  goto :failed
)
copy /Y "src-tauri\target\%AURA_TARGET%\release\aura.exe" "%AURA_OUTPUT%\Aura.exe" >nul
copy /Y "src-tauri\target\%AURA_TARGET%\release\bundle\nsis\%AURA_INSTALLER%" "%AURA_OUTPUT%\%AURA_INSTALLER%" >nul
powershell -NoProfile -Command "$files = @('%AURA_OUTPUT%\Aura.exe', '%AURA_OUTPUT%\%AURA_INSTALLER%'); $files | ForEach-Object { Get-FileHash -LiteralPath $_ -Algorithm SHA256 } | ForEach-Object { '{0} *{1}' -f $_.Hash.ToLowerInvariant(), (Split-Path $_.Path -Leaf) } | Set-Content -LiteralPath '%AURA_OUTPUT%\SHA256SUMS.txt' -Encoding ascii"
if errorlevel 1 goto :failed

echo.
echo [SUCCESS] Build complete.
echo Publisher output: %CD%\%AURA_OUTPUT%
echo This local installer is unsigned test output. Do not publish it.
echo End users should receive the signed installer from the release workflow.
if /I not "%AURA_NO_OPEN%"=="1" start "" "%CD%\%AURA_OUTPUT%"
exit /b 0

:failed
echo.
echo [ERROR] The build failed. Read the compiler message above.
pause
exit /b 1
