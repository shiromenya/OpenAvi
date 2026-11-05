//! # au2-platform
//!
//! OS 依存の処理をこのクレートに集約します。
//! 他のクレートは `USERPROFILE` / `LOCALAPPDATA` / `.exe` / `cfg(windows)` を直接扱わず、
//! ここの関数を経由してください（クロスプラットフォーム対応の窓口）。

use std::path::PathBuf;
use std::process::Command;

/// ユーザーのホームディレクトリ（Windows: `USERPROFILE`、その他: `HOME`）。
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// デスクトップフォルダ（`<home>/Desktop`）。
pub fn desktop_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join("Desktop"))
}

/// アプリのユーザーデータ領域の親ディレクトリ。
/// Windows: `%LOCALAPPDATA%` / macOS: `~/Library/Application Support` / その他: `$XDG_DATA_HOME` または `~/.local/share`。
pub fn user_data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    }
    if cfg!(target_os = "macos") {
        return home_dir().map(|h| h.join("Library").join("Application Support"));
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".local").join("share")))
}

/// 実行ファイル名に OS 固有の拡張子を付けます（Windows: `ffmpeg` → `ffmpeg.exe`）。
pub fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// 子プロセス起動時にコンソールウィンドウが一瞬表示されないようにします（Windows のみ有効、他 OS では何もしません）。
pub fn hide_console_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_name_matches_platform() {
        if cfg!(windows) {
            assert_eq!(exe_name("ffmpeg"), "ffmpeg.exe");
        } else {
            assert_eq!(exe_name("ffmpeg"), "ffmpeg");
        }
    }

    #[test]
    fn desktop_is_under_home() {
        if let (Some(h), Some(d)) = (home_dir(), desktop_dir()) {
            assert!(d.starts_with(h));
        }
    }
}
