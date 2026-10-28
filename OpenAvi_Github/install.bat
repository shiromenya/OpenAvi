@echo off
chcp 65001 > nul
setlocal enabledelayedexpansion
cd /d "%~dp0"

echo ========================================================
echo   OpenAvi Installer / ワンクリック・インストーラー
echo ========================================================
echo.

REM ZIP を展開せずに実行していないか確認 (ZIP 内から実行すると同梱ファイルが読めない)
echo "%~dp0" | findstr /I /C:"\Temp\Temp" > nul
if not errorlevel 1 goto :err_zip

REM 起動中の OpenAvi があればファイルロック解除のため終了
taskkill /F /IM openavi.exe > nul 2>&1

set "INSTALL_DIR=%LOCALAPPDATA%\Programs\OpenAvi"
set "EXE_SRC="

if exist "%~dp0openavi.exe" set "EXE_SRC=%~dp0openavi.exe"
if not defined EXE_SRC if exist "%~dp0bin\openavi.exe" set "EXE_SRC=%~dp0bin\openavi.exe"
if not defined EXE_SRC if exist "%~dp0target\release\openavi.exe" set "EXE_SRC=%~dp0target\release\openavi.exe"
if not defined EXE_SRC goto :no_exe

:install
echo [1/3] インストール先を準備しています: "%INSTALL_DIR%"
if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"
if not exist "%INSTALL_DIR%" goto :err_mkdir

echo [2/3] ファイルを配置しています...
if /I not "%EXE_SRC%"=="%INSTALL_DIR%\openavi.exe" copy /Y "%EXE_SRC%" "%INSTALL_DIR%\openavi.exe" > nul
if errorlevel 1 goto :err_copy
if exist "%~dp0bin" xcopy /E /I /Y "%~dp0bin" "%INSTALL_DIR%\bin" > nul
if exist "%~dp0assets" xcopy /E /I /Y "%~dp0assets" "%INSTALL_DIR%\assets" > nul
if exist "%~dp0samples" xcopy /E /I /Y "%~dp0samples" "%INSTALL_DIR%\samples" > nul
if exist "%~dp0plugins" xcopy /E /I /Y "%~dp0plugins" "%INSTALL_DIR%\plugins" > nul
if exist "%~dp0Launch-OpenAvi.bat" copy /Y "%~dp0Launch-OpenAvi.bat" "%INSTALL_DIR%\Launch-OpenAvi.bat" > nul
if exist "%~dp0README.md" copy /Y "%~dp0README.md" "%INSTALL_DIR%\README.md" > nul
if exist "%~dp0LICENSE" copy /Y "%~dp0LICENSE" "%INSTALL_DIR%\LICENSE" > nul

echo [3/3] ショートカットを作成しています...
powershell -NoProfile -ExecutionPolicy Bypass -Command "$ws=New-Object -ComObject WScript.Shell; $d='%INSTALL_DIR%'; foreach($t in @([Environment]::GetFolderPath('Desktop'),[Environment]::GetFolderPath('Programs'))){ if($t -and (Test-Path $t)){ $l=$ws.CreateShortcut((Join-Path $t 'OpenAvi.lnk')); $l.TargetPath=(Join-Path $d 'openavi.exe'); $l.WorkingDirectory=$d; $l.Description='OpenAvi'; $i=Join-Path $d 'assets\openavi.ico'; if(Test-Path $i){ $l.IconLocation=$i }; $l.Save() } }" > nul 2>&1

echo.
echo ========================================================
echo   インストールが完了しました。
echo   デスクトップの OpenAvi アイコンから起動できます。
echo.
if not exist "%INSTALL_DIR%\bin\ffmpeg.exe" echo   [お知らせ] 動画・音声の再生には FFmpeg が必要です。
if not exist "%INSTALL_DIR%\bin\ffmpeg.exe" echo   起動後に画面上部へ表示される「FFmpeg を自動ダウンロード」を押してください。
echo ========================================================
echo.
start "" "%INSTALL_DIR%\openavi.exe"
timeout /t 3 > nul
exit /b 0

:err_zip
echo [エラー] ZIP ファイルを展開せずに実行されています。
echo         ZIP を右クリックして「すべて展開」した後、展開先フォルダ内の install.bat を実行してください。
echo.
pause
exit /b 1

:no_exe
echo [案内] このフォルダにはビルド済みの openavi.exe がありません。
echo        GitHub の Source code ZIP を展開した場合はこの状態になります。
echo        通常は GitHub Releases の OpenAvi-windows-x64.zip をダウンロードして利用してください。
echo        なお、ソースコード用の crates フォルダは実行に不要です。
echo.
where cargo > nul 2>&1
if errorlevel 1 goto :no_cargo
set /p BUILD_CONFIRM="Rust が検出されました。ソースからビルドしてインストールしますか? [Y/N]: "
if /i not "!BUILD_CONFIRM!"=="Y" goto :no_cargo
set "CARGO_TARGET_DIR=%LOCALAPPDATA%\OpenAvi-BuildCache"
cargo build --release -p au2-app
if exist "%CARGO_TARGET_DIR%\release\openavi.exe" set "EXE_SRC=%CARGO_TARGET_DIR%\release\openavi.exe"
if defined EXE_SRC goto :install
echo [エラー] ビルドに失敗しました。
pause
exit /b 1

:no_cargo
echo.
echo   [対処方法] GitHub Releases から OpenAvi-windows-x64.zip をダウンロードし、
echo   展開したフォルダ内の install.bat または openavi.exe を実行してください。
echo.
pause
exit /b 1

:err_mkdir
echo [エラー] インストール先フォルダを作成できませんでした: "%INSTALL_DIR%"
echo         セキュリティソフトやフォルダの権限をご確認ください。
echo         インストール不要で、展開フォルダ内の openavi.exe を直接実行することもできます。
echo.
pause
exit /b 1

:err_copy
echo [エラー] openavi.exe をコピーできませんでした。OpenAvi が起動中の場合は終了してから再実行してください。
echo.
pause
exit /b 1
