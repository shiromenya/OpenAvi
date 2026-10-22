@echo off
chcp 65001 > nul
cd /d "%~dp0"

if exist "%~dp0openavi.exe" (
    start "" "%~dp0openavi.exe" %*
    exit /b 0
)
if exist "%~dp0target\release\openavi.exe" (
    start "" "%~dp0target\release\openavi.exe" %*
    exit /b 0
)
if exist "%~dp0target\debug\openavi.exe" (
    start "" "%~dp0target\debug\openavi.exe" %*
    exit /b 0
)

echo 初回ビルドを実行しています...
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cargo build --release
if exist "%~dp0target\release\openavi.exe" (
    start "" "%~dp0target\release\openavi.exe" %*
)
