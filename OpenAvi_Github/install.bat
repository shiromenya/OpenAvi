@echo off
chcp 65001 > nul
setlocal
cd /d "%~dp0"

echo ========================================================
echo   OpenAvi ワンクリック・インストーラー (Windows x64)
echo ========================================================

REM 既に起動中の OpenAvi がある場合はファイルロック解除のため終了する
taskkill /F /IM openavi.exe >nul 2>&1

set "INSTALL_DIR=%LOCALAPPDATA%\Programs\OpenAvi"
set "EXE_SRC="

if exist "%~dp0openavi.exe" (
    set "EXE_SRC=%~dp0openavi.exe"
) else (
    echo [1/3] 最新ソースコードから実行ファイルをビルド・確認しています...
    set "PATH=%USERPROFILE%\.cargo\bin;C:\Users\user\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin;%PATH%"
    set "CARGO_TARGET_DIR=%LOCALAPPDATA%\OpenAvi-BuildCache"
    cargo build --release
    if exist "%LOCALAPPDATA%\OpenAvi-BuildCache\release\openavi.exe" (
        set "EXE_SRC=%LOCALAPPDATA%\OpenAvi-BuildCache\release\openavi.exe"
    ) else if exist "%INSTALL_DIR%\openavi.exe" (
        set "EXE_SRC=%INSTALL_DIR%\openavi.exe"
    ) else (
        echo [エラー] openavi.exe が見つかりません。
        pause
        exit /b 1
    )
)

echo [2/3] インストール先: "%INSTALL_DIR%"
if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"

if /I not "%EXE_SRC%"=="%INSTALL_DIR%\openavi.exe" (
    copy /Y "%EXE_SRC%" "%INSTALL_DIR%\openavi.exe" > nul
)
if exist "%~dp0assets" xcopy /E /I /Y "%~dp0assets" "%INSTALL_DIR%\assets" > nul
if exist "%~dp0samples" xcopy /E /I /Y "%~dp0samples" "%INSTALL_DIR%\samples" > nul
if exist "%~dp0plugins" xcopy /E /I /Y "%~dp0plugins" "%INSTALL_DIR%\plugins" > nul

echo [3/3] デスクトップおよびスタートメニューにショートカットを作成しています...
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ws = New-Object -ComObject WScript.Shell; " ^
  "$installDir = '%INSTALL_DIR%'; " ^
  "$exePath = Join-Path $installDir 'openavi.exe'; " ^
  "$icoPath = Join-Path $installDir 'assets\openavi.ico'; " ^
  "foreach ($dest in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) { " ^
  "  $lnk = $ws.CreateShortcut((Join-Path $dest 'OpenAvi.lnk')); " ^
  "  $lnk.TargetPath = $exePath; " ^
  "  $lnk.WorkingDirectory = $installDir; " ^
  "  $lnk.Description = 'OpenAvi - AviUtl2 (.aup2 / .exo) 互換 64bit オープンソース動画編集ソフト'; " ^
  "  if (Test-Path $icoPath) { $lnk.IconLocation = $icoPath }; " ^
  "  $lnk.Save(); " ^
  "}"

echo.
echo ========================================================
echo   インストール完了！
echo   デスクトップの「OpenAvi」アイコンからワンクリックで起動できます。
echo ========================================================
start "" "%INSTALL_DIR%\openavi.exe"
