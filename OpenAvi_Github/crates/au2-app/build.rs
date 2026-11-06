use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../assets/openavi.ico");
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico_path = manifest_dir.join("../../assets/openavi.ico");
    if !ico_path.exists() {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc_path = out_dir.join("openavi.rc");
    let obj_path = out_dir.join("openavi_res.o");

    // Windows リソーススクリプトの作成 (スラッシュ区切りの絶対パス)
    let ico_path_str = ico_path.to_string_lossy().replace('\\', "/");
    let rc_content = format!("1 ICON \"{ico_path_str}\"\n");
    if std::fs::write(&rc_path, rc_content).is_err() {
        return;
    }

    // windres コマンドの実行 (MinGW / GCC ツールチェーン)
    let status = Command::new("windres")
        .arg("-i")
        .arg(&rc_path)
        .arg("-o")
        .arg(&obj_path)
        .arg("-O")
        .arg("coff")
        .status();

    if let Ok(st) = status {
        if st.success() {
            println!("cargo:rustc-link-arg={}", obj_path.display());
        }
    }
}
