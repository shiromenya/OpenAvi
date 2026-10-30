use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let default_path = r"C:\Users\user\Desktop\配布　1280x720_24fps_44100Hz.aup";
    let target = args.get(1).map(|s| s.as_str()).unwrap_or(default_path);

    if !Path::new(target).exists() {
        println!("File not found: {}", target);
        return;
    }

    let data = std::fs::read(target).expect("read aup");
    let doc = au2_aup::parse_aup_bytes(&data).expect("parse aup");

    println!("=== AUP Inspection ===");
    println!("File: {}", target);
    println!("Scenes count: {}", doc.scenes.len());
    for (sc_id, sc) in &doc.scenes {
        println!(
            "  Scene {}: '{}' ({}x{} @ {:.2}fps, len={}), objects={}",
            sc_id,
            sc.name,
            sc.settings.width,
            sc.settings.height,
            sc.settings.frame_rate.as_f64(),
            sc.settings.length,
            sc.object_count()
        );
    }

    let mut eff_counts = std::collections::BTreeMap::new();
    let mut script_details = std::collections::BTreeSet::new();
    let mut special_samples = std::collections::BTreeMap::<String, Vec<String>>::new();

    for (sc_id, sc) in &doc.scenes {
        for (li, layer) in sc.layers.iter().enumerate() {
            for obj in &layer.objects {
                for eff in &obj.effects {
                    *eff_counts.entry(eff.name.clone()).or_insert(0usize) += 1;
                    if matches!(
                        eff.name.as_str(),
                        "アニメーション効果"
                            | "カスタムオブジェクト"
                            | "カメラ効果"
                            | "スクリプト(カメラ制御)"
                            | "シーンチェンジ"
                    ) {
                        script_details.insert(format!(
                            "[sc={}] {}: name={:?}, type={:?}, filter={:?}, param={:?}",
                            sc_id,
                            eff.name,
                            eff.text("name"),
                            eff.text("type"),
                            eff.text("filter"),
                            eff.text("param")
                        ));
                    }
                    if matches!(
                        eff.name.as_str(),
                        "ワイプ"
                            | "マスク"
                            | "シーンチェンジ"
                            | "オブジェクト分割"
                            | "極座標変換"
                            | "画像ループ"
                            | "ラスター"
                            | "ディスプレイスメントマップ"
                            | "シャドー(カメラ制御)"
                            | "グループ制御"
                            | "シーン"
                            | "カスタムオブジェクト"
                            | "アニメーション効果"
                            | "カメラ効果"
                            | "カメラ制御"
                            | "回転"
                            | "座標"
                            | "拡大率"
                            | "反転"
                            | "斜めクリッピング"
                            | "ルミナンスキー"
                            | "エッジ抽出"
                            | "ノイズ"
                            | "グラデーション"
                            | "テキスト"
                            | "画像ファイル"
                            | "動画ファイル"
                            | "音声ファイル"
                    ) {
                        let key = if let Some(sub) = eff.text("name").filter(|s| !s.is_empty()) {
                            format!("{}:{}", eff.name, sub)
                        } else {
                            eff.name.clone()
                        };
                        let list = special_samples.entry(key).or_default();
                        if list.len() < 4 {
                            list.push(format!(
                                "sc={} L{} ({}..={}) cat={:?} params={:?}",
                                sc_id,
                                li + 1,
                                obj.frame_start,
                                obj.frame_end,
                                obj.category(),
                                eff.params
                            ));
                        }
                    }
                }
            }
        }
    }

    println!("\n=== Effect Counts Across All Scenes ===");
    for (k, v) in &eff_counts {
        println!("  {:30}: {}", k, v);
    }

    println!("\n=== Script / SceneChange Details ===");
    for s in &script_details {
        println!("  {}", s);
    }

    println!("\n=== Special Samples ===");
    for (k, list) in &special_samples {
        println!("  [{}]:", k);
        for item in list {
            println!("    {}", item);
        }
    }

    println!("\n=== Subscene Objects Summary ===");
    for (&sc_id, sc) in &doc.scenes {
        if sc_id == 0 {
            continue;
        }
        println!("  --- Scene {} ('{}', len={}) ---", sc_id, sc.name, sc.settings.length);
        for (li, layer) in sc.layers.iter().enumerate() {
            for obj in &layer.objects {
                let eff_summary: Vec<String> = obj
                    .effects
                    .iter()
                    .map(|e| {
                        let sub = e.text("name").unwrap_or("");
                        let col = e.text("color").unwrap_or("");
                        let ty = e.text("type").unwrap_or("");
                        let sc_ref = e.text("scene").unwrap_or("");
                        format!("{}[en={},name={},ty={},col={},sc={}]", e.name, e.enabled, sub, ty, col, sc_ref)
                    })
                    .collect();
                println!(
                    "    L{:02} ({:4}..={:4}) clip={} cam={}: {}",
                    li + 1,
                    obj.frame_start,
                    obj.frame_end,
                    obj.clipping,
                    obj.camera_target,
                    eff_summary.join(" -> ")
                );
            }
        }
    }

    println!("\n=== First 40 Objects in Root Scene ===");
    for (li, layer) in doc.scene.layers.iter().enumerate() {
        for obj in &layer.objects {
            if obj.frame_start < 1500 {
                let eff_summary: Vec<String> = obj
                    .effects
                    .iter()
                    .map(|e| {
                        let sub = e.text("name").unwrap_or("");
                        let col = e.text("color").unwrap_or("");
                        let ty = e.text("type").unwrap_or("");
                        format!("{}[en={},name={},ty={},col={}]", e.name, e.enabled, sub, ty, col)
                    })
                    .collect();
                println!(
                    "  L{:02} ({:4}..={:4}) clip={} cam={}: {}",
                    li + 1,
                    obj.frame_start,
                    obj.frame_end,
                    obj.clipping,
                    obj.camera_target,
                    eff_summary.join(" -> ")
                );
            }
        }
    }
}
