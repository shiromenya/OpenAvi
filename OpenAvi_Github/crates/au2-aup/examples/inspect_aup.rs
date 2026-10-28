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

    let legacy_paths = au2_aup::extract_media_paths_from_binary(&data);
    println!("Extracted legacy paths count: {}", legacy_paths.len());
    for p in &legacy_paths {
        println!("  extracted: {}", p);
    }

    // 0x2e77f0 付近の hex ダンプ
    if data.len() >= 0x2e7830 {
        let slice = &data[0x2e77f0..0x2e7830];
        let hex = slice.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
        let ascii = slice.iter().map(|&b| if b >= 32 && b <= 126 { b as char } else { '.' }).collect::<String>();
        println!("0x2e77f0: {} | {}", hex, ascii);
    }

    let mut audio_objects = Vec::new();
    for layer in &doc.scene.layers {
        for obj in &layer.objects {
            if obj.audio {
                audio_objects.push(obj);
            } else if let Some(src) = obj.source() {
                if matches!(src.kind, au2_core::EffectKind::AudioFile) {
                    audio_objects.push(obj);
                }
            }
        }
    }

    println!("Audio objects count: {}", audio_objects.len());
    for (idx, obj) in audio_objects.iter().take(20).enumerate() {
        println!(
            "[{}] range={}..={} audio_flag={}",
            idx, obj.frame_start, obj.frame_end, obj.audio
        );
        for eff in &obj.effects {
            println!("   eff: {} (kind={:?})", eff.name, eff.kind);
            for p in &eff.params {
                println!("     param {}: {:?}", p.key, p.value);
            }
        }
    }
}
