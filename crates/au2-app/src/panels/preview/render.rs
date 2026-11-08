//! オブジェクト・シェイプ・画像・動画・フィルタの合成レンダリング

use std::f32::consts::{PI, TAU};

use au2_core::{EffectKind, Frame, ObjectCategory, ObjectId, TimelineObject};
use au2_plugin::{LuaLoadedPrimitive, LuaObjState, PluginRegistry};
use eframe::egui::{self, emath::Rot2, pos2, vec2, Align2, Color32, FontId, Mesh, Pos2, Rect, Shape, Stroke};

use crate::panels::parse_color;
use super::camera::{apply_camera_effect, aviutl_interpolation, aviutl_rand, project_3d_points, rotate_3d, Camera3D, CameraParams};
use super::filters::FilterVisuals;
use super::text::{layout_aviutl_text, u32_to_color32, TextCharLayout};

#[derive(Clone, Copy, Debug)]
pub(crate) struct GroupTransform {
    pub(crate) ox: f64,
    pub(crate) oy: f64,
    pub(crate) oz: f64,
    pub(crate) zoom: f64,
    pub(crate) rx: f64,
    pub(crate) ry: f64,
    pub(crate) rz: f64,
    pub(crate) alpha: f64,
}

impl Default for GroupTransform {
    fn default() -> Self {
        Self {
            ox: 0.0,
            oy: 0.0,
            oz: 0.0,
            zoom: 1.0,
            rx: 0.0,
            ry: 0.0,
            rz: 0.0,
            alpha: 1.0,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct DrawCtx<'a> {
    pub(crate) painter: &'a egui::Painter,
    pub(crate) canvas: Rect,
    pub(crate) scale: f32,
    pub(crate) fps: f64,
    pub(crate) screen_w: f64,
    pub(crate) screen_h: f64,
    pub(crate) textures: &'a std::collections::HashMap<ObjectId, (egui::TextureId, [usize; 2])>,
    pub(crate) plugins: &'a PluginRegistry,
    pub(crate) scenes: &'a std::collections::BTreeMap<u32, au2_core::Scene>,
    pub(crate) has_ffmpeg: bool,
    pub(crate) camera: Option<Camera3D>,
    pub(crate) group_transform: Option<GroupTransform>,
    pub(crate) scene_depth: u32,
}

/// 指定したシーン (`scene`) の `frame` における描画を実行します（サブシーン呼び出し・カメラ制御・グループ制御・時間制御・クリッピング対応）。
pub(crate) fn render_scene_at(
    base_ctx: &DrawCtx,
    scene: &au2_core::Scene,
    frame: Frame,
    selected_id: Option<ObjectId>,
) {
    if base_ctx.scene_depth > 4 {
        return;
    }

    // 1. 時間制御 (TimeControl) の評価: (start_layer_idx, end_layer_idx_inclusive, mapped_rel_frame)
    let mut time_overrides: Vec<(usize, Option<usize>, Frame)> = Vec::new();
    for (layer_idx, layer) in scene.layers.iter().enumerate() {
        if !layer.visible {
            continue;
        }
        if let Some(obj) = layer.objects.iter().find(|o| o.contains(frame)) {
            if let Some(tc) = obj.effect(&EffectKind::TimeControl) {
                if tc.enabled {
                    let rel = frame - obj.frame_start;
                    let pos = tc.eval("位置", rel, 0.0);
                    let range = tc
                        .text("range")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let mapped_frame = (obj.frame_start as f64 + pos.max(0.0)).round() as Frame;
                    let end_layer = if range == 0 {
                        None
                    } else {
                        Some(layer_idx + range)
                    };
                    time_overrides.push((layer_idx + 1, end_layer, mapped_frame));
                }
            }
        }
    }

    let resolve_layer_frame = |layer_idx: usize| -> Frame {
        let mut f = frame;
        for &(start_l, end_l, mapped_f) in &time_overrides {
            if layer_idx >= start_l && end_l.map(|e| layer_idx <= e).unwrap_or(true) {
                f = mapped_f;
            }
        }
        f
    };

    // 2. カメラ制御 (Camera) の評価
    let mut active_camera: Option<(usize, Camera3D)> = None;
    for (layer_idx, layer) in scene.layers.iter().enumerate() {
        if !layer.visible {
            continue;
        }
        let lf = resolve_layer_frame(layer_idx);
        if let Some(obj) = layer.objects.iter().find(|o| o.contains(lf)) {
            if let Some(cam_eff) = obj.effect(&EffectKind::Camera) {
                if cam_eff.enabled {
                    let rel = lf - obj.frame_start;
                    let cam_x = cam_eff.eval("X", rel, 0.0);
                    let cam_y = cam_eff.eval("Y", rel, 0.0);
                    let cam_z = cam_eff.eval("Z", rel, -1024.0);
                    let cam_tx = cam_eff.eval("目標X", rel, 0.0);
                    let cam_ty = cam_eff.eval("目標Y", rel, 0.0);
                    let cam_tz = cam_eff.eval("目標Z", rel, 0.0);
                    let cam_fov = cam_eff.eval("視野角", rel, 0.0);
                    let init_d = if cam_fov.abs() > 0.1 {
                        let base_fov = (base_ctx.screen_h / (1024.0 * 2.0)).atan() * 360.0 / std::f64::consts::PI;
                        let eff_fov = if cam_fov > 0.0 && cam_fov < 30.0 {
                            cam_fov.clamp(1.0, 150.0)
                        } else {
                            (base_fov + cam_fov).clamp(1.0, 150.0)
                        };
                        base_ctx.screen_h / ((eff_fov * std::f64::consts::PI / 360.0).tan() * 2.0)
                    } else {
                        1024.0
                    };
                    let mut cp = CameraParams {
                        x: cam_x,
                        y: cam_y,
                        z: cam_z,
                        tx: cam_tx,
                        ty: cam_ty,
                        tz: cam_tz,
                        rz: cam_eff.eval("傾き", rel, 0.0),
                        ux: 0.0,
                        uy: -1.0,
                        uz: 0.0,
                        d: init_d,
                    };

                    for eff in &obj.effects {
                        if !eff.enabled || matches!(eff.kind, EffectKind::Camera) {
                            continue;
                        }
                        let script_name = eff.text("name").unwrap_or(&eff.name);
                        let time_sec = rel as f64 / base_ctx.fps.max(1.0);
                        let total_sec = obj.length() as f64 / base_ctx.fps.max(1.0);

                        let mut lua_cam = LuaObjState {
                            x: cp.x,
                            y: cp.y,
                            z: cp.z,
                            cam_tx: cp.tx,
                            cam_ty: cp.ty,
                            cam_tz: cp.tz,
                            rz: cp.rz,
                            cam_ux: cp.ux,
                            cam_uy: cp.uy,
                            cam_uz: cp.uz,
                            cam_d: cp.d,
                            time: time_sec,
                            totaltime: total_sec,
                            frame: rel as i64,
                            totalframe: obj.length() as i64,
                            layer: (layer_idx + 1) as i64,
                            screen_w: base_ctx.screen_w,
                            screen_h: base_ctx.screen_h,
                            framerate: base_ctx.fps,
                            ..Default::default()
                        };
                        if base_ctx.plugins.execute_effect(eff, rel, &mut lua_cam) {
                            cp.x = lua_cam.x + lua_cam.ox;
                            cp.y = lua_cam.y + lua_cam.oy;
                            cp.z = lua_cam.z + lua_cam.oz;
                            cp.tx = lua_cam.cam_tx;
                            cp.ty = lua_cam.cam_ty;
                            cp.tz = lua_cam.cam_tz;
                            cp.rz = lua_cam.rz;
                            cp.ux = lua_cam.cam_ux;
                            cp.uy = lua_cam.cam_uy;
                            cp.uz = lua_cam.cam_uz;
                            cp.d = lua_cam.cam_d;
                        } else {
                            let t0 = eff.eval("track0", rel, 0.0);
                            let t1 = eff.eval("track1", rel, 0.0);
                            let t2 = eff.eval("track2", rel, 0.0);
                            let t3 = eff.eval("track3", rel, 0.0);
                            apply_camera_effect(&mut cp, script_name, t0, t1, t2, t3, time_sec, base_ctx.screen_h);
                        }
                    }

                    active_camera = Some((layer_idx, Camera3D::from_params(&cp, cam_fov)));
                    break;
                }
            }
        }
    }

    // 3. グループ制御 (GroupControl) の評価
    let mut group_overrides: Vec<(usize, Option<usize>, bool, Option<u32>, GroupTransform)> = Vec::new();
    for (layer_idx, layer) in scene.layers.iter().enumerate() {
        if !layer.visible {
            continue;
        }
        let lf = resolve_layer_frame(layer_idx);
        if let Some(obj) = layer.objects.iter().find(|o| o.contains(lf)) {
            if let Some(gc) = obj.effect(&EffectKind::GroupControl) {
                if gc.enabled {
                    let rel = lf - obj.frame_start;
                    let range = gc
                        .text("range")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let end_layer = if range == 0 {
                        None
                    } else {
                        Some(layer_idx + range)
                    };
                    let inherit_upper = gc.eval("上位グループ制御の影響を受ける", rel, gc.eval("check0", rel, 0.0)) >= 0.5;
                    let same_group_only = gc.eval("同じグループのオブジェクトを対象にする", rel, gc.eval("check1", rel, 0.0)) >= 0.5;
                    let req_group = if same_group_only && obj.group.unwrap_or(0) != 0 {
                        obj.group
                    } else {
                        None
                    };

                    let mut gt = GroupTransform {
                        ox: gc.eval("X", rel, 0.0),
                        oy: gc.eval("Y", rel, 0.0),
                        oz: gc.eval("Z", rel, 0.0),
                        zoom: (gc.eval("拡大率", rel, 100.0) / 100.0).max(0.0),
                        rx: gc.eval("X軸回転", rel, 0.0),
                        ry: gc.eval("Y軸回転", rel, 0.0),
                        rz: gc.eval("Z軸回転", rel, 0.0),
                        alpha: 1.0,
                    };

                    // グループ制御オブジェクトに付随するアニメーション効果やフェード等も評価
                    let time_sec = rel as f64 / base_ctx.fps.max(1.0);
                    let total_sec = obj.length() as f64 / base_ctx.fps.max(1.0);
                    let mut lua_st = LuaObjState {
                        x: gt.ox,
                        y: gt.oy,
                        z: gt.oz,
                        rx: gt.rx,
                        ry: gt.ry,
                        rz: gt.rz,
                        zoom: 1.0,
                        alpha: 1.0,
                        time: time_sec,
                        totaltime: total_sec,
                        frame: rel as i64,
                        totalframe: obj.length() as i64,
                        layer: (layer_idx + 1) as i64,
                        screen_w: base_ctx.screen_w,
                        screen_h: base_ctx.screen_h,
                        framerate: base_ctx.fps,
                        ..Default::default()
                    };
                    for eff in &obj.effects {
                        if !eff.enabled || matches!(eff.kind, EffectKind::GroupControl) {
                            continue;
                        }
                        if eff.name == "フェード" {
                            let fin = eff.eval("イン", rel, 0.0) * base_ctx.fps;
                            let fout = eff.eval("アウト", rel, 0.0) * base_ctx.fps;
                            let remain = (obj.length() - 1 - rel) as f64;
                            if fin > 0.0 {
                                lua_st.alpha *= (rel as f64 / fin).min(1.0);
                            }
                            if fout > 0.0 {
                                lua_st.alpha *= (remain / fout).min(1.0);
                            }
                        } else if eff.name == "座標" {
                            lua_st.ox += eff.eval("X", rel, 0.0);
                            lua_st.oy += eff.eval("Y", rel, 0.0);
                            lua_st.oz += eff.eval("Z", rel, 0.0);
                        } else if eff.name == "回転" {
                            lua_st.rx += eff.eval("X", rel, 0.0);
                            lua_st.ry += eff.eval("Y", rel, 0.0);
                            lua_st.rz += eff.eval("Z", rel, 0.0);
                        } else if eff.name == "拡大率" || eff.name == "リサイズ" {
                            lua_st.zoom *= (eff.eval("拡大率", rel, 100.0) / 100.0).max(0.0);
                        } else {
                            let _ = base_ctx.plugins.execute_effect(eff, rel, &mut lua_st);
                        }
                    }
                    gt.ox = lua_st.x + lua_st.ox - lua_st.cx;
                    gt.oy = lua_st.y + lua_st.oy - lua_st.cy;
                    gt.oz = lua_st.z + lua_st.oz - lua_st.cz;
                    gt.zoom *= lua_st.zoom.max(0.0);
                    gt.rx = lua_st.rx;
                    gt.ry = lua_st.ry;
                    gt.rz = lua_st.rz;
                    gt.alpha = lua_st.alpha.clamp(0.0, 1.0);

                    group_overrides.push((layer_idx + 1, end_layer, inherit_upper, req_group, gt));
                }
            }
        }
    }

    let resolve_group_transform = |layer_idx: usize, obj_group: Option<u32>| -> GroupTransform {
        let mut combined = base_ctx.group_transform.unwrap_or_default();
        for &(start_l, end_l, inherit_upper, req_group, ref gt) in &group_overrides {
            if layer_idx >= start_l && end_l.map(|e| layer_idx <= e).unwrap_or(true) {
                if let Some(rg) = req_group {
                    if obj_group != Some(rg) {
                        continue;
                    }
                }
                if !inherit_upper {
                    combined = *gt;
                } else {
                    let rotated = rotate_3d([combined.ox, combined.oy, combined.oz], gt.rx, gt.ry, gt.rz);
                    combined.ox = gt.ox + rotated[0] * gt.zoom;
                    combined.oy = gt.oy + rotated[1] * gt.zoom;
                    combined.oz = gt.oz + rotated[2] * gt.zoom;
                    combined.zoom *= gt.zoom;
                    combined.rx += gt.rx;
                    combined.ry += gt.ry;
                    combined.rz += gt.rz;
                    combined.alpha *= gt.alpha;
                }
            }
        }
        combined
    };

    // 4. 描画対象オブジェクトの収集と 3群ソート（2D背景 / 3Dカメラ対象 Z降順 / 2D前面）
    let mut render_items: Vec<(u8, f64, usize, &TimelineObject, Frame, GroupTransform)> = Vec::new();
    let cam_layer = active_camera.map(|(l, _)| l).unwrap_or(0);

    for (layer_idx, layer) in scene.layers.iter().enumerate() {
        if !layer.visible {
            continue;
        }
        let lf = resolve_layer_frame(layer_idx);
        if let Some(obj) = layer.objects.iter().find(|o| o.contains(lf)) {
            if matches!(
                obj.source().map(|s| &s.kind),
                Some(EffectKind::Camera | EffectKind::GroupControl | EffectKind::TimeControl)
            ) {
                continue;
            }
            let rel = lf - obj.frame_start;
            let gt = resolve_group_transform(layer_idx, obj.group);
            let pass = if active_camera.is_some() && obj.camera_target {
                1u8
            } else if layer_idx < cam_layer {
                0u8
            } else {
                2u8
            };

            let depth = if let Some((_, ref cam)) = active_camera {
                if obj.camera_target {
                    let draw = obj
                        .effect(&EffectKind::StandardDraw)
                        .or_else(|| obj.effect(&EffectKind::ExtendedDraw));
                    let ev = |key: &str, default: f64| draw.map(|d| d.eval(key, rel, default)).unwrap_or(default);
                    let p_rot = rotate_3d(
                        [ev("X", 0.0) * gt.zoom, ev("Y", 0.0) * gt.zoom, ev("Z", 0.0) * gt.zoom],
                        gt.rx,
                        gt.ry,
                        gt.rz,
                    );
                    let wx = gt.ox + p_rot[0];
                    let wy = gt.oy + p_rot[1];
                    let wz = gt.oz + p_rot[2];
                    cam.project([wx, wy, wz]).map(|(_, _, _, zc)| zc).unwrap_or(f64::MAX)
                } else {
                    0.0
                }
            } else {
                0.0
            };

            render_items.push((pass, depth, layer_idx, obj, rel, gt));
        }
    }

    render_items.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| {
            if a.0 == 1 {
                b.1.total_cmp(&a.1).then_with(|| a.2.cmp(&b.2))
            } else {
                a.2.cmp(&b.2)
            }
        })
    });

    // 5. 「上のオブジェクトでクリッピング (obj.clipping)」の領域追跡を行いながら描画
    let mut last_non_clipping_bounds: Option<Rect> = None;

    for (_, _, layer_idx, obj, rel, gt) in render_items {
        let clipped_painter = if obj.clipping {
            if let Some(clip_r) = last_non_clipping_bounds {
                Some(base_ctx.painter.with_clip_rect(clip_r.intersect(base_ctx.canvas)))
            } else {
                None
            }
        } else {
            None
        };

        let item_ctx = DrawCtx {
            painter: clipped_painter.as_ref().unwrap_or(base_ctx.painter),
            canvas: base_ctx.canvas,
            scale: base_ctx.scale,
            fps: base_ctx.fps,
            screen_w: base_ctx.screen_w,
            screen_h: base_ctx.screen_h,
            textures: base_ctx.textures,
            plugins: base_ctx.plugins,
            scenes: base_ctx.scenes,
            has_ffmpeg: base_ctx.has_ffmpeg,
            camera: active_camera.map(|(_, c)| c).or(base_ctx.camera),
            group_transform: Some(gt),
            scene_depth: base_ctx.scene_depth,
        };

        let drawn_bounds = draw_object(&item_ctx, layer_idx, obj, rel, selected_id == Some(obj.id));
        if !obj.clipping {
            last_non_clipping_bounds = drawn_bounds;
        }
    }
}

fn closed_stroke(painter: &egui::Painter, pts: &[Pos2], stroke: Stroke) {
    if pts.len() < 2 {
        return;
    }
    let n = pts.len();
    for i in 0..n {
        painter.line_segment([pts[i], pts[(i + 1) % n]], stroke);
    }
}

fn draw_object(ctx: &DrawCtx, layer_idx: usize, obj: &TimelineObject, rel: Frame, selected: bool) -> Option<Rect> {
    let Some(src) = obj.source() else { return None };
    if !src.enabled {
        return None;
    }
    match obj.category() {
        ObjectCategory::Media => {}
        ObjectCategory::Filter => {
            draw_filter_object(ctx, obj, rel);
            return Some(ctx.canvas);
        }
        _ => return None,
    }

    // テキストの「文字毎に個別オブジェクト (`check0` = 1)」が有効な場合、
    // 各文字を個別のサブオブジェクト (`obj.index = 0..N-1`, `obj.num = N`) としてエフェクト評価・描画する
    if matches!(src.kind, EffectKind::Text) {
        let is_split = src.eval("文字毎に個別オブジェクト", rel, src.eval("check0", rel, 0.0)) >= 0.5;
        if is_split {
            let (chars_layout, _) = layout_aviutl_text(ctx, src, rel);
            if chars_layout.is_empty() {
                return None;
            }
            let display_on_moving = src.eval("移動座標上に表示する", rel, src.eval("check1", rel, 0.0)) >= 0.5;
            let raw_size = src.eval("サイズ", rel, 34.0).max(1.0);
            let total_num = chars_layout.len() as i64;
            let mut combined: Option<Rect> = None;
            for (idx, item) in chars_layout.iter().enumerate() {
                if item.ch == " " || item.ch == "　" {
                    continue;
                }
                let init_ox = if display_on_moving { 0.0 } else { item.local_x as f64 };
                let init_oy = if display_on_moving { 0.0 } else { item.local_y as f64 };
                let single_char = TextCharLayout {
                    ch: item.ch.clone(),
                    local_x: 0.0,
                    local_y: 0.0,
                    color_override: item.color_override,
                };
                if let Some(b) = draw_object_sub(
                    ctx,
                    layer_idx,
                    obj,
                    src,
                    rel,
                    idx as i64,
                    total_num,
                    init_ox,
                    init_oy,
                    raw_size,
                    Some(&single_char),
                    false,
                ) {
                    combined = Some(combined.map(|prev| prev.union(b)).unwrap_or(b));
                }
            }
            if selected {
                if let Some(b) = combined {
                    ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
                }
            }
            return combined;
        }
    }

    let approx_size = src.eval("サイズ", rel, 100.0).max(16.0);
    draw_object_sub(
        ctx,
        layer_idx,
        obj,
        src,
        rel,
        0,
        1,
        0.0,
        0.0,
        approx_size,
        None,
        selected,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_object_sub(
    ctx: &DrawCtx,
    layer_idx: usize,
    obj: &TimelineObject,
    src: &au2_core::Effect,
    rel: Frame,
    sub_index: i64,
    sub_num: i64,
    init_ox: f64,
    init_oy: f64,
    approx_size: f64,
    single_text_char: Option<&TextCharLayout>,
    selected: bool,
) -> Option<Rect> {
    // 標準描画 / 拡張描画パラメータ
    let draw = obj
        .effect(&EffectKind::StandardDraw)
        .or_else(|| obj.effect(&EffectKind::ExtendedDraw));
    let ev = |key: &str, default: f64| draw.map(|d| d.eval(key, rel, default)).unwrap_or(default);
    let base_x = ev("X", 0.0);
    let base_y = ev("Y", 0.0);
    let base_z = ev("Z", 0.0);
    let base_zoom = ev("拡大率", 100.0) / 100.0;
    let base_rot = ev("回転", 0.0);
    let base_alpha = (1.0 - ev("透明度", 0.0) / 100.0).clamp(0.0, 1.0);
    let base_rx = ev("X軸回転", 0.0);
    let base_ry = ev("Y軸回転", 0.0);
    let base_rz = if base_rot != 0.0 { base_rot } else { ev("Z軸回転", 0.0) };
    let base_cx = ev("中心X", 0.0);
    let base_cy = ev("中心Y", 0.0);
    let base_cz = ev("中心Z", 0.0);

    let time_sec = rel as f64 / ctx.fps.max(1.0);
    let total_sec = obj.length() as f64 / ctx.fps.max(1.0);

    // Lua スクリプト実行ステートの初期化
    let mut lua_st = LuaObjState {
        x: base_x,
        y: base_y,
        z: base_z,
        ox: init_ox,
        oy: init_oy,
        rx: base_rx,
        ry: base_ry,
        rz: base_rz,
        cx: base_cx,
        cy: base_cy,
        cz: base_cz,
        zoom: 1.0,
        alpha: base_alpha,
        w: approx_size,
        h: approx_size,
        time: time_sec,
        totaltime: total_sec,
        frame: rel as i64,
        totalframe: obj.length() as i64,
        layer: (layer_idx + 1) as i64,
        index: sub_index,
        num: sub_num,
        screen_w: ctx.screen_w,
        screen_h: ctx.screen_h,
        framerate: ctx.fps,
        ..Default::default()
    };

    let mut fx = FilterVisuals::default();
    if let Some(b) = draw.and_then(|d| d.text("blend")).and_then(|b| b.parse::<i32>().ok()) {
        fx.blend_mode = b;
    }

    // フィルタ効果・スクリプト (.anm / .obj / .cam / .scn) および内蔵エフェクトの評価
    for eff in &obj.effects {
        if !eff.enabled {
            continue;
        }
        if matches!(
            eff.kind,
            EffectKind::StandardDraw | EffectKind::ExtendedDraw | EffectKind::StandardPlayback
        ) {
            continue;
        }

        // 1) プラグインレジストリ内の Lua スクリプト (.anm/.obj/.cam/.scn) を実行
        let is_builtin_filter = matches!(
            eff.name.as_str(),
            "フェード"
                | "振動"
                | "縁取り"
                | "シャドー"
                | "ドロップシャドウ"
                | "単色化"
                | "グロー"
                | "発光"
                | "拡散光"
                | "閃光"
                | "色ズレ"
                | "ワイプ"
                | "マスク"
                | "クリッピング"
                | "斜めクリッピング"
                | "色調補正"
                | "グラデーション"
                | "凸エッジ"
                | "ミラー"
                | "リサイズ"
                | "ローテーション"
                | "反転"
                | "極座標変換"
                | "ディスプレイスメントマップ"
                | "ノイズ"
                | "モザイク"
                | "ぼかし"
                | "方向ブラー"
                | "レンズブラー"
                | "放射ブラー"
                | "座標"
                | "拡大率"
                | "透明度"
                | "回転"
                | "エッジ抽出"
                | "ルミナンスキー"
        ) && eff.text("name").is_none();

        let ran_lua = if is_builtin_filter {
            false
        } else {
            ctx.plugins.execute_effect(eff, rel, &mut lua_st)
        };

        // 2) 内蔵フィルタの視覚効果評価
        match eff.name.as_str() {
            "フェード" => {
                let fin = eff.eval("イン", rel, 0.0) * ctx.fps;
                let fout = eff.eval("アウト", rel, 0.0) * ctx.fps;
                let remain = (obj.length() - 1 - rel) as f64;
                if fin > 0.0 {
                    lua_st.alpha *= (rel as f64 / fin).clamp(0.0, 1.0);
                }
                if fout > 0.0 {
                    lua_st.alpha *= (remain / fout).clamp(0.0, 1.0);
                }
            }
            "振動" => {
                let ax = eff.eval("X", rel, 15.0);
                let ay = eff.eval("Y", rel, 15.0);
                let az = eff.eval("Z", rel, 0.0);
                let period = eff.eval("周期", rel, 1.0).max(0.1);
                let phase = (rel as f64 / period) * (TAU as f64);
                lua_st.ox += phase.sin() * ax;
                lua_st.oy += (phase * 1.3).cos() * ay;
                lua_st.oz += (phase * 0.7).sin() * az;
            }
            "リサイズ" => {
                let z = eff.eval("拡大率", rel, 100.0) / 100.0;
                lua_st.zoom *= z.max(0.0);
            }
            "ローテーション" => {
                let deg = eff.eval("90度回転", rel, 0.0);
                lua_st.rz += deg * 90.0;
            }
            "グロー" | "発光" | "拡散光" | "閃光" => {
                let strength = eff.eval("強さ", rel, 60.0) as f32;
                let diffusion = eff.eval("拡散", rel, 30.0) as f32;
                let blur = eff.eval("ぼかし", rel, 1.0) as f32;
                fx.glow_strength = (fx.glow_strength + strength / 85.0).min(8.0);
                fx.glow_radius = (fx.glow_radius + diffusion + blur * 0.5).max(8.0);
                if let Some(c) = eff.text("color").and_then(parse_color) {
                    fx.glow_color = Some(c);
                }
                if eff.eval("光成分のみ", rel, eff.eval("check0", rel, 0.0)) >= 0.5 {
                    fx.glow_only = true;
                }
            }
            "ワイプ" => {
                // AviUtl のワイプ: イン(秒) で 0→100% 出現、アウト(秒) で中央からくり抜かれて消滅（または枠線化）
                let win = eff.eval("イン", rel, 0.0) * ctx.fps;
                let wout = eff.eval("アウト", rel, 0.0) * ctx.fps;
                let remain = (obj.length() - 1 - rel) as f64;
                let rev_in = eff.eval("反転(イン)", rel, eff.eval("check0", rel, 0.0)) >= 0.5;
                let rev_out = eff.eval("反転(アウト)", rel, eff.eval("check1", rel, 0.0)) >= 0.5;

                // イン進行度 (0.0 -> 1.0)
                if win > 0.0 && (rel as f64) < win {
                    let p = ((rel as f64 + 0.5) / win).clamp(0.0, 1.0) as f32;
                    if rev_in {
                        fx.inner_hollow_ratio = fx.inner_hollow_ratio.max(1.0 - p);
                    } else {
                        lua_st.zoom *= p as f64;
                    }
                }
                // アウト進行度: 残り時間が wout 以下のとき、中央からくり抜いて枠線（フレーム）のように消えていく
                if wout > 0.0 && remain < wout {
                    let p = (1.0 - (remain / wout).clamp(0.0, 1.0)) as f32;
                    if rev_out {
                        lua_st.zoom *= (1.0 - p) as f64;
                    } else {
                        fx.inner_hollow_ratio = fx.inner_hollow_ratio.max(p.clamp(0.0, 0.98));
                    }
                }
            }
            "マスク" => {
                let m_size = eff.eval("サイズ", rel, 100.0) as f32;
                let invert = eff.eval("マスクの反転", rel, eff.eval("check0", rel, 0.0)) >= 0.5;
                let fit = eff.eval("元のサイズに合わせる", rel, eff.eval("check1", rel, 0.0)) >= 0.5;
                let obj_sz = approx_size as f32;
                if invert {
                    // マスクの反転: 図形の中央をくり抜いて中抜きフレーム（枠線）にする
                    let ratio = if fit {
                        (m_size / 100.0).clamp(0.0, 0.98)
                    } else if obj_sz > 1.0 {
                        (m_size / obj_sz).clamp(0.0, 0.98)
                    } else {
                        0.75
                    };
                    fx.inner_hollow_ratio = fx.inner_hollow_ratio.max(ratio);
                } else if !fit && obj_sz > 1.0 && m_size < obj_sz {
                    let clip_half = ((obj_sz - m_size) * 0.5).max(0.0);
                    for inset in &mut fx.clip_insets {
                        *inset = (*inset).max(clip_half);
                    }
                }
            }
            "クリッピング" => {
                let top = eff.eval("上", rel, 0.0) as f32;
                let bottom = eff.eval("下", rel, 0.0) as f32;
                let left = eff.eval("左", rel, 0.0) as f32;
                let right = eff.eval("右", rel, 0.0) as f32;
                fx.clip_insets[0] += top.max(0.0);
                fx.clip_insets[1] += bottom.max(0.0);
                fx.clip_insets[2] += left.max(0.0);
                fx.clip_insets[3] += right.max(0.0);
                if eff.eval("中心の位置を変更", rel, eff.eval("check0", rel, 0.0)) >= 0.5 {
                    lua_st.ox += ((left - right) * 0.5) as f64;
                    lua_st.oy += ((top - bottom) * 0.5) as f64;
                }
            }
            "斜めクリッピング" => {
                let width = eff.eval("幅", rel, 0.0) as f32;
                if width < 0.0 {
                    // 負の幅は帯状の切り抜き
                    let band = (-width).max(2.0);
                    let cut = ((approx_size as f32 - band) * 0.5).max(0.0);
                    fx.clip_insets[0] += cut;
                    fx.clip_insets[1] += cut;
                }
            }
            "色調補正" | "拡張色調補正" => {
                let br = (eff.eval("明るさ", rel, 100.0) as f32 - 100.0) / 100.0;
                let hue = eff.eval("色相", rel, 0.0) as f32;
                let sat = (eff.eval("彩度", rel, 100.0) as f32 / 100.0).max(0.0);
                fx.hsv_shift.0 += br;
                fx.hsv_shift.1 += hue;
                fx.hsv_shift.2 = if fx.hsv_shift.2 == 0.0 { sat } else { fx.hsv_shift.2 * sat };
            }
            "グラデーション" | "グラデーションマップ" => {
                let strength = (eff.eval("強さ", rel, 100.0) / 100.0).clamp(0.0, 1.0) as f32;
                let col1 = eff
                    .text("color")
                    .or_else(|| eff.text("開始色"))
                    .or_else(|| eff.text("色1"))
                    .and_then(parse_color);
                let col2 = eff
                    .text("color2")
                    .or_else(|| eff.text("終了色"))
                    .or_else(|| eff.text("色2"))
                    .and_then(parse_color);
                if let Some(c) = col1.or(col2) {
                    fx.grad_color = Some((c, strength * 0.65));
                }
            }
            "凸エッジ" => {
                let w = eff.eval("幅", rel, 5.0) as f32;
                fx.bevel_width = w.max(1.0);
            }
            "ミラー" => {
                let tr = eff.eval("透明度", rel, 30.0) as f32;
                fx.mirror_alpha = (1.0 - tr / 100.0).clamp(0.0, 1.0);
            }
            s if s.contains("色収差") || s.contains("色ズレ") => {
                let shift = eff.eval("ズレ幅", rel, 8.0) as f32;
                let ang = (eff.eval("角度", rel, 0.0) as f32).to_radians();
                fx.chroma_shift = shift;
                fx.chroma_angle = ang;
            }
            "縁取り" => {
                fx.border_width = eff.eval("サイズ", rel, 4.0) as f32;
                fx.border_color = eff
                    .text("color")
                    .or_else(|| eff.text("縁色"))
                    .and_then(parse_color)
                    .or(Some(Color32::WHITE));
            }
            "シャドー" | "ドロップシャドウ" => {
                let dx = eff.eval("X", rel, 6.0) as f32;
                let dy = eff.eval("Y", rel, 6.0) as f32;
                let density = (eff.eval("濃さ", rel, 60.0) / 100.0).clamp(0.0, 1.0) as f32;
                fx.shadow_offset = Some(vec2(dx, dy));
                fx.shadow_alpha = density;
            }
            "単色化" => {
                let strength = (eff.eval("強さ", rel, 100.0) / 100.0).clamp(0.0, 1.0) as f32;
                let col = eff
                    .text("color")
                    .or_else(|| eff.text("色"))
                    .and_then(parse_color)
                    .unwrap_or(Color32::from_rgb(255, 200, 120));
                fx.tint_color = Some((col, strength));
            }
            "座標" => {
                lua_st.ox += eff.eval("X", rel, 0.0);
                lua_st.oy += eff.eval("Y", rel, 0.0);
                lua_st.oz += eff.eval("Z", rel, 0.0);
            }
            "拡大率" => {
                let z = eff.eval("拡大率", rel, 100.0) / 100.0;
                let sx = eff.eval("X", rel, 100.0) / 100.0;
                let sy = eff.eval("Y", rel, 100.0) / 100.0;
                lua_st.zoom *= (z * ((sx + sy) * 0.5)).max(0.0);
            }
            "透明度" => {
                let tr = (eff.eval("透明度", rel, 0.0) / 100.0).clamp(0.0, 1.0);
                lua_st.alpha *= 1.0 - tr;
            }
            "回転" => {
                lua_st.rx += eff.eval("X", rel, eff.eval("X軸回転", rel, 0.0));
                lua_st.ry += eff.eval("Y", rel, eff.eval("Y軸回転", rel, 0.0));
                lua_st.rz += eff.eval("Z", rel, eff.eval("Z軸回転", rel, 0.0));
            }
            "極座標変換" => {
                let cw = eff.eval("中心幅", rel, 0.0) as f32;
                let zm = (eff.eval("拡大率", rel, 100.0) / 100.0).max(0.05) as f32;
                let rot = eff.eval("回転", rel, 0.0) as f32;
                let swirl = eff.eval("渦巻", rel, 0.0) as f32;
                fx.polar_transform = Some((cw, zm, rot, swirl));
            }
            "画像ループ" => {
                let nx = (eff.eval("横回数", rel, 1.0).round() as u32).clamp(1, 16);
                let ny = (eff.eval("縦回数", rel, 1.0).round() as u32).clamp(1, 16);
                let sx = eff.eval("速度X", rel, 0.0) as f32;
                let sy = eff.eval("速度Y", rel, 0.0) as f32;
                if nx > 1 || ny > 1 || sx.abs() > 0.1 || sy.abs() > 0.1 {
                    fx.image_loop = Some((nx, ny, sx, sy));
                }
            }
            "オブジェクト分割" => {
                let nx = (eff.eval("横分割数", rel, 1.0).round() as u32).clamp(1, 16);
                let ny = (eff.eval("縦分割数", rel, 1.0).round() as u32).clamp(1, 16);
                if nx > 1 || ny > 1 {
                    fx.object_split = Some((nx, ny));
                }
            }
            "ラスター" => {
                let rw = eff.eval("横幅", rel, 100.0) as f32;
                let rh = eff.eval("高さ", rel, 100.0) as f32;
                let rp = eff.eval("周期", rel, 1.0) as f32;
                fx.raster = Some((rw, rh, rp));
                let phase = (time_sec as f32) * rp.max(0.1) * TAU;
                lua_st.ox += (phase.sin() * rw * 0.15) as f64;
            }
            "ノイズ" => {
                let ns = (eff.eval("強さ", rel, 50.0) / 100.0).clamp(0.0, 1.0) as f32;
                fx.noise_strength = fx.noise_strength.max(ns);
            }
            "ぼかし" | "レンズブラー" | "放射ブラー" | "方向ブラー" | "モーションブラー" => {
                let rng = eff.eval("範囲", rel, eff.eval("間隔", rel, 5.0)) as f32;
                if rng > 0.5 {
                    fx.glow_strength = (fx.glow_strength + (rng / 60.0).min(1.5)).min(8.0);
                    fx.glow_radius = (fx.glow_radius + rng * 0.8).max(6.0);
                }
            }
            "反転" => {
                let fv = eff.eval("上下反転", rel, eff.eval("check0", rel, 0.0)) >= 0.5;
                let fh = eff.eval("左右反転", rel, eff.eval("check1", rel, 0.0)) >= 0.5;
                let il = eff.eval("輝度反転", rel, eff.eval("check2", rel, 0.0)) >= 0.5;
                let ih = eff.eval("色相反転", rel, eff.eval("check3", rel, 0.0)) >= 0.5;
                let ia = eff.eval("透明度反転", rel, eff.eval("check4", rel, 0.0)) >= 0.5;
                if fv {
                    lua_st.rx += 180.0;
                }
                if fh {
                    lua_st.ry += 180.0;
                }
                fx.inversions = (
                    fx.inversions.0 || fv,
                    fx.inversions.1 || fh,
                    fx.inversions.2 || il,
                    fx.inversions.3 || ih,
                    fx.inversions.4 || ia,
                );
            }
            "エッジ抽出" => {
                let str_val = eff.eval("強さ", rel, 100.0) as f32;
                if str_val > 10.0 {
                    fx.inner_hollow_ratio = fx.inner_hollow_ratio.max(0.85);
                    if let Some(c) = eff.text("color").and_then(parse_color) {
                        fx.tint_color = Some((c, 0.9));
                    }
                }
            }
            "ルミナンスキー" => {
                let thresh = eff.eval("基準輝度", rel, 2000.0) as f32;
                if thresh > 3800.0 {
                    lua_st.alpha *= 0.85;
                }
            }
            "ディスプレイスメントマップ" => {
                let dx = eff.eval("変形X", rel, 0.0);
                let dy = eff.eval("変形Y", rel, 0.0);
                let shift = (dx.hypot(dy) * 0.15) as f32;
                if shift > 0.5 {
                    fx.chroma_shift = fx.chroma_shift.max(shift.min(18.0));
                }
            }
            _ => {
                if !ran_lua && eff.name == "アニメーション効果" {
                    let script_name = eff.text("name").unwrap_or_default();
                    let sec = script_name.split('@').next().unwrap_or(script_name);
                    match sec {
                        "揺れ" | "振動" | "震える" => {
                            let ax = eff.eval("X振幅", rel, eff.eval("track0", rel, 25.0));
                            let ay = eff.eval("Y振幅", rel, eff.eval("track1", rel, 15.0));
                            let interval = eff.eval("間隔", rel, eff.eval("track2", rel, 0.0));
                            let angle = eff.eval("角度", rel, eff.eval("track1", rel, 0.0));
                            if interval <= 0.0 {
                                lua_st.ox += aviutl_rand(-ax, ax, 0, (time_sec * 100.0) as i64);
                                lua_st.oy += aviutl_rand(-ay, ay, 1, (time_sec * 100.0) as i64);
                            } else {
                                let t = time_sec * 100.0 / interval;
                                let p = t.floor() as i64;
                                let t_frac = t - (p as f64);
                                let x0 = aviutl_rand(-1000.0, 1000.0, 0, p) / 1000.0 * ax;
                                let x1 = aviutl_rand(-1000.0, 1000.0, 0, p + 1) / 1000.0 * ax;
                                let x2 = aviutl_rand(-1000.0, 1000.0, 0, p + 2) / 1000.0 * ax;
                                let x3 = aviutl_rand(-1000.0, 1000.0, 0, p + 3) / 1000.0 * ax;

                                let y0 = aviutl_rand(-1000.0, 1000.0, 1, p) / 1000.0 * ay;
                                let y1 = aviutl_rand(-1000.0, 1000.0, 1, p + 1) / 1000.0 * ay;
                                let y2 = aviutl_rand(-1000.0, 1000.0, 1, p + 2) / 1000.0 * ay;
                                let y3 = aviutl_rand(-1000.0, 1000.0, 1, p + 3) / 1000.0 * ay;

                                lua_st.ox += aviutl_interpolation(t_frac, x0, x1, x2, x3);
                                lua_st.oy += aviutl_interpolation(t_frac, y0, y1, y2, y3);
                                if angle != 0.0 {
                                    let r0 = aviutl_rand(-1000.0, 1000.0, 2, p) / 1000.0 * angle;
                                    let r1 = aviutl_rand(-1000.0, 1000.0, 2, p + 1) / 1000.0 * angle;
                                    let r2 = aviutl_rand(-1000.0, 1000.0, 2, p + 2) / 1000.0 * angle;
                                    let r3 = aviutl_rand(-1000.0, 1000.0, 2, p + 3) / 1000.0 * angle;
                                    lua_st.rz += aviutl_interpolation(t_frac, r0, r1, r2, r3);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Lua スクリプト内で `obj.effect(...)` が呼ばれた場合の反映
    for fcall in &lua_st.filter_calls {
        match fcall.name.as_str() {
            "グロー" | "発光" | "拡散光" | "閃光" => {
                let s = fcall.params.get("強さ").map(|v| v.as_num() as f32).unwrap_or(60.0);
                let d = fcall.params.get("拡散").map(|v| v.as_num() as f32).unwrap_or(30.0);
                fx.glow_strength = (fx.glow_strength + s / 85.0).min(8.0);
                fx.glow_radius = (fx.glow_radius + d).max(8.0);
                if let Some(c) = fcall.params.get("color").map(|v| u32_to_color32(v.as_num() as u32)) {
                    fx.glow_color = Some(c);
                }
            }
            "縁取り" => {
                fx.border_width = fcall.params.get("サイズ").map(|v| v.as_num() as f32).unwrap_or(4.0);
                fx.border_color = fcall
                    .params
                    .get("color")
                    .map(|v| u32_to_color32(v.as_num() as u32))
                    .or(Some(Color32::WHITE));
            }
            "色ズレ" | "色収差" => {
                fx.chroma_shift = fcall.params.get("ズレ幅").map(|v| v.as_num() as f32).unwrap_or(8.0);
                if let Some(ang) = fcall.params.get("角度") {
                    fx.chroma_angle = (ang.as_num() as f32).to_radians();
                }
            }
            "ぼかし" | "レンズブラー" | "放射ブラー" | "方向ブラー" => {
                let rng = fcall.params.get("範囲").map(|v| v.as_num() as f32).unwrap_or(10.0);
                if rng > 0.5 {
                    fx.glow_strength = (fx.glow_strength + (rng / 60.0).min(1.5)).min(8.0);
                    fx.glow_radius = (fx.glow_radius + rng * 0.8).max(6.0);
                }
            }
            "色調補正" | "拡張色調補正" => {
                let br = (fcall.params.get("明るさ").map(|v| v.as_num() as f32).unwrap_or(100.0) - 100.0) / 100.0;
                let hue = fcall.params.get("色相").map(|v| v.as_num() as f32).unwrap_or(0.0);
                let sat = (fcall.params.get("彩度").map(|v| v.as_num() as f32).unwrap_or(100.0) / 100.0).max(0.0);
                fx.hsv_shift.0 += br;
                fx.hsv_shift.1 += hue;
                fx.hsv_shift.2 = if fx.hsv_shift.2 == 0.0 { sat } else { fx.hsv_shift.2 * sat };
            }
            "シャドー" | "ドロップシャドウ" => {
                let dx = fcall.params.get("X").map(|v| v.as_num() as f32).unwrap_or(6.0);
                let dy = fcall.params.get("Y").map(|v| v.as_num() as f32).unwrap_or(6.0);
                let density = (fcall.params.get("濃さ").map(|v| v.as_num() as f32).unwrap_or(60.0) / 100.0).clamp(0.0, 1.0);
                fx.shadow_offset = Some(vec2(dx, dy));
                fx.shadow_alpha = density;
            }
            "単色化" => {
                let strength = (fcall.params.get("強さ").map(|v| v.as_num() as f32).unwrap_or(100.0) / 100.0).clamp(0.0, 1.0);
                let col = fcall
                    .params
                    .get("color")
                    .map(|v| u32_to_color32(v.as_num() as u32))
                    .unwrap_or(Color32::from_rgb(255, 200, 120));
                fx.tint_color = Some((col, strength));
            }
            "ミラー" => {
                let tr = fcall.params.get("透明度").map(|v| v.as_num() as f32).unwrap_or(30.0);
                fx.mirror_alpha = (1.0 - tr / 100.0).clamp(0.0, 1.0);
            }
            "ラスター" => {
                let rw = fcall.params.get("横幅").map(|v| v.as_num() as f32).unwrap_or(50.0);
                let rh = fcall.params.get("高さ").map(|v| v.as_num() as f32).unwrap_or(50.0);
                let rp = fcall.params.get("周期").map(|v| v.as_num() as f32).unwrap_or(1.0);
                fx.raster = Some((rw, rh, rp));
            }
            "ノイズ" => {
                let ns = (fcall.params.get("強さ").map(|v| v.as_num() as f32).unwrap_or(40.0) / 100.0).clamp(0.0, 1.0);
                fx.noise_strength = fx.noise_strength.max(ns);
            }
            "極座標変換" => {
                let cw = fcall.params.get("中心幅").map(|v| v.as_num() as f32).unwrap_or(0.0);
                let zm = (fcall.params.get("拡大率").map(|v| v.as_num() as f32).unwrap_or(100.0) / 100.0).max(0.05);
                let rot = fcall.params.get("回転").map(|v| v.as_num() as f32).unwrap_or(0.0);
                let swirl = fcall.params.get("渦巻").map(|v| v.as_num() as f32).unwrap_or(2.0);
                fx.polar_transform = Some((cw, zm, rot, swirl));
            }
            "クリッピング" => {
                fx.clip_insets[0] += fcall.params.get("上").map(|v| v.as_num() as f32).unwrap_or(0.0).max(0.0);
                fx.clip_insets[1] += fcall.params.get("下").map(|v| v.as_num() as f32).unwrap_or(0.0).max(0.0);
                fx.clip_insets[2] += fcall.params.get("左").map(|v| v.as_num() as f32).unwrap_or(0.0).max(0.0);
                fx.clip_insets[3] += fcall.params.get("右").map(|v| v.as_num() as f32).unwrap_or(0.0).max(0.0);
            }
            "マスク" => {
                let sz = fcall.params.get("サイズ").map(|v| v.as_num() as f32).unwrap_or(100.0);
                let inv = fcall.params.get("マスクの反転").map(|v| v.as_num()).unwrap_or(0.0) >= 0.5;
                if inv {
                    fx.inner_hollow_ratio = fx.inner_hollow_ratio.max((sz / (approx_size as f32).max(1.0)).clamp(0.1, 0.96));
                }
            }
            "扇クリッピングR" => {
                let end_deg = fcall.params.get("終了角").map(|v| v.as_num() as f32).unwrap_or(360.0);
                let start_deg = fcall.params.get("開始角").map(|v| v.as_num() as f32).unwrap_or(0.0);
                fx.sector_clip = Some((start_deg, end_deg));
            }
            _ => {}
        }
    }

    if lua_st.aspect.abs() > 0.001 {
        fx.aspect_override = lua_st.aspect as f32;
    }

    let gt = ctx.group_transform.unwrap_or_default();
    let alpha = (lua_st.alpha * gt.alpha).clamp(0.0, 1.0) as f32;
    if alpha <= 0.0 && lua_st.draw_calls.is_empty() && lua_st.poly_calls.is_empty() {
        return None;
    }

    // グループ制御の 3D 回転・スケーリング・平行移動を合成
    let local_pos = [
        (base_x + lua_st.ox - lua_st.cx) * gt.zoom,
        (base_y + lua_st.oy - lua_st.cy) * gt.zoom,
        (base_z + lua_st.oz - lua_st.cz) * gt.zoom,
    ];
    let rot_pos = rotate_3d(local_pos, gt.rx, gt.ry, gt.rz);
    let raw_x = gt.ox + rot_pos[0];
    let raw_y = gt.oy + rot_pos[1];
    let raw_z = gt.oz + rot_pos[2];
    let raw_zoom = (base_zoom * lua_st.zoom * gt.zoom).max(0.0);
    let raw_rx = lua_st.rx + gt.rx;
    let raw_ry = lua_st.ry + gt.ry;
    let raw_rz = lua_st.rz + gt.rz;

    let (x, y, zoom, angle) = if obj.camera_target {
        if let Some(ref cam) = ctx.camera {
            if let Some((proj_x, proj_y, scale_3d, _)) = cam.project([raw_x, raw_y, raw_z]) {
                (
                    proj_x,
                    proj_y,
                    (raw_zoom * scale_3d as f64) as f32,
                    ((raw_rz - cam.roll) as f32).to_radians(),
                )
            } else {
                return None;
            }
        } else {
            (
                raw_x as f32,
                raw_y as f32,
                raw_zoom as f32,
                (raw_rz as f32).to_radians(),
            )
        }
    } else {
        (
            raw_x as f32,
            raw_y as f32,
            raw_zoom as f32,
            (raw_rz as f32).to_radians(),
        )
    };

    // `obj.drawpoly(...)` が呼ばれている場合はポリゴン群を描画
    let mut poly_bounds: Option<Rect> = None;
    for poly in &lua_st.poly_calls {
        let pts: Vec<Pos2> = poly
            .pts
            .iter()
            .map(|&(px, py, pz)| {
                let p_rot = rotate_3d([px * gt.zoom, py * gt.zoom, pz * gt.zoom], raw_rx, raw_ry, raw_rz);
                let wx = raw_x + p_rot[0];
                let wy = raw_y + p_rot[1];
                let wz = raw_z + p_rot[2];
                if obj.camera_target {
                    if let Some(ref cam) = ctx.camera {
                        if let Some((proj_x, proj_y, _, _)) = cam.project([wx, wy, wz]) {
                            return ctx.canvas.center() + vec2(proj_x, proj_y) * ctx.scale;
                        }
                    }
                }
                ctx.canvas.center() + vec2(wx as f32, wy as f32) * ctx.scale
            })
            .collect();
        let col = poly
            .color
            .map(u32_to_color32)
            .or_else(|| src.text("color").and_then(parse_color))
            .unwrap_or(Color32::WHITE)
            .gamma_multiply((poly.alpha as f32 * alpha).clamp(0.0, 1.0));
        let c = Pos2::new(
            pts.iter().map(|p| p.x).sum::<f32>() / 4.0,
            pts.iter().map(|p| p.y).sum::<f32>() / 4.0,
        );
        fan(ctx.painter, c, &pts, col);
        let r = Rect::from_points(&pts);
        poly_bounds = Some(poly_bounds.map(|b| b.union(r)).unwrap_or(r));
    }

    // `obj.draw(...)` がスクリプト内で複数回呼ばれている場合（円形配置・砕け散る・ランダム配置等）は各インスタンスを描画
    if !lua_st.draw_calls.is_empty() {
        let mut combined_bounds: Option<Rect> = poly_bounds;
        for dc in &lua_st.draw_calls {
            let dc_rot = rotate_3d([dc.ox * gt.zoom, dc.oy * gt.zoom, dc.oz * gt.zoom], gt.rx, gt.ry, gt.rz);
            let inst_raw_x = raw_x + dc_rot[0];
            let inst_raw_y = raw_y + dc_rot[1];
            let inst_raw_z = raw_z + dc_rot[2];
            let inst_raw_zoom = (raw_zoom * dc.zoom).max(0.0);
            let inst_raw_rx = raw_rx + dc.rx;
            let inst_raw_ry = raw_ry + dc.ry;
            let inst_raw_rz = raw_rz + dc.rz;

            let (inst_x, inst_y, inst_zoom, inst_angle) = if obj.camera_target {
                if let Some(ref cam) = ctx.camera {
                    if let Some((proj_x, proj_y, scale_3d, _)) = cam.project([inst_raw_x, inst_raw_y, inst_raw_z]) {
                        (
                            proj_x,
                            proj_y,
                            (inst_raw_zoom * scale_3d as f64) as f32,
                            ((inst_raw_rz - cam.roll) as f32).to_radians(),
                        )
                    } else {
                        continue;
                    }
                } else {
                    (
                        inst_raw_x as f32,
                        inst_raw_y as f32,
                        inst_raw_zoom as f32,
                        (inst_raw_rz as f32).to_radians(),
                    )
                }
            } else {
                (
                    inst_raw_x as f32,
                    inst_raw_y as f32,
                    inst_raw_zoom as f32,
                    (inst_raw_rz as f32).to_radians(),
                )
            };

            let inst_alpha = (dc.alpha * gt.alpha).clamp(0.0, 1.0) as f32;
            let prim = dc.primitive.as_ref().or(lua_st.loaded_primitive.as_ref());
            let inst_transform_3d = if obj.camera_target {
                Some((
                    [inst_raw_x, inst_raw_y, inst_raw_z],
                    [inst_raw_rx, inst_raw_ry, inst_raw_rz],
                    inst_raw_zoom as f32,
                ))
            } else {
                None
            };
            let mut inst_fx = fx;
            if let Some(clip) = dc.clip {
                inst_fx.clip_insets[0] += clip[0] as f32;
                inst_fx.clip_insets[1] += clip[1] as f32;
                inst_fx.clip_insets[2] += clip[2] as f32;
                inst_fx.clip_insets[3] += clip[3] as f32;
            }
            if let Some(b) = draw_single_instance(
                ctx,
                obj,
                src,
                rel,
                inst_x,
                inst_y,
                inst_zoom,
                inst_angle,
                inst_alpha,
                prim,
                &inst_fx,
                inst_transform_3d,
                single_text_char,
                selected,
            ) {
                combined_bounds = Some(combined_bounds.map(|prev| prev.union(b)).unwrap_or(b));
            }
        }
        if selected {
            if let Some(b) = combined_bounds {
                ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
            }
        }
        return combined_bounds;
    }

    if !lua_st.poly_calls.is_empty() {
        return poly_bounds;
    }

    let transform_3d = if obj.camera_target {
        Some(([raw_x, raw_y, raw_z], [raw_rx, raw_ry, raw_rz], raw_zoom as f32))
    } else {
        None
    };

    // 「画像ループ」フィルタが有効な場合は nx × ny タイル状に展開して描画
    if let Some((nx, ny, sx, sy)) = fx.image_loop {
        if nx > 1 || ny > 1 {
            let tile_step = (approx_size as f32 * (raw_zoom as f32)).max(24.0);
            let scroll_x = (time_sec as f32 * sx) % tile_step;
            let scroll_y = (time_sec as f32 * sy) % tile_step;
            let mut loop_bounds: Option<Rect> = None;
            let mut tile_fx = fx;
            tile_fx.image_loop = None;
            for iy in 0..ny {
                for ix in 0..nx {
                    let ox_tile = (ix as f32 - (nx as f32 - 1.0) * 0.5) * tile_step + scroll_x;
                    let oy_tile = (iy as f32 - (ny as f32 - 1.0) * 0.5) * tile_step + scroll_y;
                    if let Some(b) = draw_single_instance(
                        ctx,
                        obj,
                        src,
                        rel,
                        x + ox_tile,
                        y + oy_tile,
                        zoom,
                        angle,
                        alpha,
                        lua_st.loaded_primitive.as_ref(),
                        &tile_fx,
                        transform_3d,
                        single_text_char,
                        false,
                    ) {
                        loop_bounds = Some(loop_bounds.map(|prev| prev.union(b)).unwrap_or(b));
                    }
                }
            }
            if selected {
                if let Some(b) = loop_bounds {
                    ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
                }
            }
            return loop_bounds;
        }
    }

    let bounds = draw_single_instance(
        ctx,
        obj,
        src,
        rel,
        x,
        y,
        zoom,
        angle,
        alpha,
        lua_st.loaded_primitive.as_ref(),
        &fx,
        transform_3d,
        single_text_char,
        selected,
    );

    if selected {
        if let Some(b) = bounds {
            ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
        }
    }
    bounds
}

/// 枠線（またはワイプ・マスクで中抜きされた図形）を、外周頂点列と内周頂点列を結ぶドーナツ型メッシュで描画します。
/// 3D カメラ回転時にも角の欠けや歪みが一切なく、エッジに沿った美しいグロー発光も同時に描画します。
fn draw_shape_mesh(
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

#[allow(clippy::too_many_arguments)]
fn draw_single_instance(
    ctx: &DrawCtx,
    obj: &TimelineObject,
    src: &au2_core::Effect,
    rel: Frame,
    x: f32,
    y: f32,
    zoom: f32,
    angle: f32,
    alpha: f32,
    loaded_prim: Option<&LuaLoadedPrimitive>,
    fx: &FilterVisuals,
    transform_3d: Option<([f64; 3], [f64; 3], f32)>,
    single_text_char: Option<&TextCharLayout>,
    selected: bool,
) -> Option<Rect> {
    let center = ctx.canvas.center() + vec2(x, y) * ctx.scale;
    let k = ctx.scale * zoom;

    // クリッピング（上・下・左・右）が設定されている場合は描画ペインターのクリップ矩形を絞り込む
    let has_clip = fx.clip_insets.iter().any(|&v| v > 0.1);
    let clipped_painter = if has_clip {
        let approx_r = (src.eval("サイズ", rel, 200.0) as f32 * k * 0.5).max(50.0 * k);
        let clip_rect = Rect::from_min_max(
            pos2(
                center.x - (approx_r - fx.clip_insets[2] * k).max(0.0),
                center.y - (approx_r - fx.clip_insets[0] * k).max(0.0),
            ),
            pos2(
                center.x + (approx_r - fx.clip_insets[3] * k).max(0.0),
                center.y + (approx_r - fx.clip_insets[1] * k).max(0.0),
            ),
        );
        Some(ctx.painter.with_clip_rect(clip_rect.intersect(ctx.canvas)))
    } else {
        None
    };
    let painter = clipped_painter.as_ref().unwrap_or(ctx.painter);

    let apply_tint = |c: Color32| -> Color32 {
        let mut r = c.r() as f32;
        let mut g = c.g() as f32;
        let mut b = c.b() as f32;

        // 輝度反転 / 色相反転
        if fx.inversions.2 || fx.inversions.3 {
            r = 255.0 - r;
            g = 255.0 - g;
            b = 255.0 - b;
        }

        // 色調補正（明るさ・彩度）
        if fx.hsv_shift.0.abs() > 0.01 {
            let delta = fx.hsv_shift.0 * 255.0;
            r = (r + delta).clamp(0.0, 255.0);
            g = (g + delta).clamp(0.0, 255.0);
            b = (b + delta).clamp(0.0, 255.0);
        }
        if fx.hsv_shift.2 > 0.0 && (fx.hsv_shift.2 - 1.0).abs() > 0.01 {
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            let sat = fx.hsv_shift.2;
            r = (luma + (r - luma) * sat).clamp(0.0, 255.0);
            g = (luma + (g - luma) * sat).clamp(0.0, 255.0);
            b = (luma + (b - luma) * sat).clamp(0.0, 255.0);
        }

        // 単色化
        if let Some((tc, s)) = fx.tint_color {
            let inv = 1.0 - s;
            r = r * inv + tc.r() as f32 * s;
            g = g * inv + tc.g() as f32 * s;
            b = b * inv + tc.b() as f32 * s;
        }

        // グラデーション
        if let Some((gc, s)) = fx.grad_color {
            let inv = 1.0 - s;
            r = r * inv + gc.r() as f32 * s;
            g = g * inv + gc.g() as f32 * s;
            b = b * inv + gc.b() as f32 * s;
        }

        // グローまたは加算合成（1, 11, 12）時の発光ハイライト強調
        let is_glow_or_add = fx.glow_strength > 0.01 || fx.blend_mode == 1 || fx.blend_mode == 11 || fx.blend_mode == 12;
        if is_glow_or_add {
            let boost = (fx.glow_strength * 0.22 + if fx.blend_mode == 1 { 0.35 } else { 0.15 }).min(0.68);
            r = (r * (1.0 - boost) + 255.0 * boost).min(255.0);
            g = (g * (1.0 - boost) + 255.0 * boost).min(255.0);
            b = (b * (1.0 - boost) + 255.0 * boost).min(255.0);
        }

        let eff_alpha = if fx.inversions.4 { (1.0 - alpha).clamp(0.0, 1.0) } else { alpha };
        Color32::from_rgba_unmultiplied(r as u8, g as u8, b as u8, c.a()).gamma_multiply(eff_alpha)
    };

    // ドロップシャドウの背面描画（テキストはグリフ単位の正確なシャドーを描画するため除外）
    if !matches!(src.kind, EffectKind::Text) {
        if let Some(sh_off) = fx.shadow_offset {
            let sh_center = center + sh_off * ctx.scale;
            let sh_col = Color32::from_black_alpha((fx.shadow_alpha * alpha * 180.0) as u8);
            painter.circle_filled(sh_center, 45.0 * k, sh_col);
        }
    }

    // カスタムオブジェクト (.obj) 等で `obj.load("figure", ...)` や `obj.mes(...)` が実行された場合
    if let Some(prim) = loaded_prim {
        match prim {
            LuaLoadedPrimitive::Figure { fig_type, color, size, line_width } => {
                let raw_sz = *size as f32;
                let sz = (raw_sz * k).max(1.0);
                let col = apply_tint(u32_to_color32(*color));
                let is_stroke = *line_width > 0.0 && (*line_width as f32) < raw_sz * 0.5 && *line_width < 3999.0;
                let inner_ratio = if is_stroke {
                    ((raw_sz - (*line_width as f32) * 2.0).max(0.0) / raw_sz.max(1.0)).max(fx.inner_hollow_ratio)
                } else {
                    fx.inner_hollow_ratio
                };
                let (w_mult, h_mult) = if fx.aspect_override.abs() > 0.001 {
                    let a = fx.aspect_override.clamp(-0.98, 0.98);
                    if a >= 0.0 {
                        (1.0 - a, 1.0)
                    } else {
                        (1.0, 1.0 + a)
                    }
                } else {
                    (1.0, 1.0)
                };

                let local_pts: Vec<[f32; 2]> = match fig_type.as_str() {
                    "四角形" => {
                        let hw = raw_sz * 0.5 * w_mult;
                        let hh = raw_sz * 0.5 * h_mult;
                        vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                    }
                    "三角形" => (0..3).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 3.0;
                        [a.cos() * raw_sz * 0.5 * w_mult, a.sin() * raw_sz * 0.5 * h_mult]
                    }).collect(),
                    "五角形" => (0..5).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 5.0;
                        [a.cos() * raw_sz * 0.5 * w_mult, a.sin() * raw_sz * 0.5 * h_mult]
                    }).collect(),
                    "六角形" => (0..6).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 6.0;
                        [a.cos() * raw_sz * 0.5 * w_mult, a.sin() * raw_sz * 0.5 * h_mult]
                    }).collect(),
                    "星型" => (0..10).map(|i| {
                        let r = if i % 2 == 0 { raw_sz * 0.5 } else { raw_sz * 0.2 };
                        let a = -PI / 2.0 + TAU * (i as f32) / 10.0;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect(),
                    _ => (0..24).map(|i| {
                        let a = TAU * (i as f32) / 24.0;
                        [a.cos() * raw_sz * 0.5 * w_mult, a.sin() * raw_sz * 0.5 * h_mult]
                    }).collect(),
                };

                let outer_screen = if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                    project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, &local_pts, r_zoom)
                } else {
                    let rot = Rot2::from_angle(angle);
                    Some(local_pts.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
                };
                let Some(outer_pts) = outer_screen else { return None };

                let inner_screen = if inner_ratio > 0.01 {
                    let inner_local: Vec<[f32; 2]> = local_pts.iter().map(|&[lx, ly]| [lx * inner_ratio, ly * inner_ratio]).collect();
                    if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                        project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, &inner_local, r_zoom)
                    } else {
                        let rot = Rot2::from_angle(angle);
                        Some(inner_local.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
                    }
                } else {
                    None
                };

                draw_shape_mesh(painter, center, &outer_pts, inner_screen.as_deref(), col, fx, ctx.scale, alpha);
                return Some(Rect::from_center_size(center, vec2(sz, sz)));
            }
            LuaLoadedPrimitive::Text { text, size, color, .. } => {
                let sz = (*size as f32 * k).clamp(1.0, 96.0);
                let col = apply_tint(u32_to_color32(*color));
                let galley = painter.layout(text.clone(), FontId::proportional(sz), col, f32::INFINITY);
                let half = galley.size() / 2.0;
                let pos = center - Rot2::from_angle(angle) * half;
                let r = Rect::from_center_size(center, galley.size());
                painter.add(egui::epaint::TextShape::new(pos, galley, col).with_angle(angle));
                return Some(r);
            }
            LuaLoadedPrimitive::Image { .. } => {}
        }
    }

    match src.kind {
        EffectKind::Shape => {
            let raw_size = src.eval("サイズ", rel, 100.0) as f32;
            let size = (raw_size * k).max(1.0);
            let line_width = src.eval("ライン幅", rel, 4000.0) as f32;
            let aspect = src.eval("縦横比", rel, 0.0) as f32 / 100.0;
            let (w_mult, h_mult) = if aspect >= 0.0 { (1.0 - aspect, 1.0) } else { (1.0, 1.0 + aspect) };
            let color = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let ty: u32 = src.text("type").and_then(|t| t.trim().parse().ok()).unwrap_or(1);

            // 極座標変換フィルタが適用されている場合: 直線や四角形を円環（リング・渦巻）に変換
            let effective_ty = if fx.polar_transform.is_some() && ty != 0 { 1 } else { ty };
            let polar_inner = fx.polar_transform.map(|(cw, zm, _, _)| {
                if cw > 1.0 {
                    (cw / (raw_size * zm + cw).max(1.0)).clamp(0.15, 0.92)
                } else {
                    0.55
                }
            }).unwrap_or(0.0);

            // AviUtl の図形: ライン幅 < サイズ/2 の場合は枠線（中抜き）図形、さらにワイプや反転マスク・極座標変換による中抜き比率も統合
            let is_stroke = effective_ty != 0 && line_width > 0.0 && line_width < raw_size * 0.5 && line_width < 3999.0;
            let stroke_inner_ratio = if is_stroke {
                ((raw_size - line_width * 2.0).max(0.0) / raw_size.max(1.0)).clamp(0.02, 0.98)
            } else {
                0.0
            };
            let inner_ratio = stroke_inner_ratio.max(fx.inner_hollow_ratio).max(polar_inner);

            if fx.border_width > 0.0 && effective_ty != 0 {
                let bcol = fx.border_color.unwrap_or(Color32::WHITE).gamma_multiply(alpha);
                painter.circle_stroke(
                    center,
                    size * 0.5 + fx.border_width * ctx.scale * 0.5,
                    Stroke::new((fx.border_width * ctx.scale).max(1.0), bcol),
                );
            }

            // ローカル頂点列を生成（縦横比を反映）
            let local_pts: Vec<[f32; 2]> = match effective_ty {
                0 => {
                    // 背景
                    let hw = ctx.screen_w as f32 / 2.0;
                    let hh = ctx.screen_h as f32 / 2.0;
                    vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                }
                1 => {
                    // 円 (32分割の多角形近似で3D透視投影・中抜きリング・扇クリッピングRにも滑らかに対応)
                    let rx = (raw_size * w_mult) / 2.0;
                    let ry = (raw_size * h_mult) / 2.0;
                    if let Some((start_deg, end_deg)) = fx.sector_clip {
                        let span = (end_deg - start_deg).clamp(-360.0, 360.0);
                        if span.abs() < 0.5 {
                            return None;
                        }
                        if span.abs() < 359.5 {
                            let s_rad = (start_deg - 90.0).to_radians();
                            let e_rad = (end_deg - 90.0).to_radians();
                            let steps = 28usize;
                            let r_in_ratio = inner_ratio;
                            let mut outer_arc = Vec::with_capacity(steps + 1);
                            let mut inner_arc = Vec::with_capacity(steps + 1);
                            for i in 0..=steps {
                                let t = i as f32 / steps as f32;
                                let a = s_rad + (e_rad - s_rad) * t;
                                outer_arc.push([a.cos() * rx, a.sin() * ry]);
                                inner_arc.push([a.cos() * rx * r_in_ratio, a.sin() * ry * r_in_ratio]);
                            }
                            let proj_pts = |pts_2d: &[[f32; 2]]| -> Option<Vec<Pos2>> {
                                if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                                    project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, pts_2d, r_zoom)
                                } else {
                                    let rot = Rot2::from_angle(angle);
                                    Some(pts_2d.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
                                }
                            };
                            if let (Some(out_s), Some(in_s)) = (proj_pts(&outer_arc), proj_pts(&inner_arc)) {
                                let mut mesh = Mesh::default();
                                let n_v = out_s.len();
                                for i in 0..n_v {
                                    mesh.colored_vertex(out_s[i], color);
                                    mesh.colored_vertex(in_s[i], color);
                                }
                                for i in 0..(n_v - 1) {
                                    let o0 = (i * 2) as u32;
                                    let i0 = (i * 2 + 1) as u32;
                                    let o1 = ((i + 1) * 2) as u32;
                                    let i1 = ((i + 1) * 2 + 1) as u32;
                                    mesh.add_triangle(o0, o1, i0);
                                    mesh.add_triangle(i0, o1, i1);
                                }
                                painter.add(Shape::mesh(mesh));
                                return Some(Rect::from_points(&out_s));
                            }
                            return None;
                        }
                    }
                    (0..32).map(|i| {
                        let a = TAU * (i as f32) / 32.0;
                        [a.cos() * rx, a.sin() * ry]
                    }).collect()
                }
                2 => {
                    // 四角形
                    let hw = (raw_size * w_mult) / 2.0;
                    let hh = (raw_size * h_mult) / 2.0;
                    vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                }
                3 => {
                    // 三角形
                    let r = raw_size / 2.0;
                    (0..3).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 3.0;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect()
                }
                4 => {
                    // 五角形
                    let r = raw_size / 2.0;
                    (0..5).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 5.0;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect()
                }
                5 => {
                    // 六角形
                    let r = raw_size / 2.0;
                    (0..6).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 6.0;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect()
                }
                6 => {
                    // 星型
                    let r = raw_size / 2.0;
                    (0..10).map(|i| {
                        let rad = if i % 2 == 0 { r } else { r * 0.4 };
                        let a = -PI / 2.0 + TAU * (i as f32) / 10.0;
                        [a.cos() * rad * w_mult, a.sin() * rad * h_mult]
                    }).collect()
                }
                n => {
                    let sides = (n as usize).clamp(3, 24);
                    let r = raw_size / 2.0;
                    (0..sides).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / sides as f32;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect()
                }
            };

            // 外周頂点をスクリーン座標に変換（3Dカメラ投影 or 2D平面変換）
            let screen_pts: Option<Vec<Pos2>> = if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, &local_pts, r_zoom)
            } else {
                let rot = Rot2::from_angle(angle);
                Some(local_pts.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
            };

            let Some(pts) = screen_pts else {
                return None;
            };

            // 内周頂点（枠線またはワイプ・反転マスクで中抜きされる場合）
            let inner_screen_pts: Option<Vec<Pos2>> = if ty != 0 && inner_ratio > 0.01 {
                let inner_local: Vec<[f32; 2]> = local_pts
                    .iter()
                    .map(|&[lx, ly]| [lx * inner_ratio, ly * inner_ratio])
                    .collect();
                if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                    project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, &inner_local, r_zoom)
                } else {
                    let rot = Rot2::from_angle(angle);
                    Some(inner_local.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
                }
            } else {
                None
            };

            // 色ズレ（色収差）
            if fx.chroma_shift > 0.5 && ty != 0 {
                let shift_vec = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift * ctx.scale;
                let r_pts: Vec<Pos2> = pts.iter().map(|&p| p - shift_vec).collect();
                let b_pts: Vec<Pos2> = pts.iter().map(|&p| p + shift_vec).collect();
                let r_in: Option<Vec<Pos2>> = inner_screen_pts.as_ref().map(|ip| ip.iter().map(|&p| p - shift_vec).collect());
                let b_in: Option<Vec<Pos2>> = inner_screen_pts.as_ref().map(|ip| ip.iter().map(|&p| p + shift_vec).collect());
                let mut no_glow_fx = *fx;
                no_glow_fx.glow_strength = 0.0;
                draw_shape_mesh(
                    painter,
                    center - shift_vec,
                    &r_pts,
                    r_in.as_deref(),
                    Color32::from_rgba_unmultiplied(255, 50, 50, (alpha * 130.0) as u8),
                    &no_glow_fx,
                    ctx.scale,
                    alpha,
                );
                draw_shape_mesh(
                    painter,
                    center + shift_vec,
                    &b_pts,
                    b_in.as_deref(),
                    Color32::from_rgba_unmultiplied(50, 180, 255, (alpha * 130.0) as u8),
                    &no_glow_fx,
                    ctx.scale,
                    alpha,
                );
            }

            draw_shape_mesh(
                painter,
                center,
                &pts,
                inner_screen_pts.as_deref(),
                color,
                fx,
                ctx.scale,
                alpha,
            );

            // ミラー反射
            if fx.mirror_alpha > 0.01 && ty != 0 {
                let mirror_offset = vec2(0.0, size * h_mult);
                let m_pts: Vec<Pos2> = pts.iter().map(|&p| pos2(p.x, center.y * 2.0 - p.y) + mirror_offset).collect();
                let m_in: Option<Vec<Pos2>> = inner_screen_pts
                    .as_ref()
                    .map(|ip| ip.iter().map(|&p| pos2(p.x, center.y * 2.0 - p.y) + mirror_offset).collect());
                let mut m_fx = *fx;
                m_fx.glow_strength *= 0.4;
                draw_shape_mesh(
                    painter,
                    center + mirror_offset,
                    &m_pts,
                    m_in.as_deref(),
                    color.gamma_multiply(fx.mirror_alpha * 0.5),
                    &m_fx,
                    ctx.scale,
                    alpha * fx.mirror_alpha * 0.5,
                );
            }

            Some(Rect::from_points(&pts))
        }
        EffectKind::Text => {
            // epaint のフォントアトラスは巨大フォント（>100pt）でアトラス境界違反・クラッシュを起こすため、
            // フォントのラスタライズサイズは最大 96.0 に抑え、それ以上の超巨大テキストはメッシュ頂点スケーリングで安全に描画する
            let max_font_size = 96.0_f32;
            let raw_canvas_size = src.eval("サイズ", rel, 34.0).max(1.0) as f32;
            let scaled_raw_size = (raw_canvas_size * k).max(1.0);
            let (font_size, scale_up) = if scaled_raw_size > max_font_size {
                (max_font_size, (scaled_raw_size / max_font_size).min(100.0))
            } else {
                (scaled_raw_size, 1.0)
            };

            let base_color = src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE);
            let color2 = src.text("color2").and_then(parse_color).unwrap_or(Color32::BLACK);
            let text_type = src
                .text("type")
                .and_then(|s| s.trim().parse::<u8>().ok())
                .unwrap_or(0);

            // 文字レイアウトの取得（個別オブジェクト時は単一文字、非分割時は全文レイアウト）
            let (char_items, mirror_bottom_y): (Vec<TextCharLayout>, f32) = if let Some(single) = single_text_char {
                (vec![single.clone()], raw_canvas_size * 0.5)
            } else {
                layout_aviutl_text(ctx, src, rel)
            };

            if char_items.is_empty() {
                return None;
            }

            // 3D回転（X軸回転・Y軸回転）が設定されているか判定
            let use_3d_mesh = if let (Some((_, rot_3d, _)), Some(_)) = (transform_3d, &ctx.camera) {
                rot_3d[0].abs() > 0.1 || rot_3d[1].abs() > 0.1
            } else {
                false
            };

            // ローカルキャンバス座標 `local_off` にある1つの Galley を、3D透視変換または2D回転スケーリングで描画するクロージャ
            let draw_galley_at = |p: &egui::Painter,
                                  local_off: egui::Vec2,
                                  galley: std::sync::Arc<egui::epaint::Galley>,
                                  col: Color32,
                                  flip_y_around: Option<f32>|
             -> Option<Rect> {
                let half = galley.size() * 0.5;
                if use_3d_mesh || scale_up > 1.001 || flip_y_around.is_some() {
                    let shape = egui::epaint::TextShape::new(Pos2::ZERO, galley.clone(), col);
                    let clipped = egui::epaint::ClippedShape {
                        clip_rect: egui::Rect::EVERYTHING,
                        shape: egui::Shape::Text(shape),
                    };
                    let primitives = p.ctx().tessellate(vec![clipped], p.ctx().pixels_per_point());
                    let inv_k = 1.0 / k.max(0.0001);
                    let rot2 = Rot2::from_angle(angle);
                    let mut bounds: Option<Rect> = None;
                    for prim in primitives {
                        if let egui::epaint::Primitive::Mesh(mut mesh) = prim.primitive {
                            let mut all_valid = true;
                            for v in &mut mesh.vertices {
                                v.color = col;
                                let gx = (v.pos.x - half.x) * scale_up * inv_k;
                                let gy = (v.pos.y - half.y) * scale_up * inv_k;
                                let lx = local_off.x + gx;
                                let mut ly = local_off.y + gy;
                                if let Some(my) = flip_y_around {
                                    ly = my * 2.0 - ly;
                                }
                                if use_3d_mesh {
                                    if let (Some((w_pos, rot_3d, r_zoom)), Some(ref cam)) = (transform_3d, &ctx.camera) {
                                        let p_rot = rotate_3d(
                                            [(lx * r_zoom) as f64, (ly * r_zoom) as f64, 0.0],
                                            rot_3d[0],
                                            rot_3d[1],
                                            rot_3d[2],
                                        );
                                        let pw = [
                                            w_pos[0] + p_rot[0],
                                            w_pos[1] + p_rot[1],
                                            w_pos[2] + p_rot[2],
                                        ];
                                        if let Some((sx, sy, _, _)) = cam.project(pw) {
                                            v.pos = ctx.canvas.center() + vec2(sx, sy) * ctx.scale;
                                        } else {
                                            all_valid = false;
                                            break;
                                        }
                                    }
                                } else {
                                    v.pos = center + rot2 * (vec2(lx, ly) * k);
                                }
                            }
                            if all_valid && !mesh.vertices.is_empty() {
                                let mut min_p = mesh.vertices[0].pos;
                                let mut max_p = mesh.vertices[0].pos;
                                for v in &mesh.vertices {
                                    min_p = min_p.min(v.pos);
                                    max_p = max_p.max(v.pos);
                                }
                                let r = Rect::from_min_max(min_p, max_p);
                                bounds = Some(bounds.map(|b| b.union(r)).unwrap_or(r));
                                p.add(egui::Shape::Mesh(mesh));
                            }
                        }
                    }
                    bounds
                } else {
                    let rot2 = Rot2::from_angle(angle);
                    let scaled_off: egui::Vec2 = local_off * k;
                    let target_center = center + (rot2 * scaled_off);
                    let unscaled_pos = target_center - (rot2 * half);
                    let r = Rect::from_center_size(target_center, galley.size());
                    p.add(egui::epaint::TextShape::new(unscaled_pos, galley, col).with_angle(angle));
                    Some(r)
                }
            };

            let mut total_bounds: Option<Rect> = None;

            for item in &char_items {
                if item.ch == " " || item.ch == "　" {
                    continue;
                }
                let item_off = vec2(item.local_x, item.local_y);
                let main_col = apply_tint(item.color_override.unwrap_or(base_color));
                let galley = painter.layout_no_wrap(item.ch.clone(), FontId::proportional(font_size), main_col);

                // 1. グロー効果 (`fx.glow_strength`)
                if fx.glow_strength > 0.01 {
                    let glow_alpha = (fx.glow_strength * alpha * 38.0).clamp(8.0, 140.0) as u8;
                    let gcol = fx
                        .glow_color
                        .unwrap_or(Color32::from_rgb(
                            main_col.r().saturating_add(50),
                            main_col.g().saturating_add(50),
                            main_col.b().saturating_add(50),
                        ));
                    let gc = Color32::from_rgba_unmultiplied(gcol.r(), gcol.g(), gcol.b(), glow_alpha);
                    let gr = (fx.glow_radius * 0.12).clamp(1.5, 8.0);
                    for (dx, dy) in [(-gr, 0.0), (gr, 0.0), (0.0, -gr), (0.0, gr)] {
                        let _ = draw_galley_at(painter, item_off + vec2(dx, dy), galley.clone(), gc.gamma_multiply(0.45), None);
                    }
                }
                if fx.glow_only {
                    continue;
                }

                // 2. シャドーフィルタ (`fx.shadow_offset`)
                if let Some(sh_off) = fx.shadow_offset {
                    let sh_col = Color32::from_black_alpha((fx.shadow_alpha * alpha * 200.0) as u8);
                    let _ = draw_galley_at(painter, item_off + sh_off, galley.clone(), sh_col, None);
                }

                // 3. 文字装飾 (	ype = 1..4: 影付き / 縁取り) または 縁取りフィルタ (x.border_width)
                if text_type == 1 || text_type == 2 {
                    // 多層ソフトドロップシャドウ（自然で上品なテロップ影）
                    let sh_a = if text_type == 1 { 0.85 } else { 0.45 };
                    let d = (raw_canvas_size * 0.075).clamp(2.0, 7.0);
                    let base_sc = color2.gamma_multiply(alpha * sh_a);
                    let _ = draw_galley_at(painter, item_off + vec2(d, d), galley.clone(), base_sc, None);

                    // 微細ブラー層（ソフトエッジ）
                    let blur_sc = color2.gamma_multiply(alpha * sh_a * 0.35);
                    let blur_r = (d * 0.45).clamp(1.0, 3.0);
                    for (bx, by) in [(-blur_r, 0.0), (blur_r, 0.0), (0.0, -blur_r), (0.0, blur_r)] {
                        let _ = draw_galley_at(painter, item_off + vec2(d + bx, d + by), galley.clone(), blur_sc, None);
                    }
                } else if text_type == 3 || text_type == 4 || fx.border_width > 0.5 {
                    // 高品質・角丸円周サンプリング縁取り（16〜24方向＋内側補間）
                    let bw = if fx.border_width > 0.5 {
                        fx.border_width
                    } else if text_type == 3 {
                        (raw_canvas_size * 0.075).clamp(2.0, 6.5)
                    } else {
                        (raw_canvas_size * 0.040).clamp(1.2, 3.2)
                    };
                    let bc = if fx.border_width > 0.5 {
                        fx.border_color.unwrap_or(color2)
                    } else {
                        color2
                    }
                    .gamma_multiply(alpha);

                    // 太さに応じて 16 または 24 方向を全周サンプリング（滑らかな角丸縁取り）
                    let samples = if bw >= 3.5 { 24 } else { 16 };
                    let step_angle = std::f32::consts::TAU / (samples as f32);

                    // 太い縁取りの時は内側の隙間を埋めるため中間半径もサンプリング
                    if bw >= 2.5 {
                        let inner_r = bw * 0.55;
                        for i in 0..samples {
                            let angle = i as f32 * step_angle;
                            let dx = inner_r * angle.cos();
                            let dy = inner_r * angle.sin();
                            let _ = draw_galley_at(painter, item_off + vec2(dx, dy), galley.clone(), bc, None);
                        }
                    }
                    // 外周サンプリング
                    for i in 0..samples {
                        let angle = i as f32 * step_angle;
                        let dx = bw * angle.cos();
                        let dy = bw * angle.sin();
                        let _ = draw_galley_at(painter, item_off + vec2(dx, dy), galley.clone(), bc, None);
                    }
                }

                // 4. 色ズレ (`fx.chroma_shift`)
                if fx.chroma_shift > 0.5 {
                    let shift_canvas = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift;
                    let r_col = Color32::from_rgba_unmultiplied(255, 50, 50, (alpha * 150.0) as u8);
                    let b_col = Color32::from_rgba_unmultiplied(50, 180, 255, (alpha * 150.0) as u8);
                    let _ = draw_galley_at(painter, item_off - shift_canvas, galley.clone(), r_col, None);
                    let _ = draw_galley_at(painter, item_off + shift_canvas, galley.clone(), b_col, None);
                }

                // 5. 凸エッジ (`fx.bevel_width`)
                if fx.bevel_width > 0.5 {
                    let bd = (fx.bevel_width * 0.35).clamp(0.8, 3.5);
                    let hi_col = Color32::from_white_alpha((alpha * 110.0) as u8);
                    let lo_col = Color32::from_black_alpha((alpha * 140.0) as u8);
                    let _ = draw_galley_at(painter, item_off + vec2(bd, bd), galley.clone(), lo_col, None);
                    let _ = draw_galley_at(painter, item_off - vec2(bd * 0.8, bd * 0.8), galley.clone(), hi_col, None);
                }

                // 6. 本体文字描画
                if let Some(r) = draw_galley_at(painter, item_off, galley.clone(), main_col, None) {
                    total_bounds = Some(total_bounds.map(|b| b.union(r)).unwrap_or(r));
                }

                // 7. ミラー反射 (`fx.mirror_alpha`)
                if fx.mirror_alpha > 0.01 {
                    let m_col = main_col.gamma_multiply(fx.mirror_alpha * 0.45);
                    let _ = draw_galley_at(painter, item_off, galley, m_col, Some(mirror_bottom_y));
                }
            }

            total_bounds
        }
        EffectKind::Scene => {
            // サブシーンオブジェクト: Scene 1..49 を再帰的にレンダリング
            let scene_id = src
                .text("scene")
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(1);
            if let Some(sub_scene) = ctx.scenes.get(&scene_id) {
                let start_pos = src.eval("再生位置", rel, 0.0).max(0.0) as Frame;
                let speed = src.eval("再生速度", rel, 100.0) / 100.0;
                let is_loop = src.eval("ループ再生", rel, src.eval("check0", rel, 0.0)) >= 0.5;
                let raw_sub_frame = start_pos + ((rel as f64) * speed.max(0.0)).round() as Frame;
                let sub_len = sub_scene
                    .layers
                    .iter()
                    .flat_map(|l| &l.objects)
                    .map(|o| o.frame_end)
                    .max()
                    .unwrap_or(1)
                    .max(1);
                let sub_frame = if is_loop { raw_sub_frame % sub_len } else { raw_sub_frame };

                let mut sub_gt = ctx.group_transform.unwrap_or_default();
                sub_gt.ox = x as f64;
                sub_gt.oy = y as f64;
                sub_gt.zoom = zoom as f64;
                sub_gt.rz = angle.to_degrees() as f64;
                sub_gt.alpha = alpha as f64;

                let sub_ctx = DrawCtx {
                    painter,
                    canvas: ctx.canvas,
                    scale: ctx.scale,
                    fps: ctx.fps,
                    screen_w: ctx.screen_w,
                    screen_h: ctx.screen_h,
                    textures: ctx.textures,
                    plugins: ctx.plugins,
                    scenes: ctx.scenes,
                    has_ffmpeg: ctx.has_ffmpeg,
                    camera: ctx.camera,
                    group_transform: Some(sub_gt),
                    scene_depth: ctx.scene_depth + 1,
                };
                render_scene_at(&sub_ctx, sub_scene, sub_frame, None);
            }
            Some(ctx.canvas)
        }
        EffectKind::ParticleOutput => {
            let freq = src.eval("出力頻度", rel, 10.0).clamp(1.0, 60.0) as usize;
            let speed = src.eval("出力速度", rel, 200.0) as f32;
            let col = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let time_s = rel as f32 / (ctx.fps.max(1.0) as f32);
            for i in 0..freq {
                let seed = (i as i32) * 13 + 7;
                let vx = aviutl_rand(-1.0, 1.0, seed, 1) as f32 * speed * 0.5;
                let vy = aviutl_rand(-1.0, 1.0, seed, 2) as f32 * speed * 0.5;
                let life = ((time_s + (i as f32) * 0.13) % 1.5) / 1.5;
                let p = center + vec2(vx * life, vy * life) * k;
                let p_col = col.gamma_multiply(1.0 - life);
                painter.circle_filled(p, (6.0 * k * (1.0 - life * 0.5)).max(1.5), p_col);
            }
            Some(Rect::from_center_size(center, vec2(160.0 * k, 160.0 * k)))
        }
        EffectKind::AudioWaveform => {
            let w = src.eval("横幅", rel, 240.0) as f32 * k;
            let h = src.eval("高さ", rel, 100.0) as f32 * k;
            let col = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let bars = 32usize;
            let bar_w = (w / bars as f32).max(1.5);
            for i in 0..bars {
                let amp = (aviutl_rand(0.1, 1.0, i as i32, rel as i64 / 2) as f32) * h * 0.5;
                let bx = center.x - w * 0.5 + (i as f32 + 0.5) * (w / bars as f32);
                painter.line_segment([pos2(bx, center.y - amp), pos2(bx, center.y + amp)], Stroke::new(bar_w * 0.7, col));
            }
            Some(Rect::from_center_size(center, vec2(w, h)))
        }
        EffectKind::ImageFile | EffectKind::VideoFile => {
            if let Some(&(tex_id, [tw, th])) = ctx.textures.get(&obj.id) {
                let size = vec2(tw as f32 * k, th as f32 * k);
                let r = Rect::from_center_size(center, size);
                let fh = fx.inversions.1;
                let fv = fx.inversions.0;
                if fx.chroma_shift > 0.5 {
                    let shift_vec = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift * ctx.scale;
                    draw_textured_quad_uv(
                        painter,
                        tex_id,
                        r.translate(-shift_vec),
                        angle,
                        Color32::from_rgba_unmultiplied(255, 60, 60, (alpha * 120.0) as u8),
                        fh,
                        fv,
                    );
                    draw_textured_quad_uv(
                        painter,
                        tex_id,
                        r.translate(shift_vec),
                        angle,
                        Color32::from_rgba_unmultiplied(60, 180, 255, (alpha * 120.0) as u8),
                        fh,
                        fv,
                    );
                }
                draw_textured_quad_uv(painter, tex_id, r, angle, apply_tint(Color32::WHITE), fh, fv);
                Some(r)
            } else {
                // 画像・動画ファイルがローカルに存在しない場合の軽量スマート描画（画面全体の塗り潰しを防止し、キャラクター立ち絵や映像配置を視認可能にする）
                let file_stem = src
                    .text("file")
                    .and_then(|p| std::path::Path::new(p).file_stem())
                    .and_then(|s| s.to_str())
                    .unwrap_or("Media");
                let is_video = matches!(src.kind, EffectKind::VideoFile);
                let (base_w, base_h) = if is_video { (360.0, 202.0) } else { (220.0, 320.0) };
                let size = vec2(base_w * k, base_h * k);
                let r = Rect::from_center_size(center, size);
                let corners = rotated_rect(r, angle);
                let fill_col = apply_tint(Color32::from_rgba_unmultiplied(65, 85, 120, 42));
                let stroke_col = apply_tint(Color32::from_rgba_unmultiplied(150, 190, 255, 130));
                fan(painter, center, &corners, fill_col);
                closed_stroke(painter, &corners, Stroke::new((1.5 * ctx.scale).max(1.0), stroke_col));

                let label = if fx.inversions.1 {
                    format!("{} [反転]", file_stem)
                } else {
                    file_stem.to_string()
                };
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    label,
                    FontId::proportional((13.0 * k.max(0.4)).clamp(9.0, 22.0)),
                    apply_tint(Color32::from_rgba_unmultiplied(235, 245, 255, 210)),
                );
                Some(r)
            }
        }
        EffectKind::CustomObject => {
            if selected {
                let r = Rect::from_center_size(center, vec2(64.0 * ctx.scale, 64.0 * ctx.scale));
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    obj.display_name(),
                    FontId::proportional(12.0),
                    apply_tint(Color32::LIGHT_GRAY),
                );
                Some(r)
            } else {
                None
            }
        }
        EffectKind::FrameBuffer => {
            if fx.chroma_shift > 0.5 {
                let shift = (fx.chroma_shift * ctx.scale).clamp(1.0, 24.0);
                let r_rect = Rect::from_min_max(ctx.canvas.left_top(), pos2(ctx.canvas.left() + shift * 1.5, ctx.canvas.bottom()));
                let b_rect = Rect::from_min_max(pos2(ctx.canvas.right() - shift * 1.5, ctx.canvas.top()), ctx.canvas.right_bottom());
                painter.rect_filled(r_rect, 0.0, Color32::from_rgba_unmultiplied(255, 60, 60, (alpha * 50.0) as u8));
                painter.rect_filled(b_rect, 0.0, Color32::from_rgba_unmultiplied(60, 180, 255, (alpha * 50.0) as u8));
            }
            Some(ctx.canvas)
        }
        _ => {
            let sz = (src.eval("サイズ", rel, 100.0) as f32 * k).clamp(16.0 * ctx.scale, ctx.canvas.width());
            let r = Rect::from_center_size(center, vec2(sz, sz));
            let corners = rotated_rect(r, angle);
            let col = apply_tint(
                src.text("color")
                    .and_then(parse_color)
                    .unwrap_or(Color32::from_rgba_unmultiplied(100, 130, 180, 70)),
            );
            fan(painter, center, &corners, col);
            if selected {
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    format!("{}\n{}", src.name, obj.display_name()),
                    FontId::proportional((13.0 * zoom.max(0.5)).min(24.0)),
                    apply_tint(Color32::WHITE),
                );
            }
            Some(r)
        }
    }
}

/// 回転・透明度付きのテクスチャ矩形を描画します。
#[allow(dead_code)]
fn draw_textured_quad(painter: &egui::Painter, tex_id: egui::TextureId, rect: Rect, angle: f32, tint: Color32) {
    draw_textured_quad_uv(painter, tex_id, rect, angle, tint, false, false);
}

/// UV反転（左右・上下）対応のテクスチャ矩形描画。
fn draw_textured_quad_uv(
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

/// フィルタオブジェクトのプレビュー（下位レイヤー全体に掛かる：色調補正・32bit/64bit `.auf` フィルタ・シーンチェンジ等）。
fn draw_filter_object(ctx: &DrawCtx, obj: &TimelineObject, rel: Frame) {
    let Some(src) = obj.source() else { return };
    if src.name == "色調補正" {
        let b = src.eval("明るさ", rel, 100.0) as f32;
        let overlay = if b >= 100.0 {
            Color32::from_white_alpha((((b - 100.0) / 100.0).min(1.0) * 160.0) as u8)
        } else {
            Color32::from_black_alpha((((100.0 - b) / 100.0).min(1.0) * 255.0) as u8)
        };
        ctx.painter.rect_filled(ctx.canvas, 0.0, overlay);
    } else if src.name.contains("グロー") || src.name.contains("発光") || src.name.contains("拡散光") || src.name.contains("閃光") {
        let strength = (src.eval("強さ", rel, 60.0) / 200.0).clamp(0.0, 1.0) as f32;
        let overlay = Color32::from_rgba_unmultiplied(255, 248, 215, (strength * 85.0) as u8);
        ctx.painter.rect_filled(ctx.canvas, 0.0, overlay);
    } else if src.name.contains("色収差") || src.name.contains("色ズレ") || src.name.contains("色ずれ") {
        let shift = (src.eval("ズレ幅", rel, src.eval("ずれ幅", rel, 8.0)) as f32 * ctx.scale).max(1.0);
        let r_rect = Rect::from_min_max(ctx.canvas.left_top(), pos2(ctx.canvas.left() + shift * 2.0, ctx.canvas.bottom()));
        let b_rect = Rect::from_min_max(pos2(ctx.canvas.right() - shift * 2.0, ctx.canvas.top()), ctx.canvas.right_bottom());
        ctx.painter.rect_filled(r_rect, 0.0, Color32::from_rgba_unmultiplied(255, 40, 40, 85));
        ctx.painter.rect_filled(b_rect, 0.0, Color32::from_rgba_unmultiplied(40, 180, 255, 85));
    } else if src.name == "シーンチェンジ" {
        let progress = (rel as f32 / (obj.length().max(1) as f32)).clamp(0.0, 1.0);
        // ピーク波形（トランジションの中間 p=0.5 で最大 1.0 になり、両端で 0.0 になる sin カーブ）
        let wave = (progress * PI).sin().clamp(0.0, 1.0);
        let sc_name = src.text("name").unwrap_or_default();
        let sc_type = src.text("type").and_then(|t| t.parse::<i32>().ok()).unwrap_or(0);

        match sc_name {
            "発光" => {
                let flash = (wave * 235.0) as u8;
                ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_white_alpha(flash));
            }
            "フェードアウトイン" => {
                let fade = (wave * 250.0) as u8;
                ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_black_alpha(fade));
            }
            "レンズブラー" | "放射ブラー" | "ぼかし" => {
                let bloom = (wave * 115.0) as u8;
                ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_rgba_unmultiplied(230, 240, 255, bloom));
            }
            "リール回転" | "スライス" | "押し出し(横)" | "押し出し(縦)" => {
                // リール回転・スライス時は画面を横切るモーションブラー帯を描画
                let bands = 5;
                let h = ctx.canvas.height() / bands as f32;
                let alpha_b = (wave * 95.0) as u8;
                for i in 0..bands {
                    let y0 = ctx.canvas.top() + ((i as f32 + progress * 3.0) % bands as f32) * h;
                    let r = Rect::from_min_max(pos2(ctx.canvas.left(), y0), pos2(ctx.canvas.right(), (y0 + h * 0.35).min(ctx.canvas.bottom())));
                    ctx.painter.rect_filled(r, 0.0, Color32::from_white_alpha(alpha_b));
                }
            }
            "ワイプ(円)" | "図形ワイプ" => {
                let max_r = ctx.canvas.width().hypot(ctx.canvas.height()) * 0.55;
                let r = max_r * (1.0 - wave * 0.85);
                ctx.painter.circle_stroke(ctx.canvas.center(), r, Stroke::new((max_r - r).max(1.0), Color32::from_black_alpha((wave * 200.0) as u8)));
            }
            "砕け散る" => {
                let count = 16;
                for i in 0..count {
                    let rx = aviutl_rand(-0.45, 0.45, i, 1) as f32 * ctx.canvas.width();
                    let ry = (aviutl_rand(-0.45, 0.45, i, 2) as f32 + progress * 0.4) * ctx.canvas.height();
                    let sz = (24.0 * (1.0 - progress * 0.7) * ctx.scale).max(4.0);
                    let p = ctx.canvas.center() + vec2(rx, ry);
                    ctx.painter.rect_filled(
                        Rect::from_center_size(p, vec2(sz, sz)),
                        0.0,
                        Color32::from_white_alpha(((1.0 - progress) * 140.0) as u8),
                    );
                }
            }
            _ => {
                if sc_type == 14 {
                    let fade = (wave * 250.0) as u8;
                    ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_black_alpha(fade));
                } else {
                    let fade = (wave * 140.0) as u8;
                    ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_black_alpha(fade));
                }
            }
        }
    }
}

fn rotated_rect(r: Rect, angle: f32) -> Vec<Pos2> {
    let rot = Rot2::from_angle(angle);
    let c = r.center();
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
        .into_iter()
        .map(|p| c + rot * (p - c))
        .collect()
}

/// 中心から見て星形（凹形含む）の多角形を三角形ファンで塗りつぶします。
fn fan(painter: &egui::Painter, center: Pos2, pts: &[Pos2], color: Color32) {
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

