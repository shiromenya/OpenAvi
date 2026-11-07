//! AviUtl テキスト処理・制御タグ解析・文字レイアウト（組版）

use au2_core::Frame;
use au2_plugin::LuaObjState;
use eframe::egui::{Align2, Color32, FontId, Pos2};

use crate::panels::parse_color;
use super::render::DrawCtx;

pub(crate) fn u32_to_color32(rgb: u32) -> Color32 {
    let r = ((rgb >> 16) & 0xFF) as u8;
    let g = ((rgb >> 8) & 0xFF) as u8;
    let b = (rgb & 0xFF) as u8;
    Color32::from_rgb(r, g, b)
}

/// 固定幅（固定ピッチ）でテキストを描画するヘルパー。
/// 数字や記号がフレームごとにガタつかないように 1 文字ずつ固定ピッチで配置します。
pub fn draw_fixed_pitch_text(
    painter: &eframe::egui::Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font_id: FontId,
    color: Color32,
    pitch: f32,
) {
    let count = text.chars().count();
    if count == 0 {
        return;
    }
    let total_w = count as f32 * pitch;
    let start_x = match anchor.x() {
        eframe::emath::Align::Min => pos.x,
        eframe::emath::Align::Center => pos.x - total_w * 0.5,
        eframe::emath::Align::Max => pos.x - total_w,
    };
    for (i, ch) in text.chars().enumerate() {
        let ch_x = start_x + (i as f32 + 0.5) * pitch;
        painter.text(
            Pos2::new(ch_x, pos.y),
            Align2::CENTER_TOP,
            ch.to_string(),
            font_id.clone(),
            color,
        );
    }
}

/// AviUtl テキストの1文字レイアウト要素（キャンバス座標系におけるオブジェクト原点からのローカルオフセット）
#[derive(Debug, Clone)]
pub(crate) struct TextCharLayout {
    pub(crate) ch: String,
    pub(crate) local_x: f32,
    pub(crate) local_y: f32,
    pub(crate) color_override: Option<Color32>,
}

pub(crate) fn preprocess_aviutl2_text(raw: &str, ctx: &DrawCtx, rel: Frame) -> String {
    let mut s = raw.to_string();
    // 1. `<//コメント//>` を除去
    while let Some(start) = s.find("<//") {
        if let Some(end_rel) = s[start + 3..].find("//>") {
            s.replace_range(start..start + 3 + end_rel + 3, "");
        } else {
            break;
        }
    }
    // 2. ルビタグ `</>親文字<!>ルビ</>` の親文字を残してルビ指定タグを整形
    while let Some(start) = s.find("</>") {
        let rest = &s[start + 3..];
        if let Some(end_rel) = rest.find("</>") {
            let segment = &rest[..end_rel];
            if let Some((base_txt, _ruby_txt)) = segment.split_once("<!>") {
                let base_owned = base_txt.to_string();
                s.replace_range(start..start + 3 + end_rel + 3, &base_owned);
                continue;
            }
        }
        s.replace_range(start..start + 3, "");
    }
    // 3. 画面サイズ・時間変数タグの置換
    let sw_str = format!("{}", ctx.screen_w.round() as i64);
    let sh_str = format!("{}", ctx.screen_h.round() as i64);
    let time_sec = rel as f64 / ctx.fps.max(1.0);
    s = s
        .replace("<gw>", &sw_str)
        .replace("<gh>", &sh_str)
        .replace("<tr>", &format!("{rel}"))
        .replace("<tw>", &format!("{time_sec:.2}"))
        .replace("<th>", &format!("{:.1}", ctx.fps));
    // 4. `<?=式?>` または `<?式?>` の簡易評価
    while let Some(start) = s.find("<?") {
        if let Some(end_rel) = s[start + 2..].find("?>") {
            let inner = s[start + 2..start + 2 + end_rel].trim();
            let expr_str = inner.strip_prefix('=').unwrap_or(inner).trim();
            let mut st = LuaObjState {
                time: time_sec,
                frame: rel as i64,
                screen_w: ctx.screen_w,
                screen_h: ctx.screen_h,
                framerate: ctx.fps,
                ..Default::default()
            };
            let code = format!("return ({expr_str})");
            au2_plugin::execute_lua_script(&code, &mut st);
            let repl = if let Some(v) = st.return_value {
                if (v - v.round()).abs() < 1e-6 {
                    format!("{}", v.round() as i64)
                } else {
                    format!("{v:.2}")
                }
            } else {
                String::new()
            };
            s.replace_range(start..start + 2 + end_rel + 2, &repl);
        } else {
            break;
        }
    }
    // 5. 初代 AviUtl の Unicode 数値文字参照 (`&#12345;` / `&#x3042;`) をデコード
    let mut search_from = 0;
    while let Some(rel_pos) = s[search_from..].find("&#") {
        let start = search_from + rel_pos;
        if let Some(semi_rel) = s[start + 2..].find(';') {
            if semi_rel <= 8 {
                let code_str = s[start + 2..start + 2 + semi_rel].trim();
                let parsed_code = if let Some(hex) = code_str.strip_prefix('x').or_else(|| code_str.strip_prefix('X')) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    code_str.parse::<u32>().ok()
                };
                if let Some(ch) = parsed_code.and_then(char::from_u32) {
                    let ch_str = ch.to_string();
                    s.replace_range(start..start + 2 + semi_rel + 1, &ch_str);
                    search_from = start + ch_str.len();
                    continue;
                }
            }
        }
        search_from = start + 2;
    }
    s
}

/// AviUtl テキスト制御タグ（`<#rrggbb>`, `<s...>`, `<r...>`, `<w...>`, `<p...>`, `<@...>` 等）を解析しつつ、
/// `表示速度`・`align` (0..17 横書き/縦書き)・`spacing_x`・`spacing_y`・`monospace` に従った文字配置と全体境界を計算します。
pub(crate) fn layout_aviutl_text(
    ctx: &DrawCtx,
    src: &au2_core::Effect,
    rel: Frame,
) -> (Vec<TextCharLayout>, f32) {
    let raw_input = src.text("text").unwrap_or_default().replace("\r\n", "\n");
    let raw_text = preprocess_aviutl2_text(&raw_input, ctx, rel);
    if raw_text.is_empty() {
        return (Vec::new(), 0.0);
    }

    let raw_size = src.eval("サイズ", rel, 34.0).max(1.0) as f32;
    let speed = src.eval("表示速度", rel, 0.0);
    let time_sec = rel as f64 / ctx.fps.max(1.0);
    let max_visible_chars = if speed > 0.0 {
        Some((time_sec * speed).floor().max(0.0) as usize)
    } else {
        None
    };

    let align = src
        .text("align")
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(0);
    let spacing_x = src
        .text("spacing_x")
        .and_then(|s| s.trim().parse::<i32>().ok())
        .map(|v| (v as i8) as f32)
        .unwrap_or(0.0);
    let spacing_y = src
        .text("spacing_y")
        .and_then(|s| s.trim().parse::<i32>().ok())
        .map(|v| (v as i8) as f32)
        .unwrap_or(0.0);
    let monospace = src
        .text("monospace")
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(0)
        != 0;

    // 制御タグ (<#rrggbb> 等) の解析と行分割
    let mut lines: Vec<Vec<(char, Option<Color32>)>> = vec![Vec::new()];
    let mut current_color: Option<Color32> = None;
    let mut visible_count = 0usize;
    let chars: Vec<char> = raw_text.chars().collect();
    let mut idx = 0;
    while idx < chars.len() {
        if let Some(limit) = max_visible_chars {
            if visible_count >= limit {
                break;
            }
        }
        let c = chars[idx];
        if c == '<' {
            if let Some(close_rel) = chars[idx + 1..].iter().position(|&x| x == '>') {
                let tag: String = chars[idx + 1..idx + 1 + close_rel].iter().collect();
                let tag_trim = tag.trim();
                if let Some(hex) = tag_trim.strip_prefix('#') {
                    if hex.is_empty() {
                        current_color = None;
                    } else if hex.len() >= 6 {
                        if let Some(col) = parse_color(&hex[..6]) {
                            current_color = Some(col);
                        }
                    }
                    idx += close_rel + 2;
                    continue;
                } else if tag_trim.starts_with('s')
                    || tag_trim.starts_with('r')
                    || tag_trim.starts_with('w')
                    || tag_trim.starts_with('c')
                    || tag_trim.starts_with('p')
                    || tag_trim.starts_with('@')
                    || tag_trim == "/"
                    || tag_trim == "!"
                {
                    idx += close_rel + 2;
                    continue;
                }
            }
        }
        if c == '\n' {
            lines.push(Vec::new());
        } else {
            if let Some(last_line) = lines.last_mut() {
                last_line.push((c, current_color));
            }
            visible_count += 1;
        }
        idx += 1;
    }

    // 末尾の空行が改行由来でない場合は維持しつつ、各文字のキャンバス幅を算出
    let measure_font_sz = 32.0_f32;
    let char_canvas_width = |ch: char| -> f32 {
        if monospace {
            return raw_size;
        }
        if ch == ' ' {
            return raw_size * 0.35;
        }
        if ch == '　' {
            return raw_size * 0.92;
        }
        let g = ctx.painter.layout_no_wrap(
            ch.to_string(),
            FontId::proportional(measure_font_sz),
            Color32::WHITE,
        );
        let ratio = (g.size().x / measure_font_sz).clamp(0.3, 1.25);
        raw_size * ratio
    };

    let mut out = Vec::new();
    let mut max_bottom_y: f32;

    if align < 9 {
        // 横書き (align 0..8)
        let h_align = align % 3; // 0: 左寄, 1: 中央, 2: 右寄
        let v_align = (align / 3) % 3; // 0: 上, 1: 中, 2: 下
        let num_lines = lines.len().max(1);
        let line_step_y = (raw_size + spacing_y).max(1.0);
        let total_h = num_lines as f32 * raw_size + (num_lines.saturating_sub(1)) as f32 * spacing_y;
        let start_y = match v_align {
            0 => 0.0,
            1 => -total_h * 0.5,
            _ => -total_h,
        };
        max_bottom_y = start_y + total_h;

        for (row_idx, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let widths: Vec<f32> = line.iter().map(|&(ch, _)| char_canvas_width(ch)).collect();
            let line_w: f32 = widths.iter().sum::<f32>()
                + (line.len().saturating_sub(1) as f32) * spacing_x;
            let start_x = match h_align {
                0 => 0.0,
                1 => -line_w * 0.5,
                _ => -line_w,
            };
            let cy = start_y + row_idx as f32 * line_step_y + raw_size * 0.5;
            let mut cursor_x = start_x;
            for ((ch, col_ov), w) in line.iter().zip(widths.into_iter()) {
                let cx = cursor_x + w * 0.5;
                out.push(TextCharLayout {
                    ch: ch.to_string(),
                    local_x: cx,
                    local_y: cy,
                    color_override: *col_ov,
                });
                cursor_x += w + spacing_x;
            }
        }
    } else {
        // 縦書き (align 9..17)
        let v_align = (align - 9) % 3; // 0: 上寄, 1: 中央, 2: 下寄
        let h_align = ((align - 9) / 3) % 3; // 0: 右, 1: 中, 2: 左
        let num_cols = lines.len().max(1);
        let col_step_x = (raw_size + spacing_x).max(1.0);
        let char_step_y = (raw_size + spacing_y).max(1.0);
        let total_w = num_cols as f32 * raw_size + (num_cols.saturating_sub(1)) as f32 * spacing_x;
        let col0_cx = match h_align {
            0 => -raw_size * 0.5,
            1 => total_w * 0.5 - raw_size * 0.5,
            _ => total_w - raw_size * 0.5,
        };
        max_bottom_y = raw_size * 0.5;

        for (col_idx, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let n_chars = line.len();
            let col_h = n_chars as f32 * raw_size + (n_chars.saturating_sub(1)) as f32 * spacing_y;
            let start_y = match v_align {
                0 => 0.0,
                1 => -col_h * 0.5,
                _ => -col_h,
            };
            max_bottom_y = max_bottom_y.max(start_y + col_h);
            let cx = col0_cx - col_idx as f32 * col_step_x;
            for (char_row, (ch, col_ov)) in line.iter().enumerate() {
                let cy = start_y + char_row as f32 * char_step_y + raw_size * 0.5;
                out.push(TextCharLayout {
                    ch: ch.to_string(),
                    local_x: cx,
                    local_y: cy,
                    color_override: *col_ov,
                });
            }
        }
    }

    (out, max_bottom_y)
}

