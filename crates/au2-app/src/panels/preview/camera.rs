//! 3D カメラ制御・座標変換・透視投影・AviUtl 互換数学関数群

use eframe::egui::{Pos2, Rect, vec2};

#[derive(Clone, Copy, Debug)]
pub struct CameraParams {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub tx: f64,
    pub ty: f64,
    pub tz: f64,
    pub rz: f64,
    pub ux: f64,
    pub uy: f64,
    pub uz: f64,
    pub d: f64,
}

/// AviUtl 互換の決定論的疑似乱数生成器 (obj.rand 互換)
pub fn aviutl_rand(st_num: f64, ed_num: f64, seed: i32, frame: i64) -> f64 {
    let mut h = (frame as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ ((seed as u64).wrapping_mul(0xBF58476D1CE4E5B9));
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D049BB133111EB);
    h ^= h >> 31;
    let norm = (h as f64) / (u64::MAX as f64);
    st_num + norm * (ed_num - st_num)
}

/// AviUtl 互換の Catmull-Rom スプライン補間 (obj.interpolation 互換)
/// AviUtl の「手ぶれ・振動」効果における正規化乱数スケール（[-1000, 1000] を 1000 で除算して [-1.0, 1.0] に正規化）
const AVIUTL_SHAKE_RANDOM_SCALE: f64 = 1000.0;

pub fn aviutl_interpolation(t: f64, p0: f64, p1: f64, p2: f64, p3: f64) -> f64 {
    0.5 * (
        (2.0 * p1)
            + (-p0 + p2) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
            + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t
    )
}

/// カメラ効果（@手ぶれ、@目標中心回転、@目標サイズ固定視野角 等）をカメラパラメータに適用します。
pub fn apply_camera_effect(
    cam: &mut CameraParams,
    eff_name: &str,
    track0: f64,
    track1: f64,
    track2: f64,
    _track3: f64,
    time_sec: f64,
    screen_h: f64,
) {
    let sec = eff_name.split('@').next().unwrap_or(eff_name);
    match sec {
        "手ぶれ" | "手ブレ" => {
            if track2 <= 0.0 {
                cam.tx += aviutl_rand(-track0, track0, 0, (time_sec * 100.0) as i64);
                cam.ty += aviutl_rand(-track0, track0, 1, (time_sec * 100.0) as i64);
                cam.tz += aviutl_rand(-track0, track0, 2, (time_sec * 100.0) as i64);
                cam.rz += aviutl_rand(-track1, track1, 3, (time_sec * 100.0) as i64);
            } else {
                let t = time_sec * 100.0 / track2;
                let p = t.floor() as i64;
                let t_frac = t - (p as f64);

                let x0 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 0, p) / AVIUTL_SHAKE_RANDOM_SCALE * track0;
                let x1 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 0, p + 1) / AVIUTL_SHAKE_RANDOM_SCALE * track0;
                let x2 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 0, p + 2) / AVIUTL_SHAKE_RANDOM_SCALE * track0;
                let x3 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 0, p + 3) / AVIUTL_SHAKE_RANDOM_SCALE * track0;

                let y0 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 1, p) / AVIUTL_SHAKE_RANDOM_SCALE * track0;
                let y1 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 1, p + 1) / AVIUTL_SHAKE_RANDOM_SCALE * track0;
                let y2 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 1, p + 2) / AVIUTL_SHAKE_RANDOM_SCALE * track0;
                let y3 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 1, p + 3) / AVIUTL_SHAKE_RANDOM_SCALE * track0;

                let z0 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 2, p) / AVIUTL_SHAKE_RANDOM_SCALE * track0;
                let z1 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 2, p + 1) / AVIUTL_SHAKE_RANDOM_SCALE * track0;
                let z2 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 2, p + 2) / AVIUTL_SHAKE_RANDOM_SCALE * track0;
                let z3 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 2, p + 3) / AVIUTL_SHAKE_RANDOM_SCALE * track0;

                let r0 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 3, p) / AVIUTL_SHAKE_RANDOM_SCALE * track1;
                let r1 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 3, p + 1) / AVIUTL_SHAKE_RANDOM_SCALE * track1;
                let r2 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 3, p + 2) / AVIUTL_SHAKE_RANDOM_SCALE * track1;
                let r3 = aviutl_rand(-AVIUTL_SHAKE_RANDOM_SCALE, AVIUTL_SHAKE_RANDOM_SCALE, 3, p + 3) / AVIUTL_SHAKE_RANDOM_SCALE * track1;

                cam.tx += aviutl_interpolation(t_frac, x0, x1, x2, x3);
                cam.ty += aviutl_interpolation(t_frac, y0, y1, y2, y3);
                cam.tz += aviutl_interpolation(t_frac, z0, z1, z2, z3);
                cam.rz += aviutl_interpolation(t_frac, r0, r1, r2, r3);
            }
        }
        "目標中心回転" => {
            let x = cam.x - cam.tx;
            let y = cam.y - cam.ty;
            let z = cam.z - cam.tz;

            let vx = cam.ux;
            let vy = cam.uy;
            let vz = cam.uz;
            let r_h = track0.to_radians();
            let s_h = r_h.sin();
            let c_h = r_h.cos();
            let m00 = vx * vx * (1.0 - c_h) + c_h;
            let m01 = vx * vy * (1.0 - c_h) - vz * s_h;
            let m02 = vz * vx * (1.0 - c_h) + vy * s_h;
            let m10 = vx * vy * (1.0 - c_h) + vz * s_h;
            let m11 = vy * vy * (1.0 - c_h) + c_h;
            let m12 = vy * vz * (1.0 - c_h) - vx * s_h;
            let m20 = vz * vx * (1.0 - c_h) - vy * s_h;
            let m21 = vy * vz * (1.0 - c_h) + vx * s_h;
            let m22 = vz * vz * (1.0 - c_h) + c_h;

            let x2 = m00 * x + m01 * y + m02 * z;
            let y2 = m10 * x + m11 * y + m12 * z;
            let z2 = m20 * x + m21 * y + m22 * z;

            let l2 = (x2 * x2 + y2 * y2 + z2 * z2).sqrt();
            if l2 > 1e-6 {
                let n_vx = x2 / l2;
                let n_vy = y2 / l2;
                let n_vz = z2 / l2;

                let cr_x = n_vy * cam.uz - n_vz * cam.uy;
                let cr_y = n_vz * cam.ux - n_vx * cam.uz;
                let cr_z = n_vx * cam.uy - n_vy * cam.ux;
                let cr_l = (cr_x * cr_x + cr_y * cr_y + cr_z * cr_z).sqrt();
                if cr_l > 1e-6 {
                    let ax_x = cr_x / cr_l;
                    let ax_y = cr_y / cr_l;
                    let ax_z = cr_z / cr_l;

                    let r_v = track1.to_radians();
                    let s_v = r_v.sin();
                    let c_v = r_v.cos();

                    let vm00 = ax_x * ax_x * (1.0 - c_v) + c_v;
                    let vm01 = ax_x * ax_y * (1.0 - c_v) - ax_z * s_v;
                    let vm02 = ax_z * ax_x * (1.0 - c_v) + ax_y * s_v;
                    let vm10 = ax_x * ax_y * (1.0 - c_v) + ax_z * s_v;
                    let vm11 = ax_y * ax_y * (1.0 - c_v) + c_v;
                    let vm12 = ax_y * ax_z * (1.0 - c_v) - ax_x * s_v;
                    let vm20 = ax_z * ax_x * (1.0 - c_v) - ax_y * s_v;
                    let vm21 = ax_y * ax_z * (1.0 - c_v) + ax_x * s_v;
                    let vm22 = ax_z * ax_z * (1.0 - c_v) + c_v;

                    cam.x = cam.tx + vm00 * x2 + vm01 * y2 + vm02 * z2;
                    cam.y = cam.ty + vm10 * x2 + vm11 * y2 + vm12 * z2;
                    cam.z = cam.tz + vm20 * x2 + vm21 * y2 + vm22 * z2;

                    let ux = cam.ux;
                    let uy = cam.uy;
                    let uz = cam.uz;
                    cam.ux = vm00 * ux + vm01 * uy + vm02 * uz;
                    cam.uy = vm10 * ux + vm11 * uy + vm12 * uz;
                    cam.uz = vm20 * ux + vm21 * uy + vm22 * uz;
                }
            }
        }
        "目標サイズ固定視野角" => {
            let x = cam.tx - cam.x;
            let y = cam.ty - cam.y;
            let z = cam.tz - cam.z;
            let l = (x * x + y * y + z * z).sqrt();
            if l > 1e-6 && cam.d > 1e-6 {
                let norm_x = x / l;
                let norm_y = y / l;
                let norm_z = z / l;
                let r = l / cam.d;
                let mut p = (screen_h / (cam.d * 2.0)).atan() * 360.0 / std::f64::consts::PI;
                p = (p + track0).clamp(1.0, 120.0);
                cam.d = screen_h / ((p * std::f64::consts::PI / 360.0).tan() * 2.0);
                let new_l = r * cam.d;
                cam.x = cam.tx - new_l * norm_x;
                cam.y = cam.ty - new_l * norm_y;
                cam.z = cam.tz - new_l * norm_z;
            }
        }
        "ParallelCamera" => {
            let x = cam.tx - cam.x;
            let y = cam.ty - cam.y;
            let z = cam.tz - cam.z;
            let l = (x * x + y * y + z * z).sqrt();
            if l > 1.0 {
                cam.d = l;
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Camera3D {
    pub eye: [f64; 3],
    #[allow(dead_code)]
    pub target: [f64; 3],
    pub roll: f64,
    #[allow(dead_code)]
    pub fov: f64,
    pub focal_length: f64,
    pub forward: [f64; 3],
    pub right_prime: [f64; 3],
    pub up_prime: [f64; 3],
}

impl Camera3D {
    pub fn from_params(p: &CameraParams, fov: f64) -> Self {
        let dx = p.tx - p.x;
        let dy = p.ty - p.y;
        let dz = p.tz - p.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let forward = if dist < 1e-6 {
            [0.0, 0.0, 1.0]
        } else {
            [dx / dist, dy / dist, dz / dist]
        };

        // 上方向ベクトル
        let up_norm = (p.ux * p.ux + p.uy * p.uy + p.uz * p.uz).sqrt();
        let cam_up = if up_norm < 1e-6 {
            if forward[1].abs() > 0.999 {
                [0.0, 0.0, 1.0]
            } else {
                [0.0, -1.0, 0.0]
            }
        } else {
            [p.ux / up_norm, p.uy / up_norm, p.uz / up_norm]
        };

        // right = cross(forward, cam_up)
        let rx = forward[1] * cam_up[2] - forward[2] * cam_up[1];
        let ry = forward[2] * cam_up[0] - forward[0] * cam_up[2];
        let rz = forward[0] * cam_up[1] - forward[1] * cam_up[0];
        let r_len = (rx * rx + ry * ry + rz * rz).sqrt();
        let right = if r_len < 1e-6 {
            [1.0, 0.0, 0.0]
        } else {
            [rx / r_len, ry / r_len, rz / r_len]
        };

        // up = cross(right, forward)
        let ux = right[1] * forward[2] - right[2] * forward[1];
        let uy = right[2] * forward[0] - right[0] * forward[2];
        let uz = right[0] * forward[1] - right[1] * forward[0];
        let up = [ux, uy, uz];

        // ロール回転（傾き）
        let roll_rad = p.rz.to_radians();
        let cos_r = roll_rad.cos();
        let sin_r = roll_rad.sin();
        let right_prime = [
            right[0] * cos_r + up[0] * sin_r,
            right[1] * cos_r + up[1] * sin_r,
            right[2] * cos_r + up[2] * sin_r,
        ];
        let up_prime = [
            -right[0] * sin_r + up[0] * cos_r,
            -right[1] * sin_r + up[1] * cos_r,
            -right[2] * sin_r + up[2] * cos_r,
        ];

        Self {
            eye: [p.x, p.y, p.z],
            target: [p.tx, p.ty, p.tz],
            roll: p.rz,
            fov,
            focal_length: p.d,
            forward,
            right_prime,
            up_prime,
        }
    }

    #[allow(dead_code)]
    pub fn new(eye: [f64; 3], target: [f64; 3], roll: f64, fov: f64) -> Self {
        let dx = target[0] - eye[0];
        let dy = target[1] - eye[1];
        let dz = target[2] - eye[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let forward = if dist < 1e-6 {
            [0.0, 0.0, 1.0]
        } else {
            [dx / dist, dy / dist, dz / dist]
        };

        // ワールドの上方向：Y軸下向きが画面下なので、上方向は [0.0, -1.0, 0.0]
        let world_up = if forward[1].abs() > 0.999 {
            [0.0, 0.0, 1.0]
        } else {
            [0.0, -1.0, 0.0]
        };

        // right = cross(forward, world_up)
        let rx = forward[1] * world_up[2] - forward[2] * world_up[1];
        let ry = forward[2] * world_up[0] - forward[0] * world_up[2];
        let rz = forward[0] * world_up[1] - forward[1] * world_up[0];
        let r_len = (rx * rx + ry * ry + rz * rz).sqrt();
        let right = if r_len < 1e-6 {
            [1.0, 0.0, 0.0]
        } else {
            [rx / r_len, ry / r_len, rz / r_len]
        };

        // up = cross(right, forward)
        let ux = right[1] * forward[2] - right[2] * forward[1];
        let uy = right[2] * forward[0] - right[0] * forward[2];
        let uz = right[0] * forward[1] - right[1] * forward[0];
        let up = [ux, uy, uz];

        // ロール回転（傾き）
        let roll_rad = roll.to_radians();
        let cos_r = roll_rad.cos();
        let sin_r = roll_rad.sin();
        let right_prime = [
            right[0] * cos_r + up[0] * sin_r,
            right[1] * cos_r + up[1] * sin_r,
            right[2] * cos_r + up[2] * sin_r,
        ];
        let up_prime = [
            -right[0] * sin_r + up[0] * cos_r,
            -right[1] * sin_r + up[1] * cos_r,
            -right[2] * sin_r + up[2] * cos_r,
        ];

        // 焦点距離（AviUtl基準: 視野角0時は焦点距離1024.0）
        let focal_length = if fov > 0.1 {
            (1024.0 / 2.0) / (fov.to_radians() / 2.0).tan()
        } else {
            1024.0
        };

        Self {
            eye,
            target,
            roll,
            fov,
            focal_length,
            forward,
            right_prime,
            up_prime,
        }
    }

    /// 3D ワールド座標 [x, y, z] をカメラ座標系に変換し、透視投影した結果 (screen_x, screen_y, scale_3d, z_cam) を返す
    pub fn project(&self, p: [f64; 3]) -> Option<(f32, f32, f32, f64)> {
        let vx = p[0] - self.eye[0];
        let vy = p[1] - self.eye[1];
        let vz = p[2] - self.eye[2];

        let x_cam = vx * self.right_prime[0] + vy * self.right_prime[1] + vz * self.right_prime[2];
        let y_cam = -(vx * self.up_prime[0] + vy * self.up_prime[1] + vz * self.up_prime[2]);
        let z_cam = vx * self.forward[0] + vy * self.forward[1] + vz * self.forward[2];

        if z_cam <= 1.0 {
            // カメラの背面または近すぎるクリップ面
            return None;
        }

        let scale_3d = (self.focal_length / z_cam) as f32;
        let x_proj = (x_cam as f32) * scale_3d;
        let y_proj = (y_cam as f32) * scale_3d;

        Some((x_proj, y_proj, scale_3d, z_cam))
    }
}

pub(crate) fn rotate_3d(p: [f64; 3], rx_deg: f64, ry_deg: f64, rz_deg: f64) -> [f64; 3] {
    let rx = rx_deg.to_radians();
    let ry = ry_deg.to_radians();
    let rz = rz_deg.to_radians();

    // 1. X軸回転
    let cos_x = rx.cos();
    let sin_x = rx.sin();
    let x1 = p[0];
    let y1 = p[1] * cos_x - p[2] * sin_x;
    let z1 = p[1] * sin_x + p[2] * cos_x;

    // 2. Y軸回転
    let cos_y = ry.cos();
    let sin_y = ry.sin();
    let x2 = x1 * cos_y + z1 * sin_y;
    let y2 = y1;
    let z2 = -x1 * sin_y + z1 * cos_y;

    // 3. Z軸回転
    let cos_z = rz.cos();
    let sin_z = rz.sin();
    let x3 = x2 * cos_z - y2 * sin_z;
    let y3 = x2 * sin_z + y2 * cos_z;
    let z3 = z2;

    [x3, y3, z3]
}

pub(crate) fn project_3d_points(
    cam: &Camera3D,
    canvas: Rect,
    scale: f32,
    world_pos: [f64; 3],
    rot: [f64; 3],
    local_pts: &[[f32; 2]],
    zoom: f32,
) -> Option<Vec<Pos2>> {
    let mut out = Vec::with_capacity(local_pts.len());
    for &[lx, ly] in local_pts {
        let p_rot = rotate_3d([(lx * zoom) as f64, (ly * zoom) as f64, 0.0], rot[0], rot[1], rot[2]);
        let pw = [world_pos[0] + p_rot[0], world_pos[1] + p_rot[1], world_pos[2] + p_rot[2]];
        if let Some((sx, sy, _, _)) = cam.project(pw) {
            out.push(canvas.center() + vec2(sx, sy) * scale);
        } else {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aviutl_camera_effects_math() {
        let mut cam = CameraParams {
            x: 0.0,
            y: 0.0,
            z: -1024.0,
            tx: 0.0,
            ty: 0.0,
            tz: 0.0,
            rz: 0.0,
            ux: 0.0,
            uy: -1.0,
            uz: 0.0,
            d: 1024.0,
        };

        // 1. 目標中心回転の検証（水平90度回転）
        apply_camera_effect(&mut cam, "目標中心回転", 90.0, 0.0, 0.0, 0.0, 0.0, 720.0);
        assert!((cam.x - 1024.0).abs() < 1e-3, "水平90度回転でX軸へ移動すること (x={})", cam.x);
        assert!(cam.z.abs() < 1e-3, "水平90度回転でZが0付近になること (z={})", cam.z);

        // 2. 目標サイズ固定視野角の検証
        let orig_d = cam.d;
        apply_camera_effect(&mut cam, "目標サイズ固定視野角", 10.0, 0.0, 0.0, 0.0, 0.0, 720.0);
        assert!(cam.d != orig_d, "画角変更に伴い焦点距離dが更新されること");

        // 3. 手ブレ効果の検証
        let pre_tx = cam.tx;
        apply_camera_effect(&mut cam, "手ぶれ", 10.0, 5.0, 0.0, 0.0, 1.23, 720.0);
        assert!(cam.tx != pre_tx, "手ブレで目標位置が揺れること");
    }

    #[test]
    fn test_aviutl_rand_and_interpolation() {
        // 決定論的乱数
        let r1 = aviutl_rand(0.0, 100.0, 42, 10);
        let r2 = aviutl_rand(0.0, 100.0, 42, 10);
        assert_eq!(r1, r2, "同一シード・同一フレームで乱数値が一致すること");

        // スプライン補間 (t=0.0 で p1, t=1.0 で p2)
        let v0 = aviutl_interpolation(0.0, 0.0, 10.0, 20.0, 30.0);
        let v1 = aviutl_interpolation(1.0, 0.0, 10.0, 20.0, 30.0);
        assert!((v0 - 10.0).abs() < 1e-6);
        assert!((v1 - 20.0).abs() < 1e-6);
    }
}
