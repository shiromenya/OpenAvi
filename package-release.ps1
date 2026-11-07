param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
Set-Location $PSScriptRoot

$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Users\user\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin;$env:PATH"

# 1. Generate assets/openavi.ico and assets/openavi_icon.png from OpenAvi Logo
New-Item -ItemType Directory -Force -Path "assets" | Out-Null
Add-Type -AssemblyName System.Drawing

$logoCandidates = @(
    (Join-Path $PSScriptRoot "assets\openavi_logo.png"),
    (Join-Path $PSScriptRoot "assets\OpenAvi Logo.png")
)
$logoPath = $null
foreach ($cand in $logoCandidates) {
    if (Test-Path $cand) {
        $logoPath = $cand
        break
    }
}

# 256x256 正方形 PNG アイコン生成関数
function Create-SquareLogo {
    param(
        $img,
        [int]$size
    )
    $bmp = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)

    $targetW = [int]($size * 0.94)
    $targetH = [int]($targetW * ($img.Height / $img.Width))
    $x = [int](($size - $targetW) / 2)
    $y = [int](($size - $targetH) / 2)

    $g.DrawImage($img, $x, $y, $targetW, $targetH)
    $g.Dispose()
    return ,$bmp
}

$icoPath = Join-Path $PSScriptRoot "assets\openavi.ico"
if ($logoPath) {
    $srcImg = [System.Drawing.Image]::FromFile($logoPath)

    $bmp256 = Create-SquareLogo -img $srcImg -size 256
    $bmp256.Save((Join-Path $PSScriptRoot "assets\openavi_icon.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp256.Dispose()

    # マルチサイズ ICO 生成 (16, 32, 48, 64, 128, 256)
    $sizes = @(16, 32, 48, 64, 128, 256)
    $images = @()
    foreach ($s in $sizes) {
        $bmp = Create-SquareLogo -img $srcImg -size $s
        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        $images += [PSCustomObject]@{
            Size = $s
            Bytes = $ms.ToArray()
        }
        $ms.Dispose()
    }
    $srcImg.Dispose()

    $fs = [System.IO.File]::Create($icoPath)
    $bw = New-Object System.IO.BinaryWriter($fs)
    $bw.Write([uint16]0)
    $bw.Write([uint16]1)
    $bw.Write([uint16]$images.Count)
    $offset = 6 + ($images.Count * 16)
    foreach ($img in $images) {
        $dim = if ($img.Size -ge 256) { 0 } else { [byte]$img.Size }
        $bw.Write([byte]$dim)
        $bw.Write([byte]$dim)
        $bw.Write([byte]0)
        $bw.Write([byte]0)
        $bw.Write([uint16]1)
        $bw.Write([uint16]32)
        $bw.Write([uint32]$img.Bytes.Length)
        $bw.Write([uint32]$offset)
        $offset += $img.Bytes.Length
    }
    foreach ($img in $images) {
        $bw.Write($img.Bytes)
    }
    $bw.Close()
    $fs.Close()
    Write-Host "Generated multi-size icon from logo: $icoPath"
} else {
    Write-Host "Logo not found, keeping existing icon: $icoPath"
}

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
$hasDesktop = try { Test-Path $desktopDir -ErrorAction SilentlyContinue } catch { $false }
if (-not $hasDesktop) {
    $desktopDir = [Environment]::GetFolderPath("Desktop")
    $hasDesktop = try { Test-Path $desktopDir -ErrorAction SilentlyContinue } catch { $false }
}
$startPrograms = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs"

$ws = New-Object -ComObject WScript.Shell
foreach ($folder in @($desktopDir, $startPrograms)) {
    $folderExists = try { $folder -and (Test-Path $folder -ErrorAction SilentlyContinue) } catch { $false }
    if ($folderExists) {
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
if ($hasDesktop) {
    try {
        Copy-Item -Force $zipPath (Join-Path $desktopDir "OpenAvi-windows-x64.zip")

        $desktopExtract = Join-Path $desktopDir "OpenAvi-windows-x64"
        if (Test-Path $desktopExtract -ErrorAction SilentlyContinue) {
            & robocopy $stage $desktopExtract /MIR /R:1 /W:1 | Out-Null
            Write-Host "Updated desktop folder: $desktopExtract"
        }

        $githubDir = Join-Path $desktopDir "OpenAvi_Github"
        if (Test-Path $githubDir -ErrorAction SilentlyContinue) {
            & robocopy $PSScriptRoot $githubDir /MIR /XD target dist .git /XF *.pdb | Out-Null
            Write-Host "Synced source tree to: $githubDir"
        }
    } catch {
        Write-Warning "Skipped syncing to Desktop folder: $_"
    }
}

if (Test-Path "target\debug\openavi.exe" -ErrorAction SilentlyContinue) {
    Copy-Item (Join-Path $stage "openavi.exe") "target\debug\openavi.exe" -Force
}
