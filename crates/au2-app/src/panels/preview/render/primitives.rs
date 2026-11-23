//! 図形メッシュ・テクスチャ矩形・ポリゴン輪郭などの基本描画プリミティブ。

use eframe::egui::{self, emath::Rot2, pos2, Color32, Mesh, Pos2, Rect, Shape, Stroke};

use super::super::filters::FilterVisuals;

pub(super) fn closed_stroke(painter: &egui::Painter, pts: &[Pos2], stroke: Stroke) {
    if pts.len() < 2 {
        return;
    }
    let n = pts.len();
    for i in 0..n {
        painter.line_segment([pts[i], pts[(i + 1) % n]], stroke);
    }
}

/// 枠線（またはワイプ・マスクで中抜きされた図形）を、外周頂点列と内周頂点列を結ぶドーナツ型メッシュで描画します。
/// 3D カメラ回転時にも角の欠けや歪みが一切なく、エッジに沿った美しいグロー発光も同時に描画します。
pub(super) fn draw_shape_mesh(
    painter: &egui::Painter,
    center: Pos2,
    outer_pts: &[Pos2],
    inner_pts: Option<&[Pos2]>,
    color: Color32,
    fx: &FilterVisuals,
    scale: f32,
    alpha: f32,
) {
    if outer_pts.len() < 3 {
        return;
    }

    // 1. 頂点・エッジに沿った多層グロー（発光・拡散ブルーム）
    if fx.glow_strength > 0.01 {
        let glow_alpha = (fx.glow_strength * alpha * 42.0).clamp(8.0, 175.0) as u8;
        let base_gcol = fx.glow_color.unwrap_or(Color32::from_rgb(
            color.r().saturating_add(45),
            color.g().saturating_add(45),
            color.b().saturating_add(45),
        ));
        let gcol = Color32::from_rgba_unmultiplied(base_gcol.r(), base_gcol.g(), base_gcol.b(), glow_alpha);
        let diff_px = (fx.glow_radius * scale * 0.45).clamp(3.0, 48.0);

        // 外周と内周の輪郭線に沿って3段階の太い半透明グローストロークを重ねる
        for (mult, a_factor) in [(2.4_f32, 0.22_f32), (1.35, 0.45), (0.65, 0.75)] {
            let st = Stroke::new((diff_px * mult).max(1.5), gcol.gamma_multiply(a_factor));
            closed_stroke(painter, outer_pts, st);
            if let Some(in_pts) = inner_pts {
                closed_stroke(painter, in_pts, st);
            }
        }
    }

    if fx.glow_only {
        return;
    }

    // 2. 本体メッシュ描画（中抜きリング または 塗りつぶしファン）
    if let Some(in_pts) = inner_pts {
        if in_pts.len() == outer_pts.len() {
            let n = outer_pts.len();
            let mut mesh = Mesh::default();
            for i in 0..n {
                mesh.colored_vertex(outer_pts[i], color);
                mesh.colored_vertex(in_pts[i], color);
            }
            for i in 0..n {
                let next = (i + 1) % n;
                let o0 = (i * 2) as u32;
                let i0 = (i * 2 + 1) as u32;
                let o1 = (next * 2) as u32;
                let i1 = (next * 2 + 1) as u32;
                mesh.add_triangle(o0, o1, i0);
                mesh.add_triangle(i0, o1, i1);
            }
            painter.add(Shape::mesh(mesh));

            // 非常に細い枠線のアンチエイリアス補強として 1px ストロークを重ねる
            closed_stroke(painter, outer_pts, Stroke::new(1.0_f32, color));
            return;
        }
    }

    fan(painter, center, outer_pts, color);

    // 凸エッジ（ベベルハイライト）
    if fx.bevel_width > 0.5 {
        let bw = (fx.bevel_width * scale * 0.5).clamp(1.0, 10.0);
        closed_stroke(
            painter,
            outer_pts,
            Stroke::new(bw, Color32::from_white_alpha((alpha * 95.0) as u8)),
        );
    }
}

/// 回転・透明度付きのテクスチャ矩形を描画します。
#[allow(dead_code)]
pub(super) fn draw_textured_quad(painter: &egui::Painter, tex_id: egui::TextureId, rect: Rect, angle: f32, tint: Color32) {
    draw_textured_quad_uv(painter, tex_id, rect, angle, tint, false, false);
}

/// UV反転（左右・上下）対応のテクスチャ矩形描画。
pub(super) fn draw_textured_quad_uv(
    painter: &egui::Painter,
    tex_id: egui::TextureId,
    rect: Rect,
    angle: f32,
    tint: Color32,
    flip_h: bool,
    flip_v: bool,
) {
    let corners = rotated_rect(rect, angle);
    let (u0, u1) = if flip_h { (1.0, 0.0) } else { (0.0, 1.0) };
    let (v0, v1) = if flip_v { (1.0, 0.0) } else { (0.0, 1.0) };
    let uvs = [pos2(u0, v0), pos2(u1, v0), pos2(u1, v1), pos2(u0, v1)];
    let mut mesh = Mesh::with_texture(tex_id);
    for (p, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv, color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

pub(super) fn rotated_rect(r: Rect, angle: f32) -> Vec<Pos2> {
    let rot = Rot2::from_angle(angle);
    let c = r.center();
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
        .into_iter()
        .map(|p| c + rot * (p - c))
        .collect()
}

/// 中心から見て星形（凹形含む）の多角形を三角形ファンで塗りつぶします。
pub(super) fn fan(painter: &egui::Painter, center: Pos2, pts: &[Pos2], color: Color32) {
    if pts.len() < 3 {
        return;
    }
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    for &p in pts {
        mesh.colored_vertex(p, color);
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
}
