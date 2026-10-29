param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
Set-Location $PSScriptRoot

$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Users\user\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin;$env:PATH"

# 1. Generate assets/openavi.ico (256x256 32-bit ICO for Windows Explorer)
New-Item -ItemType Directory -Force -Path "assets" | Out-Null
Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap(256, 256, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.Clear([System.Drawing.Color]::Transparent)

$rect = New-Object System.Drawing.Rectangle(16, 16, 224, 224)
$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    $rect,
    [System.Drawing.Color]::FromArgb(255, 26, 35, 126),
    [System.Drawing.Color]::FromArgb(255, 233, 69, 96),
    45.0
)
$g.FillEllipse($brush, $rect)

$pts = @(
    (New-Object System.Drawing.Point(96, 72)),
    (New-Object System.Drawing.Point(96, 184)),
    (New-Object System.Drawing.Point(184, 128))
)
$whiteBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
$g.FillPolygon($whiteBrush, $pts)
$g.Dispose()

$msPng = New-Object System.IO.MemoryStream
$bmp.Save($msPng, [System.Drawing.Imaging.ImageFormat]::Png)
$pngBytes = $msPng.ToArray()
$msPng.Dispose()
$bmp.Dispose()

$icoPath = Join-Path $PSScriptRoot "assets\openavi.ico"
$fs = [System.IO.File]::Create($icoPath)
$bw = New-Object System.IO.BinaryWriter($fs)
# ICONDIR (Reserved=0, Type=1, Count=1)
$bw.Write([uint16]0)
$bw.Write([uint16]1)
$bw.Write([uint16]1)
# ICONDIRENTRY (Width=0 [256], Height=0 [256], ColorCount=0, Reserved=0, Planes=1, BitCount=32, BytesInRes, ImageOffset=22)
$bw.Write([byte]0)
$bw.Write([byte]0)
$bw.Write([byte]0)
$bw.Write([byte]0)
$bw.Write([uint16]1)
$bw.Write([uint16]32)
$bw.Write([uint32]$pngBytes.Length)
$bw.Write([uint32]22)
$bw.Write($pngBytes)
$bw.Close()
$fs.Close()
Write-Host "Generated icon: $icoPath"

# 2. Build or locate binary
if (-not $SkipBuild) {
    Write-Host "Building OpenAvi..."
    cargo build --workspace
}

$exeSrc = if (Test-Path "target\release\openavi.exe") {
    "target\release\openavi.exe"
} else {
    "target\debug\openavi.exe"
}

# 古いバイナリを誤って配布しないよう、ソースより exe が古い場合は警告して中断する
$newestSrc = Get-ChildItem -Path "crates" -Recurse -Include *.rs,Cargo.toml | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($newestSrc -and (Get-Item $exeSrc).LastWriteTime -lt $newestSrc.LastWriteTime) {
    throw "Binary '$exeSrc' is older than source '$($newestSrc.FullName)'. Run 'cargo build --release -p au2-app' first."
}

# 3. Create portable package in dist/OpenAvi and dist/OpenAvi-windows-x64.zip
$stage = Join-Path $PSScriptRoot "dist\OpenAvi"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

Copy-Item $exeSrc (Join-Path $stage "openavi.exe") -Force
$stripExe = "C:\Users\user\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin\strip.exe"
if (Test-Path $stripExe) {
    & $stripExe --strip-all (Join-Path $stage "openavi.exe")
}

# 3.1 Bundle FFmpeg tools (ffmpeg.exe, ffprobe.exe) into dist/OpenAvi/bin for standalone video playback
$stageBin = Join-Path $stage "bin"
New-Item -ItemType Directory -Force -Path $stageBin | Out-Null

$ffmpegPath = $null
$ffprobePath = $null
$ffmpegCmd = Get-Command ffmpeg.exe -ErrorAction SilentlyContinue
if ($ffmpegCmd) { $ffmpegPath = $ffmpegCmd.Source }
$ffprobeCmd = Get-Command ffprobe.exe -ErrorAction SilentlyContinue
if ($ffprobeCmd) { $ffprobePath = $ffprobeCmd.Source }

if (-not $ffmpegPath -or -not (Test-Path $ffmpegPath)) {
    $wingetFfmpeg = Get-ChildItem -Path "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Recurse -Filter "ffmpeg.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($wingetFfmpeg) { $ffmpegPath = $wingetFfmpeg.FullName }
}
if (-not $ffprobePath -or -not (Test-Path $ffprobePath)) {
    $wingetFfprobe = Get-ChildItem -Path "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Recurse -Filter "ffprobe.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($wingetFfprobe) { $ffprobePath = $wingetFfprobe.FullName }
}

if ($ffmpegPath -and (Test-Path $ffmpegPath)) {
    Copy-Item $ffmpegPath (Join-Path $stageBin "ffmpeg.exe") -Force
    Write-Host "Bundled FFmpeg: $ffmpegPath -> $stageBin\ffmpeg.exe"
} else {
    Write-Warning "FFmpeg binary not found to bundle into package!"
}
if ($ffprobePath -and (Test-Path $ffprobePath)) {
    Copy-Item $ffprobePath (Join-Path $stageBin "ffprobe.exe") -Force
    Write-Host "Bundled FFprobe: $ffprobePath -> $stageBin\ffprobe.exe"
} else {
    Write-Warning "FFprobe binary not found to bundle into package!"
}

Copy-Item "assets" (Join-Path $stage "assets") -Recurse -Force
Copy-Item "samples" (Join-Path $stage "samples") -Recurse -Force
Copy-Item "plugins" (Join-Path $stage "plugins") -Recurse -Force
Copy-Item "install.bat" (Join-Path $stage "install.bat") -Force
Copy-Item "Launch-OpenAvi.bat" (Join-Path $stage "Launch-OpenAvi.bat") -Force
Copy-Item "README.md" (Join-Path $stage "README.md") -Force
Copy-Item "LICENSE" (Join-Path $stage "LICENSE") -Force

Add-Type -AssemblyName System.IO.Compression.FileSystem
$zipPath = Join-Path $PSScriptRoot "dist\OpenAvi-windows-x64.zip"
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
[System.IO.Compression.ZipFile]::CreateFromDirectory($stage, $zipPath)
Write-Host "Created portable release archive: $zipPath"

# 4. Install to %LOCALAPPDATA%\Programs\OpenAvi and create Desktop + Start Menu shortcuts
$installDir = Join-Path $env:LOCALAPPDATA "Programs\OpenAvi"
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
& robocopy $stage $installDir /MIR /R:1 /W:1 | Out-Null
Write-Host "Installed files to: $installDir"

$desktopDir = Join-Path $env:USERPROFILE "Desktop"
if (-not (Test-Path $desktopDir)) {
    $desktopDir = [Environment]::GetFolderPath("Desktop")
}
$startPrograms = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs"

$ws = New-Object -ComObject WScript.Shell
foreach ($folder in @($desktopDir, $startPrograms)) {
    if ($folder -and (Test-Path $folder)) {
        try {
            $lnkPath = Join-Path $folder "OpenAvi.lnk"
            $lnk = $ws.CreateShortcut($lnkPath)
            $lnk.TargetPath = Join-Path $installDir "openavi.exe"
            $lnk.WorkingDirectory = $installDir
            $lnk.Description = "OpenAvi - AviUtl2 (.aup2 / .exo) 互換 64bit オープンソース動画編集ソフト"
            $lnk.IconLocation = Join-Path $installDir "assets\openavi.ico"
            $lnk.Save()
            Write-Host "Created shortcut: $lnkPath"
        } catch {
            Write-Warning ("Failed to create shortcut in " + $folder)
        }
    }
}

# 5. Sync Desktop ZIP, Desktop folder, and OpenAvi_Github folder
Copy-Item -Force $zipPath (Join-Path $desktopDir "OpenAvi-windows-x64.zip")

$desktopExtract = Join-Path $desktopDir "OpenAvi-windows-x64"
if (Test-Path $desktopExtract) {
    & robocopy $stage $desktopExtract /MIR /R:1 /W:1 | Out-Null
    Write-Host "Updated desktop folder: $desktopExtract"
}

if (Test-Path "target\debug\openavi.exe") {
    Copy-Item (Join-Path $stage "openavi.exe") "target\debug\openavi.exe" -Force
}

$githubDir = Join-Path $desktopDir "OpenAvi_Github"
if (Test-Path $githubDir) {
    & robocopy $PSScriptRoot $githubDir /MIR /XD target dist .git /XF *.pdb | Out-Null
    Write-Host "Synced source tree to: $githubDir"
}
