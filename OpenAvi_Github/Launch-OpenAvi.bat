@echo off
chcp 65001 > nul
cd /d "%~dp0"

if exist "%~dp0openavi.exe" goto :run_local
if exist "%LOCALAPPDATA%\Programs\OpenAvi\openavi.exe" goto :run_installed
goto :build

:run_local
start "" "%~dp0openavi.exe" %*
exit /b 0

:run_installed
start "" "%LOCALAPPDATA%\Programs\OpenAvi\openavi.exe" %*
exit /b 0

:build
echo ビルド済みの openavi.exe が見つからないため、ソースからビルドします...
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where cargo > nul 2>&1
if errorlevel 1 goto :no_cargo
set "CARGO_TARGET_DIR=%LOCALAPPDATA%\OpenAvi-BuildCache"
cargo build --release -p au2-app
if exist "%CARGO_TARGET_DIR%\release\openavi.exe" start "" "%CARGO_TARGET_DIR%\release\openavi.exe" %*
exit /b 0

:no_cargo
echo [エラー] openavi.exe も Rust (cargo) も見つかりません。
echo GitHub Releases の OpenAvi-windows-x64.zip をダウンロードして展開し、その中の openavi.exe を実行してください。
pause
exit /b 1
