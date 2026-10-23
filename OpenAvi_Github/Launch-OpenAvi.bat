@echo off
chcp 65001 > nul
cd /d "%~dp0"

if exist "%~dp0openavi.exe" (
    start "" "%~dp0openavi.exe" %*
    exit /b 0
)

if exist "%LOCALAPPDATA%\Programs\OpenAvi\openavi.exe" (
    start "" "%LOCALAPPDATA%\Programs\OpenAvi\openavi.exe" %*
    exit /b 0
)

echo 初回ビルドを実行しています...
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
set "CARGO_TARGET_DIR=%LOCALAPPDATA%\OpenAvi-BuildCache"
cargo build --release
if exist "%CARGO_TARGET_DIR%\release\openavi.exe" (
    start "" "%CARGO_TARGET_DIR%\release\openavi.exe" %*
)
