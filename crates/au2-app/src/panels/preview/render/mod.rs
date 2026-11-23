//! オブジェクト・シェイプ・画像・動画・フィルタの合成レンダリング

mod instance;
mod object;
mod primitives;

use au2_core::{EffectKind, Frame, ObjectCategory, ObjectId, TimelineObject};
use au2_plugin::{LuaObjState, PluginRegistry};
use eframe::egui::{self, Rect};

use super::camera::{apply_camera_effect, rotate_3d, Camera3D, CameraParams};
use object::draw_object;

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
    pub(crate) audio_waveform: &'a [f32],
    pub(crate) current_scene: Option<(&'a au2_core::Scene, Frame)>,
    pub(crate) layer_limit: Option<usize>,
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

        // レイヤー設定「座標のリンク (coordinate_link)」が有効な場合、直上のリンク元レイヤーの座標オフセットを加算
        let mut cur = layer_idx;
        while cur > 0 && scene.layers.get(cur).map(|l| l.coordinate_link).unwrap_or(false) {
            cur -= 1;
            if let Some(anchor_layer) = scene.layers.get(cur) {
                if !anchor_layer.visible {
                    continue;
                }
                let af = resolve_layer_frame(cur);
                if let Some(anchor_obj) = anchor_layer.objects.iter().find(|o| o.contains(af)) {
                    let arel = af - anchor_obj.frame_start;
                    let adraw = anchor_obj
                        .effect(&EffectKind::StandardDraw)
                        .or_else(|| anchor_obj.effect(&EffectKind::ExtendedDraw));
                    let mut ax = adraw.map(|d| d.eval("X", arel, 0.0)).unwrap_or(0.0);
                    let mut ay = adraw.map(|d| d.eval("Y", arel, 0.0)).unwrap_or(0.0);
                    let mut az = adraw.map(|d| d.eval("Z", arel, 0.0)).unwrap_or(0.0);
                    for eff in &anchor_obj.effects {
                        if eff.enabled && eff.name == "座標" {
                            ax += eff.eval("X", arel, 0.0);
                            ay += eff.eval("Y", arel, 0.0);
                            az += eff.eval("Z", arel, 0.0);
                        }
                    }
                    combined.ox += ax;
                    combined.oy += ay;
                    combined.oz += az;
                }
            }
        }

        combined
    };

    // 4. 描画対象オブジェクトの収集と 3群ソート（2D背景 / 3Dカメラ対象 Z降順 / 2D前面）
    let mut render_items: Vec<(u8, f64, usize, &TimelineObject, Frame, GroupTransform)> = Vec::new();
    let cam_layer = active_camera.map(|(l, _)| l).unwrap_or(0);

    for (layer_idx, layer) in scene.layers.iter().enumerate() {
        if let Some(limit) = base_ctx.layer_limit {
            if layer_idx >= limit {
                break;
            }
        }
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

    // 5. 「上のオブジェクトでクリッピング (obj.clipping / layer.clip_above)」および「直前オブジェクト (PreviousObject)」の追跡を行いながら描画
    let mut last_non_clipping_bounds: Option<Rect> = None;
    let mut last_media_object: Option<(&TimelineObject, Frame)> = None;

    for (_, _, layer_idx, obj, rel, gt) in render_items {
        let is_clipping = obj.clipping
            || scene
                .layers
                .get(layer_idx)
                .map(|l| l.clip_above)
                .unwrap_or(false);
        let clipped_painter = if is_clipping {
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
            audio_waveform: base_ctx.audio_waveform,
            current_scene: Some((scene, frame)),
            layer_limit: base_ctx.layer_limit,
        };

        let is_prev_obj = matches!(
            obj.source().map(|s| &s.kind),
            Some(EffectKind::PreviousObject)
        );
        let drawn_bounds = if is_prev_obj {
            if let Some((prev_obj, _prev_rel)) = last_media_object {
                if let Some(prev_src) = prev_obj.source() {
                    let mut merged_effects = Vec::with_capacity(prev_obj.effects.len() + obj.effects.len());
                    merged_effects.push(prev_src.clone());
                    for eff in &prev_obj.effects {
                        if !std::ptr::eq(eff, prev_src)
                            && !matches!(
                                eff.kind,
                                EffectKind::StandardDraw
                                    | EffectKind::ExtendedDraw
                                    | EffectKind::StandardPlayback
                            )
                        {
                            merged_effects.push(eff.clone());
                        }
                    }
                    let obj_has_draw = obj.effects.iter().any(|e| {
                        matches!(e.kind, EffectKind::StandardDraw | EffectKind::ExtendedDraw)
                    });
                    if !obj_has_draw {
                        if let Some(prev_draw) = prev_obj
                            .effect(&EffectKind::StandardDraw)
                            .or_else(|| prev_obj.effect(&EffectKind::ExtendedDraw))
                        {
                            merged_effects.push(prev_draw.clone());
                        }
                    }
                    for eff in &obj.effects {
                        if !matches!(eff.kind, EffectKind::PreviousObject) {
                            merged_effects.push(eff.clone());
                        }
                    }
                    let mut merged_obj = prev_obj.clone();
                    merged_obj.frame_start = obj.frame_start;
                    merged_obj.frame_end = obj.frame_end;
                    merged_obj.group = obj.group;
                    merged_obj.camera_target = obj.camera_target;
                    merged_obj.clipping = obj.clipping;
                    merged_obj.effects = merged_effects;
                    draw_object(&item_ctx, layer_idx, &merged_obj, rel, selected_id == Some(obj.id))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            let b = draw_object(&item_ctx, layer_idx, obj, rel, selected_id == Some(obj.id));
            if obj.category() == ObjectCategory::Media {
                last_media_object = Some((obj, rel));
            }
            b
        };

        if !is_clipping {
            last_non_clipping_bounds = drawn_bounds;
        }
    }
}