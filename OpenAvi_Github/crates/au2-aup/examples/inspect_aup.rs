use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let default_path = r"C:\Users\user\Desktop\goot time pf.aup";
    let target = args.get(1).map(|s| s.as_str()).unwrap_or(default_path);

    if !Path::new(target).exists() {
        println!("File not found: {}", target);
        return;
    }

    let data = std::fs::read(target).expect("read aup");
    let doc = au2_aup::parse_aup_bytes(&data).expect("parse aup");

    println!("=== AUP Inspection ===");
    println!("File: {}", target);
    println!("Version: {}", doc.version);
    println!(
        "Settings: {}x{} @ {:.2} fps, length={} frames",
        doc.scene.settings.width,
        doc.scene.settings.height,
        doc.scene.settings.frame_rate.as_f64(),
        doc.scene.settings.length,
    );
    println!("Layers: {}", doc.scene.layers.len());
    println!("Total objects: {}", doc.scene.object_count());
}
