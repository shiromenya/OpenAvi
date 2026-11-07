//! 純 Rust 製 PSD / PSB (Photoshop Document) パーサー・レイヤー合成エンジン（PSDToolKit 互換）。
//!
//! Windows / macOS / Linux の全 OS で外部プロセスやネイティブ DLL に依存せず、
//! `.psd` / `.psb` ファイルを直接オブジェクトとして読み込み・レイヤー表示切替・目パチ・口パク合成します。
//!
//! ## 主な対応機能
//! - PSD (v1) / PSB (v2) ヘッダ・レイヤーツリー（フォルダ階層 `lsct` / `lsdk`）・クリッピングマスク解析
//! - Unicode レイヤー名 (`luni` UTF-16BE) および日本語 Shift_JIS パスカル文字列のデコード
//! - Raw (`0`) および PackBits RLE (`1`) 圧縮チャンネルの高速展開（8-bit / 16-bit 対応）
//! - ブレンドモード合成（通常・乗算・スクリーン・オーバーレイ・加算/リニアドッジ・比較暗/明・ハードライト・ソフトライト・差の絶対値・減算）
//! - `oov/aviutl_psdtoolkit` / `aviutl2_psdtoolkit2` 互換のレイヤー命名規則:
//!   - `*レイヤー名`: 強制表示（必須レイヤー）
//!   - `!レイヤー名`: 同階層フォルダ内での排他選択（ラジオボタン挙動）
//! - 目パチ（自動まばたき）・口パク（音声振幅連動 / 周期アニメーション）のレイヤー自動検出とフレーム評価
//! - `.pfv` (PSDToolKit Favorites) お気に入りプリセットファイルの解析
//! - レイヤー可視状態のハッシュによる合成済み [`RgbaFrame`] メモリキャッシュ（再生・書き出し時の再合成コストをゼロ化）

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::RgbaFrame;

/// PSD のブレンドモード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PsdBlendMode {
    PassThrough,
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    LinearDodge,
    ColorBurn,
    LinearBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Subtract,
}

impl PsdBlendMode {
    pub fn from_key(key: &[u8; 4]) -> Self {
        match key {
            b"pass" => Self::PassThrough,
            b"norm" => Self::Normal,
            b"mul " => Self::Multiply,
            b"scrn" => Self::Screen,
            b"over" => Self::Overlay,
            b"dark" => Self::Darken,
            b"lite" => Self::Lighten,
            b"div " => Self::ColorDodge,
            b"lddg" => Self::LinearDodge,
            b"idiv" => Self::ColorBurn,
            b"lbrn" => Self::LinearBurn,
            b"hLit" => Self::HardLight,
            b"sLit" => Self::SoftLight,
            b"diff" => Self::Difference,
            b"smud" => Self::Exclusion,
            b"fsub" => Self::Subtract,
            _ => Self::Normal,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PassThrough => "通過",
            Self::Normal => "通常",
            Self::Multiply => "乗算",
            Self::Screen => "スクリーン",
            Self::Overlay => "オーバーレイ",
            Self::Darken => "比較(暗)",
            Self::Lighten => "比較(明)",
            Self::ColorDodge => "覆い焼きカラー",
            Self::LinearDodge => "加算(覆い焼きリニア)",
            Self::ColorBurn => "焼き込みカラー",
            Self::LinearBurn => "焼き込みリニア",
            Self::HardLight => "ハードライト",
            Self::SoftLight => "ソフトライト",
            Self::Difference => "差の絶対値",
            Self::Exclusion => "除外",
            Self::Subtract => "減算",
        }
    }

    /// AviUtl の標準描画・拡張描画の合成モード番号（`"0"`〜`"9"`）に変換します。
    pub fn to_aviutl_blend_code(self) -> &'static str {
        match self {
            Self::PassThrough | Self::Normal => "0",
            Self::LinearDodge | Self::ColorDodge => "1",
            Self::Subtract | Self::ColorBurn | Self::LinearBurn => "2",
            Self::Multiply => "3",
            Self::Screen => "4",
            Self::Overlay | Self::HardLight | Self::SoftLight => "5",
            Self::Lighten => "6",
            Self::Darken => "7",
            Self::Difference | Self::Exclusion => "9",
        }
    }
}

/// PSD をレイヤーまたはフォルダ（パーツグループ）単位で分解した際の 1 パーツ情報。
#[derive(Debug, Clone)]
pub struct PsdDecomposedPart {
    /// タイムライン上に表示するパーツ名（例: `"*体"`, `"目"`, `"前髪"`）
    pub label: String,
    /// このパーツのみを表示する `psd_layers` パラメータ文字列（`*=0;@crop=l,t,r,b;...`）
    pub psd_layers_param: String,
    /// 切り抜き矩形 `[left, top, right, bottom]`（PSD キャンバス座標系）
    pub crop_rect: [u32; 4],
    /// PSD キャンバス中心からのパーツ中心オフセット `(offset_x, offset_y)`
    pub center_offset: (f64, f64),
    /// 標準描画に引き継ぐブレンドモード
    pub blend_mode: PsdBlendMode,
    /// 上のオブジェクト（下位レイヤーのベース）でクリッピングするか
    pub clipping: bool,
    /// 目パチ設定を引き継ぐ対象パーツか
    pub inherit_blink: bool,
    /// 口パク設定を引き継ぐ対象パーツか
    pub inherit_lipsync: bool,
}

/// レイヤーまたはフォルダの種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsdLayerKind {
    /// 通常のピクセル描画レイヤー
    Pixel,
    /// フォルダ（グループ）の開始ヘッダ（上から下へ走査した際の親ノード）
    FolderStart { open: bool },
    /// フォルダ（グループ）の終了マーカー（`</Layer group>` 等、UI 非表示）
    FolderEnd,
}

/// PSD 内の 1 レイヤー情報。
#[derive(Debug, Clone)]
pub struct PsdLayer {
    /// ボトムアップ配列内のインデックス (`0..layers.len()`)
    pub id: usize,
    /// 親フォルダの `id`（ルート直下なら `None`）
    pub parent_id: Option<usize>,
    /// ネストの深さ（ルート直下 = 0）
    pub depth: usize,
    /// PSD ファイル内の生レイヤー名（`!通常`, `*体` などプレフィックス込み）
    pub name: String,
    /// UI 表示用のクリーンな名前（先頭の `!` や `*` を除いた名前）
    pub display_name: String,
    /// ルートからのスラッシュ区切りパス（例: `"眉/!通常"`, `"目/!閉じ"`）
    pub path: String,
    /// レイヤー種別
    pub kind: PsdLayerKind,
    /// バウンディングボックス (上, 左, 下, 右)
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
    pub right: i32,
    /// 不透明度 (`0..=255`)
    pub opacity: u8,
    /// ブレンドモード
    pub blend_mode: PsdBlendMode,
    /// クリッピングマスク（下のベースレイヤーの不透明度でクリップ）
    pub clipping: bool,
    /// PSD 保存時の初期表示状態
    pub default_visible: bool,
    /// PSDToolKit の `*` 強制表示フラグ
    pub is_mandatory: bool,
    /// PSDToolKit の `!` 排他ラジオ選択フラグ
    pub is_radio: bool,
    /// レイヤー矩形 `(right - left) * (bottom - top) * 4` の RGBA ピクセルデータ
    pub pixels: Vec<u8>,
}

impl PsdLayer {
    #[inline]
    pub fn width(&self) -> u32 {
        (self.right - self.left).max(0) as u32
    }

    #[inline]
    pub fn height(&self) -> u32 {
        (self.bottom - self.top).max(0) as u32
    }
}

/// `.pfv` (PSDToolKit Favorites) のお気に入り項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsdFavoritePreset {
    pub name: String,
    /// 有効化するレイヤーパス一覧
    pub active_paths: Vec<String>,
}

/// 目パチ（自動まばたき）設定。
#[derive(Debug, Clone, PartialEq)]
pub struct PsdBlinkConfig {
    pub enabled: bool,
    /// まばたき間隔（秒、既定 4.0 秒）
    pub interval_sec: f32,
    /// 目を閉じているフレーム数（既定 3 フレーム）
    pub close_frames: u32,
    /// 開き目レイヤーのパス（空なら自動検出）
    pub open_path: String,
    /// 閉じ目レイヤーのパス（空なら自動検出）
    pub close_path: String,
}

impl Default for PsdBlinkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_sec: 4.0,
            close_frames: 3,
            open_path: String::new(),
            close_path: String::new(),
        }
    }
}

impl PsdBlinkConfig {
    pub fn parse(raw: &str) -> Self {
        let mut cfg = Self::default();
        let parts: Vec<&str> = raw.split(',').collect();
        if let Some(en) = parts.first() {
            cfg.enabled = en.trim() == "1" || en.trim().eq_ignore_ascii_case("true");
        }
        if let Some(iv) = parts.get(1).and_then(|s| s.trim().parse::<f32>().ok()) {
            cfg.interval_sec = iv.clamp(0.5, 30.0);
        }
        if let Some(cf) = parts.get(2).and_then(|s| s.trim().parse::<u32>().ok()) {
            cfg.close_frames = cf.clamp(1, 30);
        }
        if let Some(op) = parts.get(3) {
            cfg.open_path = op.trim().to_string();
        }
        if let Some(cp) = parts.get(4) {
            cfg.close_path = cp.trim().to_string();
        }
        cfg
    }

    pub fn to_param_string(&self) -> String {
        if !self.enabled && self.open_path.is_empty() && self.close_path.is_empty() {
            return String::new();
        }
        format!(
            "{},{:.2},{},{},{}",
            if self.enabled { 1 } else { 0 },
            self.interval_sec,
            self.close_frames,
            self.open_path,
            self.close_path
        )
    }
}

/// 口パク（リップシンク）モード。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsdLipSyncMode {
    /// 無効
    Off = 0,
    /// タイムライン上の音声トラックの振幅に自動連動
    AudioSync = 1,
    /// 一定フレーム周期で開閉を繰り返す
    Periodic = 2,
}

/// 口パク（リップシンク）設定。
#[derive(Debug, Clone, PartialEq)]
pub struct PsdLipSyncConfig {
    pub mode: PsdLipSyncMode,
    /// 感度（AudioSync 時: 0.1..=5.0、既定 1.0）または周期フレーム数（Periodic 時: 2..=30、既定 4.0）
    pub param: f32,
    /// 閉じ口レイヤーのパス（空なら自動検出）
    pub close_path: String,
    /// 開き口レイヤーのパス（空なら自動検出）
    pub open_path: String,
}

impl Default for PsdLipSyncConfig {
    fn default() -> Self {
        Self {
            mode: PsdLipSyncMode::Off,
            param: 1.0,
            close_path: String::new(),
            open_path: String::new(),
        }
    }
}

impl PsdLipSyncConfig {
    pub fn parse(raw: &str) -> Self {
        let mut cfg = Self::default();
        let parts: Vec<&str> = raw.split(',').collect();
        if let Some(m) = parts.first().and_then(|s| s.trim().parse::<u8>().ok()) {
            cfg.mode = match m {
                1 => PsdLipSyncMode::AudioSync,
                2 => PsdLipSyncMode::Periodic,
                _ => PsdLipSyncMode::Off,
            };
        }
        if let Some(p) = parts.get(1).and_then(|s| s.trim().parse::<f32>().ok()) {
            cfg.param = p.clamp(0.1, 60.0);
        }
        if let Some(cp) = parts.get(2) {
            cfg.close_path = cp.trim().to_string();
        }
        if let Some(op) = parts.get(3) {
            cfg.open_path = op.trim().to_string();
        }
        cfg
    }

    pub fn to_param_string(&self) -> String {
        if self.mode == PsdLipSyncMode::Off && self.close_path.is_empty() && self.open_path.is_empty() {
            return String::new();
        }
        format!(
            "{},{:.2},{},{}",
            self.mode as u8, self.param, self.close_path, self.open_path
        )
    }
}

/// 解析済み PSD ドキュメント。
pub struct PsdDocument {
    pub width: u32,
    pub height: u32,
    /// ボトムアップ順（0 = 最背面、len - 1 = 最前面）のレイヤー一覧
    pub layers: Vec<PsdLayer>,
    /// パス文字列 -> レイヤー ID の高速逆引きマップ
    pub path_to_id: HashMap<String, usize>,
    /// PSD 末尾に保存されていた統合画像（レイヤー0個の PSD 用フォールバック）
    pub merged_image: Option<Arc<RgbaFrame>>,
    /// 自動検出された目パチ候補 `(開き目パス, 閉じ目パス)`
    pub detected_blink: Option<(String, String)>,
    /// 自動検出された口パク候補 `(閉じ口パス, 開き口パス)`
    pub detected_lipsync: Option<(String, String)>,
    /// 隣接または同名の `.pfv` から読み込まれたお気に入りプリセット一覧
    pub favorites: Vec<PsdFavoritePreset>,
    /// 可視状態ハッシュ -> 合成済み `RgbaFrame` のキャッシュ
    composited_cache: Mutex<HashMap<u64, Arc<RgbaFrame>>>,
}

impl std::fmt::Debug for PsdDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PsdDocument")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("layers_count", &self.layers.len())
            .finish()
    }
}

/// PSD ファイルの先頭 26 バイトのみを読み取り、`(width, height)` を高速に返します。
pub fn probe_psd_dimensions(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 26];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"8BPS" {
        return None;
    }
    let version = u16::from_be_bytes([header[4], header[5]]);
    if version != 1 && version != 2 {
        return None;
    }
    let height = u32::from_be_bytes([header[14], header[15], header[16], header[17]]);
    let width = u32::from_be_bytes([header[18], header[19], header[20], header[21]]);
    if width == 0 || height == 0 || width > 30000 || height > 30000 {
        return None;
    }
    Some((width, height))
}

impl PsdDocument {
    /// ファイルパスから PSD ドキュメントを読み込みます（同名の `.pfv` ファイルがあれば自動で読み込みます）。
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("PSD ファイルを開けません ({}): {e}", path.display()))?;
        let mut doc = Self::from_bytes(&bytes)?;

        // 同階層にある `<stem>.pfv` または `<filename>.pfv` を自動検索して読み込む
        let mut pfv_candidates = Vec::new();
        pfv_candidates.push(path.with_extension("pfv"));
        if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
            if let Some(parent) = path.parent() {
                pfv_candidates.push(parent.join(format!("{fname}.pfv")));
            }
        }
        for pfv_path in pfv_candidates {
            if pfv_path.is_file() {
                if let Ok(pfv_bytes) = std::fs::read(&pfv_path) {
                    doc.favorites = parse_pfv_bytes(&pfv_bytes);
                    break;
                }
            }
        }

        Ok(doc)
    }

    /// バイト列から PSD / PSB ドキュメントを解析します。
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        if data.len() < 26 || &data[0..4] != b"8BPS" {
            return Err("有効な PSD ヘッダ (8BPS) が見つかりません".into());
        }
        let version = read_u16_be(data, 4)?;
        if version != 1 && version != 2 {
            return Err(format!("未対応の PSD バージョンです: {version}"));
        }
        let is_psb = version == 2;
        let num_channels = read_u16_be(data, 12)? as usize;
        let height = read_u32_be(data, 14)?;
        let width = read_u32_be(data, 18)?;
        let depth = read_u16_be(data, 22)?;
        let color_mode = read_u16_be(data, 24)?;

        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(format!("不正な PSD キャンバスサイズです: {width}x{height}"));
        }
        if depth != 8 && depth != 16 {
            return Err(format!("未対応のビット深度です: {depth}-bit (8-bit または 16-bit のみ対応)"));
        }
        if color_mode != 3 && color_mode != 1 {
            return Err(format!("未対応のカラーモードです: {color_mode} (RGB またはグレースケールのみ対応)"));
        }

        let mut pos = 26usize;

        // 1. Color Mode Data Section
        let color_mode_len = read_u32_be(data, pos)? as usize;
        pos = pos.checked_add(4 + color_mode_len).ok_or("PSD Color Mode セクションが破損しています")?;

        // 2. Image Resources Section
        let img_res_len = read_u32_be(data, pos)? as usize;
        pos = pos.checked_add(4 + img_res_len).ok_or("PSD Image Resources セクションが破損しています")?;

        // 3. Layer and Mask Information Section
        let layer_mask_len = if is_psb {
            let l = read_u64_be(data, pos)? as usize;
            pos += 8;
            l
        } else {
            let l = read_u32_be(data, pos)? as usize;
            pos += 4;
            l
        };
        let layer_mask_end = pos
            .checked_add(layer_mask_len)
            .ok_or("PSD Layer and Mask セクション長が不正です")?
            .min(data.len());

        let mut layers = Vec::new();
        if layer_mask_len > 0 && pos + 4 <= layer_mask_end {
            let layer_info_len = if is_psb {
                let l = read_u64_be(data, pos)? as usize;
                pos += 8;
                l
            } else {
                let l = read_u32_be(data, pos)? as usize;
                pos += 4;
                l
            };

            if layer_info_len > 0 && pos + 2 <= layer_mask_end {
                let raw_layer_count = read_i16_be(data, pos)?;
                pos += 2;
                let layer_count = raw_layer_count.unsigned_abs() as usize;

                struct RawLayerRecord {
                    top: i32,
                    left: i32,
                    bottom: i32,
                    right: i32,
                    channels: Vec<(i16, usize)>,
                    blend_mode: PsdBlendMode,
                    opacity: u8,
                    clipping: bool,
                    default_visible: bool,
                    name: String,
                    kind: PsdLayerKind,
                    mask_rect: Option<(i32, i32, i32, i32, u8, bool)>,
                }

                let mut records = Vec::with_capacity(layer_count);

                for _ in 0..layer_count {
                    if pos + 18 > layer_mask_end {
                        break;
                    }
                    let top = read_i32_be(data, pos)?;
                    let left = read_i32_be(data, pos + 4)?;
                    let bottom = read_i32_be(data, pos + 8)?;
                    let right = read_i32_be(data, pos + 12)?;
                    let ch_count = read_u16_be(data, pos + 16)? as usize;
                    pos += 18;

                    let mut channels = Vec::with_capacity(ch_count);
                    for _ in 0..ch_count {
                        let ch_id = read_i16_be(data, pos)?;
                        pos += 2;
                        let ch_len = if is_psb {
                            let l = read_u64_be(data, pos)? as usize;
                            pos += 8;
                            l
                        } else {
                            let l = read_u32_be(data, pos)? as usize;
                            pos += 4;
                            l
                        };
                        channels.push((ch_id, ch_len));
                    }

                    if pos + 16 > layer_mask_end {
                        break;
                    }
                    // Blend mode signature ("8BIM") + key (4 bytes)
                    let blend_key: [u8; 4] = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
                    let blend_mode = PsdBlendMode::from_key(&blend_key);
                    let opacity = data[pos + 8];
                    let clipping = data[pos + 9] != 0;
                    let flags = data[pos + 10];
                    // bit 1 (0x02) = invisible
                    let default_visible = (flags & 0x02) == 0;
                    let extra_len = read_u32_be(data, pos + 12)? as usize;
                    pos += 16;

                    let extra_end = (pos + extra_len).min(layer_mask_end);
                    let mut ex_pos = pos;

                    // 3a. Layer mask data
                    let mut mask_rect = None;
                    if ex_pos + 4 <= extra_end {
                        let mask_len = read_u32_be(data, ex_pos)? as usize;
                        ex_pos += 4;
                        if mask_len >= 20 && ex_pos + 20 <= extra_end {
                            let m_top = read_i32_be(data, ex_pos)?;
                            let m_left = read_i32_be(data, ex_pos + 4)?;
                            let m_bottom = read_i32_be(data, ex_pos + 8)?;
                            let m_right = read_i32_be(data, ex_pos + 12)?;
                            let default_color = data[ex_pos + 16];
                            let m_flags = data[ex_pos + 17];
                            let mask_disabled = (m_flags & 0x02) != 0;
                            if !mask_disabled && m_right > m_left && m_bottom > m_top {
                                mask_rect = Some((m_top, m_left, m_bottom, m_right, default_color, (m_flags & 0x04) != 0));
                            }
                        }
                        ex_pos = (ex_pos + mask_len).min(extra_end);
                    }

                    // 3b. Layer blending ranges
                    if ex_pos + 4 <= extra_end {
                        let ranges_len = read_u32_be(data, ex_pos)? as usize;
                        ex_pos = (ex_pos + 4 + ranges_len).min(extra_end);
                    }

                    // 3c. Pascal layer name (padded to 4-byte boundary)
                    let mut layer_name = String::new();
                    if ex_pos < extra_end {
                        let name_len = data[ex_pos] as usize;
                        let name_start = ex_pos + 1;
                        let name_end = (name_start + name_len).min(extra_end);
                        let raw_name = &data[name_start..name_end];
                        layer_name = decode_psd_string(raw_name);
                        let padded = ((1 + name_len + 3) / 4) * 4;
                        ex_pos = (ex_pos + padded).min(extra_end);
                    }

                    // 3d. Additional Layer Information ("luni", "lsct", "lsdk")
                    let mut kind = PsdLayerKind::Pixel;
                    let mut folder_blend_override = None;
                    while ex_pos + 12 <= extra_end {
                        let sig = &data[ex_pos..ex_pos + 4];
                        if sig != b"8BIM" && sig != b"8B64" {
                            break;
                        }
                        let key = &data[ex_pos + 4..ex_pos + 8];
                        ex_pos += 8;
                        let is_64bit_len = is_psb
                            && matches!(
                                key,
                                b"LMsk" | b"Lr16" | b"Lr32" | b"Layr" | b"Mt16" | b"Mt32" | b"Mtrn" | b"Alph" | b"FMsk" | b"lnk2" | b"FEid" | b"FXid" | b"PxSD"
                            );
                        let block_len = if is_64bit_len {
                            if ex_pos + 8 > extra_end {
                                break;
                            }
                            let l = read_u64_be(data, ex_pos)? as usize;
                            ex_pos += 8;
                            l
                        } else {
                            let l = read_u32_be(data, ex_pos)? as usize;
                            ex_pos += 4;
                            l
                        };
                        let block_end = (ex_pos + block_len).min(extra_end);
                        let block_data = &data[ex_pos..block_end];

                        if key == b"luni" && block_data.len() >= 4 {
                            let char_count = u32::from_be_bytes([
                                block_data[0],
                                block_data[1],
                                block_data[2],
                                block_data[3],
                            ]) as usize;
                            let byte_len = (char_count * 2).min(block_data.len() - 4);
                            let mut u16_buf = Vec::with_capacity(byte_len / 2);
                            for chunk in block_data[4..4 + byte_len].chunks_exact(2) {
                                let code = u16::from_be_bytes([chunk[0], chunk[1]]);
                                if code != 0 {
                                    u16_buf.push(code);
                                }
                            }
                            if let Ok(s) = String::from_utf16(&u16_buf) {
                                if !s.is_empty() {
                                    layer_name = s;
                                }
                            }
                        } else if (key == b"lsct" || key == b"lsdk") && block_data.len() >= 4 {
                            let div_type = u32::from_be_bytes([
                                block_data[0],
                                block_data[1],
                                block_data[2],
                                block_data[3],
                            ]);
                            match div_type {
                                1 => kind = PsdLayerKind::FolderStart { open: true },
                                2 => kind = PsdLayerKind::FolderStart { open: false },
                                3 => kind = PsdLayerKind::FolderEnd,
                                _ => {}
                            }
                            if block_data.len() >= 12 && &block_data[4..8] == b"8BIM" {
                                let bkey: [u8; 4] = [block_data[8], block_data[9], block_data[10], block_data[11]];
                                folder_blend_override = Some(PsdBlendMode::from_key(&bkey));
                            }
                        }

                        // パディング（偶数または4バイト境界）をスキップ
                        let padded_len = if block_len % 2 != 0 { block_len + 1 } else { block_len };
                        ex_pos = (ex_pos + padded_len).min(extra_end);
                    }

                    pos = extra_end;
                    records.push(RawLayerRecord {
                        top,
                        left,
                        bottom,
                        right,
                        channels,
                        blend_mode: folder_blend_override.unwrap_or(blend_mode),
                        opacity,
                        clipping,
                        default_visible,
                        name: layer_name,
                        kind,
                        mask_rect,
                    });
                }

                // 3e. Channel Image Data for each layer
                for (id, rec) in records.into_iter().enumerate() {
                    let lw = (rec.right - rec.left).max(0) as usize;
                    let lh = (rec.bottom - rec.top).max(0) as usize;
                    let pixel_count = lw * lh;

                    let mut r_ch: Option<Vec<u8>> = None;
                    let mut g_ch: Option<Vec<u8>> = None;
                    let mut b_ch: Option<Vec<u8>> = None;
                    let mut a_ch: Option<Vec<u8>> = None;
                    let mut mask_ch: Option<Vec<u8>> = None;

                    for &(ch_id, ch_len) in &rec.channels {
                        let ch_end = (pos + ch_len).min(layer_mask_end);
                        let ch_slice = if pos <= ch_end { &data[pos..ch_end] } else { &[] };
                        pos = ch_end;

                        let (target_w, target_h) = if ch_id == -2 {
                            if let Some((mt, ml, mb, mr, _, _)) = rec.mask_rect {
                                ((mr - ml).max(0) as usize, (mb - mt).max(0) as usize)
                            } else {
                                (0, 0)
                            }
                        } else {
                            (lw, lh)
                        };

                        if target_w == 0 || target_h == 0 || rec.kind != PsdLayerKind::Pixel {
                            continue;
                        }

                        if let Some(decoded) = decode_channel_slice(ch_slice, target_w, target_h, depth, is_psb) {
                            match ch_id {
                                0 => r_ch = Some(decoded),
                                1 => g_ch = Some(decoded),
                                2 => b_ch = Some(decoded),
                                -1 => a_ch = Some(decoded),
                                -2 => mask_ch = Some(decoded),
                                _ => {}
                            }
                        }
                    }

                    let mut rgba = Vec::new();
                    if rec.kind == PsdLayerKind::Pixel && pixel_count > 0 {
                        rgba.resize(pixel_count * 4, 0u8);
                        if color_mode == 1 {
                            // Grayscale
                            let gray = r_ch.as_deref();
                            let alpha = a_ch.as_deref();
                            for i in 0..pixel_count {
                                let v = gray.and_then(|c| c.get(i).copied()).unwrap_or(0);
                                let a = alpha.and_then(|c| c.get(i).copied()).unwrap_or(255);
                                rgba[i * 4] = v;
                                rgba[i * 4 + 1] = v;
                                rgba[i * 4 + 2] = v;
                                rgba[i * 4 + 3] = a;
                            }
                        } else {
                            // RGB
                            let r = r_ch.as_deref();
                            let g = g_ch.as_deref();
                            let b = b_ch.as_deref();
                            let a = a_ch.as_deref();
                            for i in 0..pixel_count {
                                rgba[i * 4] = r.and_then(|c| c.get(i).copied()).unwrap_or(0);
                                rgba[i * 4 + 1] = g.and_then(|c| c.get(i).copied()).unwrap_or(0);
                                rgba[i * 4 + 2] = b.and_then(|c| c.get(i).copied()).unwrap_or(0);
                                rgba[i * 4 + 3] = a.and_then(|c| c.get(i).copied()).unwrap_or(255);
                            }
                        }

                        // ユーザーレイヤーマスク (-2) が存在する場合はアルファに乗算
                        if let (Some(m_buf), Some((mt, ml, mb, mr, def_col, invert))) =
                            (mask_ch.as_deref(), rec.mask_rect)
                        {
                            let mw = (mr - ml).max(0) as usize;
                            for y in 0..lh {
                                let cy = rec.top + y as i32;
                                for x in 0..lw {
                                    let cx = rec.left + x as i32;
                                    let mut mv = if cx >= ml && cx < mr && cy >= mt && cy < mb {
                                        let mx = (cx - ml) as usize;
                                        let my = (cy - mt) as usize;
                                        m_buf.get(my * mw + mx).copied().unwrap_or(def_col)
                                    } else {
                                        def_col
                                    };
                                    if invert {
                                        mv = 255 - mv;
                                    }
                                    let idx = (y * lw + x) * 4 + 3;
                                    rgba[idx] = ((rgba[idx] as u16 * mv as u16 + 127) / 255) as u8;
                                }
                            }
                        }
                    }

                    let (is_mandatory, is_radio, display_name) = parse_psdtoolkit_prefix(&rec.name);

                    layers.push(PsdLayer {
                        id,
                        parent_id: None,
                        depth: 0,
                        name: rec.name,
                        display_name,
                        path: String::new(),
                        kind: rec.kind,
                        top: rec.top,
                        left: rec.left,
                        bottom: rec.bottom,
                        right: rec.right,
                        opacity: rec.opacity,
                        blend_mode: rec.blend_mode,
                        clipping: rec.clipping,
                        default_visible: rec.default_visible || is_mandatory,
                        is_mandatory,
                        is_radio,
                        pixels: rgba,
                    });
                }
            }
        }

        // 4. トップダウン走査（末尾から先頭へ）でフォルダ階層・親ID・フルパスを構築
        let mut folder_stack: Vec<usize> = Vec::new();
        for idx in (0..layers.len()).rev() {
            let kind = layers[idx].kind;
            match kind {
                PsdLayerKind::FolderStart { .. } => {
                    let parent = folder_stack.last().copied();
                    let depth = folder_stack.len();
                    let path = build_layer_path(&layers, &folder_stack, &layers[idx].name);
                    layers[idx].parent_id = parent;
                    layers[idx].depth = depth;
                    layers[idx].path = path;
                    folder_stack.push(idx);
                }
                PsdLayerKind::FolderEnd => {
                    let parent = folder_stack.last().copied();
                    let depth = folder_stack.len();
                    layers[idx].parent_id = parent;
                    layers[idx].depth = depth;
                    folder_stack.pop();
                }
                PsdLayerKind::Pixel => {
                    let parent = folder_stack.last().copied();
                    let depth = folder_stack.len();
                    let path = build_layer_path(&layers, &folder_stack, &layers[idx].name);
                    layers[idx].parent_id = parent;
                    layers[idx].depth = depth;
                    layers[idx].path = path;
                }
            }
        }

        // パス重複がある場合（同名レイヤーが同フォルダ内にある場合）の曖昧さを防ぎつつ逆引きマップを構築
        let mut path_to_id = HashMap::with_capacity(layers.len());
        for l in &layers {
            if !l.path.is_empty() && l.kind != PsdLayerKind::FolderEnd {
                path_to_id.insert(l.path.clone(), l.id);
            }
        }

        // 5. 末尾の Merged Image Data セクション（レイヤーが無い場合のフォールバック用）
        pos = layer_mask_end;
        let merged_image = if pos + 2 <= data.len() {
            decode_merged_image(&data[pos..], width as usize, height as usize, num_channels, depth, color_mode, is_psb)
        } else {
            None
        };

        let detected_blink = detect_blink_layers(&layers);
        let detected_lipsync = detect_lipsync_layers(&layers);

        Ok(Self {
            width,
            height,
            layers,
            path_to_id,
            merged_image,
            detected_blink,
            detected_lipsync,
            favorites: Vec::new(),
            composited_cache: Mutex::new(HashMap::new()),
        })
    }

    /// 初期状態のレイヤー可視配列（`layers.len()` 要素）を返します。
    pub fn default_visibility(&self) -> Vec<bool> {
        self.layers
            .iter()
            .map(|l| l.default_visible || l.is_mandatory)
            .collect()
    }

    /// `psd_layers` オーバーライド文字列（`path=1;path=0`、`*=0`、`@crop=l,t,r,b`）を適用した可視配列を計算します。
    pub fn compute_visibility_from_overrides(&self, overrides_str: &str) -> Vec<bool> {
        let mut vis = self.default_visibility();
        if overrides_str.trim().is_empty() {
            return vis;
        }
        let mut isolated_mode = false;
        for token in overrides_str.split(';') {
            let token = token.trim();
            if token.is_empty() || token.starts_with("@crop=") {
                continue;
            }
            if token == "*=0" {
                vis.fill(false);
                isolated_mode = true;
                continue;
            }
            if let Some((path, val_str)) = token.split_once('=') {
                let on = val_str.trim() == "1" || val_str.trim().eq_ignore_ascii_case("true");
                if isolated_mode {
                    self.set_layer_path_visible_exact(&mut vis, path.trim(), on);
                } else {
                    self.set_layer_path_visible(&mut vis, path.trim(), on);
                }
            } else {
                // パスのみ記述されている場合は ON（かつラジオ排他適用）
                self.set_layer_path_visible(&mut vis, token, true);
            }
        }
        if !isolated_mode {
            for l in &self.layers {
                if l.is_mandatory {
                    vis[l.id] = true;
                }
            }
        }
        vis
    }

    /// `overrides_str` 内に `@crop=left,top,right,bottom` が含まれていれば切り抜き矩形を返します。
    pub fn parse_crop_rect(&self, overrides_str: &str) -> Option<[u32; 4]> {
        for token in overrides_str.split(';') {
            let token = token.trim();
            if let Some(coords_str) = token.strip_prefix("@crop=") {
                let mut nums = coords_str.split(',').filter_map(|s| s.trim().parse::<u32>().ok());
                let l = nums.next()?.min(self.width);
                let t = nums.next()?.min(self.height);
                let r = nums.next()?.min(self.width);
                let b = nums.next()?.min(self.height);
                if r > l && b > t {
                    return Some([l, t, r, b]);
                }
            }
        }
        None
    }

    /// `overrides_str` が分解済みの単独パーツ指定（`*=0` を含む）かどうかを返します。
    pub fn is_isolated_spec(overrides_str: &str) -> bool {
        overrides_str.split(';').any(|t| t.trim() == "*=0")
    }

    fn set_layer_path_visible_exact(&self, vis: &mut [bool], path: &str, on: bool) {
        let normalized = path.trim().trim_start_matches("v1.");
        let found_id = self.path_to_id.get(normalized).copied().or_else(|| {
            let clean_target = strip_prefixes_in_path(normalized);
            self.layers
                .iter()
                .find(|l| l.kind != PsdLayerKind::FolderEnd && strip_prefixes_in_path(&l.path) == clean_target)
                .map(|l| l.id)
        });
        if let Some(id) = found_id {
            vis[id] = on;
            if on {
                let mut cur_parent = self.layers.get(id).and_then(|l| l.parent_id);
                while let Some(pid) = cur_parent {
                    vis[pid] = true;
                    cur_parent = self.layers.get(pid).and_then(|p| p.parent_id);
                }
            }
        }
    }

    /// 特定のレイヤー（ID指定）の表示状態を切り替え、PSDToolKit の `!` ラジオ排他規則（またはフォルダ内単一選択）を適用します。
    pub fn set_layer_id_visible(&self, vis: &mut [bool], layer_id: usize, on: bool, force_radio_in_folder: bool) {
        let Some(target) = self.layers.get(layer_id) else {
            return;
        };
        if target.is_mandatory {
            vis[layer_id] = true;
            return;
        }
        if on && (target.is_radio || force_radio_in_folder) {
            let parent = target.parent_id;
            for sibling in &self.layers {
                if sibling.parent_id == parent
                    && sibling.id != layer_id
                    && sibling.kind != PsdLayerKind::FolderEnd
                    && !sibling.is_mandatory
                    && (sibling.is_radio || force_radio_in_folder)
                {
                    vis[sibling.id] = false;
                }
            }
        }
        vis[layer_id] = on;
        // 子レイヤーを持つ親フォルダがまだ非表示だった場合、子の ON 時に親フォルダも自動的に ON にする
        if on {
            let mut cur_parent = target.parent_id;
            while let Some(pid) = cur_parent {
                vis[pid] = true;
                cur_parent = self.layers.get(pid).and_then(|p| p.parent_id);
            }
        }
    }

    /// パス指定でレイヤーの表示状態を切り替えます（クリーン名一致や部分パス一致にも柔軟に対応）。
    pub fn set_layer_path_visible(&self, vis: &mut [bool], path: &str, on: bool) {
        let normalized = path.trim().trim_start_matches("v1.");
        if let Some(&id) = self.path_to_id.get(normalized) {
            // 同フォルダ内に複数のピクセルレイヤーがあるパーツ切替では、ON 指定時に同フォルダの兄弟パーツを排他切替
            let has_siblings = self.folder_has_multiple_parts(self.layers[id].parent_id);
            self.set_layer_id_visible(vis, id, on, on && has_siblings);
            return;
        }
        // `!` や `*` を除いたクリーンパスでも検索
        let clean_target = strip_prefixes_in_path(normalized);
        if let Some(found) = self
            .layers
            .iter()
            .find(|l| l.kind != PsdLayerKind::FolderEnd && strip_prefixes_in_path(&l.path) == clean_target)
        {
            let has_siblings = self.folder_has_multiple_parts(found.parent_id);
            self.set_layer_id_visible(vis, found.id, on, on && has_siblings);
        }
    }

    fn folder_has_multiple_parts(&self, parent_id: Option<usize>) -> bool {
        let Some(pid) = parent_id else {
            return false;
        };
        let count = self
            .layers
            .iter()
            .filter(|l| l.parent_id == Some(pid) && l.kind == PsdLayerKind::Pixel && !l.clipping)
            .count();
        count >= 2
    }

    /// 現在の可視配列 `vis` を、初期状態からの差分文字列（`psd_layers` パラメータ用）にシリアライズします。
    pub fn encode_visibility_overrides(&self, vis: &[bool]) -> String {
        self.encode_visibility_overrides_preserving(vis, "")
    }

    /// 既存の `prev_overrides` に含まれる分解フラグ（`*=0`）や切り抜き範囲（`@crop=...`）を維持しながら可視配列をシリアライズします。
    pub fn encode_visibility_overrides_preserving(&self, vis: &[bool], prev_overrides: &str) -> String {
        let any_mandatory_off = self
            .layers
            .iter()
            .any(|l| l.is_mandatory && !vis.get(l.id).copied().unwrap_or(false));
        let isolated = Self::is_isolated_spec(prev_overrides) || any_mandatory_off;
        let crop = self.parse_crop_rect(prev_overrides);

        let mut tokens = Vec::new();
        if isolated {
            tokens.push("*=0".to_string());
        }
        if let Some([l, t, r, b]) = crop {
            tokens.push(format!("@crop={l},{t},{r},{b}"));
        }

        if isolated {
            for l in &self.layers {
                if l.kind == PsdLayerKind::FolderEnd || l.path.is_empty() {
                    continue;
                }
                if vis.get(l.id).copied().unwrap_or(false) {
                    tokens.push(format!("{}=1", l.path));
                }
            }
        } else {
            let default_vis = self.default_visibility();
            for l in &self.layers {
                if l.kind == PsdLayerKind::FolderEnd || l.path.is_empty() {
                    continue;
                }
                let cur = vis.get(l.id).copied().unwrap_or(l.default_visible);
                let def = default_vis.get(l.id).copied().unwrap_or(l.default_visible);
                if cur != def {
                    tokens.push(format!("{}={}", l.path, if cur { 1 } else { 0 }));
                }
            }
        }
        tokens.join(";")
    }

    /// ベース可視状態に「目パチ」「口パク」のフレーム評価を適用した最終可視配列を返します。
    pub fn evaluate_animated_visibility(
        &self,
        base_overrides: &str,
        blink: &PsdBlinkConfig,
        lipsync: &PsdLipSyncConfig,
        rel_frame: u32,
        fps: f64,
        audio_amplitude: f32,
    ) -> Vec<bool> {
        let mut vis = self.compute_visibility_from_overrides(base_overrides);

        // 1. 目パチ (Auto Blink) 評価
        if blink.enabled {
            let (open_p, close_p) = self.resolve_blink_paths(blink);
            if !close_p.is_empty() {
                let interval_frames = ((blink.interval_sec.max(0.5) as f64) * fps.max(1.0)).round() as u32;
                let close_f = blink.close_frames.clamp(1, 30);
                let cycle = interval_frames.max(close_f + 2);
                // 開始直後にいきなり閉じないよう cycle / 2 オフセットを持たせる
                let phase = (rel_frame + cycle / 2) % cycle;
                let is_closed = phase < close_f;
                if is_closed {
                    if !open_p.is_empty() {
                        self.set_layer_path_visible(&mut vis, &open_p, false);
                    }
                    self.set_layer_path_visible(&mut vis, &close_p, true);
                } else if !open_p.is_empty() {
                    self.set_layer_path_visible(&mut vis, &close_p, false);
                    self.set_layer_path_visible(&mut vis, &open_p, true);
                }
            }
        }

        // 2. 口パク (Lip Sync) 評価
        if lipsync.mode != PsdLipSyncMode::Off {
            let (close_p, open_p) = self.resolve_lipsync_paths(lipsync);
            if !open_p.is_empty() {
                let is_mouth_open = match lipsync.mode {
                    PsdLipSyncMode::Off => false,
                    PsdLipSyncMode::AudioSync => {
                        let threshold = (0.04 / lipsync.param.max(0.1)).clamp(0.005, 0.5);
                        audio_amplitude >= threshold
                    }
                    PsdLipSyncMode::Periodic => {
                        let period = (lipsync.param.round() as u32).clamp(2, 60);
                        (rel_frame / period) % 2 == 1
                    }
                };
                if is_mouth_open {
                    if !close_p.is_empty() {
                        self.set_layer_path_visible(&mut vis, &close_p, false);
                    }
                    self.set_layer_path_visible(&mut vis, &open_p, true);
                } else if !close_p.is_empty() {
                    self.set_layer_path_visible(&mut vis, &open_p, false);
                    self.set_layer_path_visible(&mut vis, &close_p, true);
                }
            }
        }

        vis
    }

    pub fn resolve_blink_paths(&self, blink: &PsdBlinkConfig) -> (String, String) {
        let open_p = if !blink.open_path.is_empty() {
            blink.open_path.clone()
        } else {
            self.detected_blink.as_ref().map(|(o, _)| o.clone()).unwrap_or_default()
        };
        let close_p = if !blink.close_path.is_empty() {
            blink.close_path.clone()
        } else {
            self.detected_blink.as_ref().map(|(_, c)| c.clone()).unwrap_or_default()
        };
        (open_p, close_p)
    }

    pub fn resolve_lipsync_paths(&self, lipsync: &PsdLipSyncConfig) -> (String, String) {
        let close_p = if !lipsync.close_path.is_empty() {
            lipsync.close_path.clone()
        } else {
            self.detected_lipsync.as_ref().map(|(c, _)| c.clone()).unwrap_or_default()
        };
        let open_p = if !lipsync.open_path.is_empty() {
            lipsync.open_path.clone()
        } else {
            self.detected_lipsync.as_ref().map(|(_, o)| o.clone()).unwrap_or_default()
        };
        (close_p, open_p)
    }

    /// 指定した可視状態配列 `vis` に基づいて全レイヤーを合成し、キャッシュされた [`Arc<RgbaFrame>`] と状態ハッシュ `u64` を返します。
    pub fn composite_with_visibility(&self, vis: &[bool]) -> (u64, Arc<RgbaFrame>) {
        self.composite_with_visibility_and_crop(vis, None)
    }

    /// 指定した可視状態配列 `vis` とオプションの切り抜き矩形 `crop` (`[left, top, right, bottom]`) に基づいてレイヤーを合成します。
    pub fn composite_with_visibility_and_crop(
        &self,
        vis: &[bool],
        crop: Option<[u32; 4]>,
    ) -> (u64, Arc<RgbaFrame>) {
        // レイヤーが 0 個の PSD の場合は統合画像を返す
        if self.layers.is_empty() {
            if let Some(ref merged) = self.merged_image {
                return (0, Arc::clone(merged));
            }
            let w = self.width;
            let h = self.height;
            return (
                0,
                Arc::new(RgbaFrame {
                    width: w,
                    height: h,
                    original_width: w,
                    original_height: h,
                    pixels: vec![0u8; (w * h * 4) as usize],
                }),
            );
        }

        // 各ピクセルレイヤーの「実効可視状態（親フォルダの可視状態を含む）」を計算してハッシュ化
        let mut effective_vis = vec![false; self.layers.len()];
        for l in &self.layers {
            if l.kind != PsdLayerKind::Pixel {
                continue;
            }
            let mut is_vis = vis.get(l.id).copied().unwrap_or(l.default_visible);
            let mut cur_parent = l.parent_id;
            while is_vis && cur_parent.is_some() {
                let pid = cur_parent.unwrap();
                if let Some(p_layer) = self.layers.get(pid) {
                    let p_vis = vis.get(pid).copied().unwrap_or(p_layer.default_visible);
                    if !p_vis {
                        is_vis = false;
                        break;
                    }
                    cur_parent = p_layer.parent_id;
                } else {
                    break;
                }
            }
            effective_vis[l.id] = is_vis;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        effective_vis.hash(&mut hasher);
        crop.hash(&mut hasher);
        let state_hash = hasher.finish();

        if let Ok(cache) = self.composited_cache.lock() {
            if let Some(cached) = cache.get(&state_hash) {
                return (state_hash, Arc::clone(cached));
            }
        }

        let (cl, ct, cr, cb) = match crop {
            Some([l, t, r, b]) if r > l && b > t && r <= self.width && b <= self.height => {
                (l as i32, t as i32, r as i32, b as i32)
            }
            _ => (0, 0, self.width as i32, self.height as i32),
        };
        let cw = (cr - cl).max(1) as usize;
        let ch = (cb - ct).max(1) as usize;
        let mut canvas = vec![0u8; cw * ch * 4];

        // 分解された単一のクリッピングレイヤーのみが表示されている場合は、タイムライン側のクリッピングに委ねるため内部マスクを無効化
        let any_unclipped_vis = self
            .layers
            .iter()
            .any(|l| l.kind == PsdLayerKind::Pixel && effective_vis[l.id] && !l.clipping);

        // クリッピングマスクのベースとなる直近の非クリッピングレイヤーのアルファマップ（必要時のみ保持）
        let mut clip_base_alpha: Option<(i32, i32, i32, i32, Vec<u8>)> = None;

        for (idx, layer) in self.layers.iter().enumerate() {
            if layer.kind != PsdLayerKind::Pixel {
                continue;
            }

            let next_is_clipped = self
                .layers
                .get(idx + 1)
                .is_some_and(|nl| nl.kind == PsdLayerKind::Pixel && nl.clipping);

            if !layer.clipping {
                clip_base_alpha = None;
            }

            if !effective_vis[idx] {
                continue;
            }

            // 親フォルダの不透明度を乗算
            let mut eff_opacity = layer.opacity as u32;
            let mut cur_parent = layer.parent_id;
            while let Some(pid) = cur_parent {
                if let Some(p_layer) = self.layers.get(pid) {
                    eff_opacity = (eff_opacity * p_layer.opacity as u32 + 127) / 255;
                    cur_parent = p_layer.parent_id;
                } else {
                    break;
                }
            }

            // このレイヤーがクリッピングのベースで、すぐ上にクリッピングレイヤーが存在する場合はアルファを記録
            if !layer.clipping && next_is_clipped {
                let lw = layer.width() as usize;
                let lh = layer.height() as usize;
                let mut base_a = vec![0u8; lw * lh];
                for i in 0..(lw * lh) {
                    let a = layer.pixels.get(i * 4 + 3).copied().unwrap_or(0) as u32;
                    base_a[i] = ((a * eff_opacity + 127) / 255) as u8;
                }
                clip_base_alpha = Some((layer.top, layer.left, layer.bottom, layer.right, base_a));
            }

            if eff_opacity == 0 || layer.pixels.is_empty() {
                continue;
            }

            blend_layer_onto_canvas(
                &mut canvas,
                cl,
                ct,
                cr,
                cb,
                cw,
                layer,
                eff_opacity as u8,
                if layer.clipping && any_unclipped_vis {
                    clip_base_alpha.as_ref()
                } else {
                    None
                },
            );
        }

        let frame = Arc::new(RgbaFrame {
            width: cw as u32,
            height: ch as u32,
            original_width: cw as u32,
            original_height: ch as u32,
            pixels: canvas,
        });

        if let Ok(mut cache) = self.composited_cache.lock() {
            if cache.len() >= 64 {
                cache.clear();
            }
            cache.insert(state_hash, Arc::clone(&frame));
        }

        (state_hash, frame)
    }

    fn effective_visibility_for(&self, vis: &[bool]) -> Vec<bool> {
        let mut effective_vis = vec![false; self.layers.len()];
        for l in &self.layers {
            if l.kind != PsdLayerKind::Pixel {
                continue;
            }
            let mut is_vis = vis.get(l.id).copied().unwrap_or(l.default_visible);
            let mut cur_parent = l.parent_id;
            while is_vis && cur_parent.is_some() {
                let pid = cur_parent.unwrap();
                if let Some(p_layer) = self.layers.get(pid) {
                    let p_vis = vis.get(pid).copied().unwrap_or(p_layer.default_visible);
                    if !p_vis {
                        is_vis = false;
                        break;
                    }
                    cur_parent = p_layer.parent_id;
                } else {
                    break;
                }
            }
            effective_vis[l.id] = is_vis;
        }
        effective_vis
    }

    fn clamp_bbox(&self, left: i32, top: i32, right: i32, bottom: i32, tight_crop: bool) -> ([u32; 4], (f64, f64)) {
        if !tight_crop {
            return ([0, 0, self.width, self.height], (0.0, 0.0));
        }
        let cl = left.clamp(0, self.width as i32) as u32;
        let ct = top.clamp(0, self.height as i32) as u32;
        let cr = right.clamp(0, self.width as i32) as u32;
        let cb = bottom.clamp(0, self.height as i32) as u32;
        if cr <= cl || cb <= ct {
            return ([0, 0, self.width, self.height], (0.0, 0.0));
        }
        let ox = (cl + cr) as f64 * 0.5 - self.width as f64 * 0.5;
        let oy = (ct + cb) as f64 * 0.5 - self.height as f64 * 0.5;
        ([cl, ct, cr, cb], (ox, oy))
    }

    /// 現在表示中の各ピクセルレイヤーを 1 枚ずつ個別のタイムラインオブジェクト用パーツへ分解します（背面→前面順）。
    pub fn decompose_by_layers(&self, current_overrides: &str, tight_crop: bool) -> Vec<PsdDecomposedPart> {
        let vis = self.compute_visibility_from_overrides(current_overrides);
        let eff_vis = self.effective_visibility_for(&vis);
        let (blink_open, blink_close) = self.resolve_blink_paths(&PsdBlinkConfig::default());
        let (lip_close, lip_open) = self.resolve_lipsync_paths(&PsdLipSyncConfig::default());

        let mut parts = Vec::new();
        for layer in &self.layers {
            if layer.kind != PsdLayerKind::Pixel || !eff_vis[layer.id] || layer.pixels.is_empty() {
                continue;
            }
            if layer.width() == 0 || layer.height() == 0 {
                continue;
            }

            let (crop_rect, center_offset) =
                self.clamp_bbox(layer.left, layer.top, layer.right, layer.bottom, tight_crop);

            let mut tokens = vec!["*=0".to_string()];
            if tight_crop {
                tokens.push(format!(
                    "@crop={},{},{},{}",
                    crop_rect[0], crop_rect[1], crop_rect[2], crop_rect[3]
                ));
            }
            // 親フォルダから順に =1 を付与
            let mut ancestors = Vec::new();
            let mut cur_p = layer.parent_id;
            while let Some(pid) = cur_p {
                if let Some(pl) = self.layers.get(pid) {
                    if !pl.path.is_empty() {
                        ancestors.push(pl.path.clone());
                    }
                    cur_p = pl.parent_id;
                } else {
                    break;
                }
            }
            for a_path in ancestors.into_iter().rev() {
                tokens.push(format!("{a_path}=1"));
            }
            tokens.push(format!("{}=1", layer.path));

            let inherit_blink = !blink_open.is_empty()
                && (layer.path == blink_open || layer.path == blink_close);
            let inherit_lipsync =
                !lip_open.is_empty() && (layer.path == lip_close || layer.path == lip_open);

            parts.push(PsdDecomposedPart {
                label: layer.path.clone(),
                psd_layers_param: tokens.join(";"),
                crop_rect,
                center_offset,
                blend_mode: layer.blend_mode,
                clipping: layer.clipping,
                inherit_blink,
                inherit_lipsync,
            });
        }
        parts
    }

    /// フォルダ（パーツグループ：眉・目・口・髪・体など）単位でタイムラインオブジェクト用パーツへ分解します（背面→前面順）。
    /// 各フォルダ内の表情差分（`!閉じ` や `!あ` など）も同じパーツ内に保持されるため、分解後も目パチ・口パク・表情切替がそのまま機能します。
    pub fn decompose_by_folders(&self, current_overrides: &str, tight_crop: bool) -> Vec<PsdDecomposedPart> {
        let vis = self.compute_visibility_from_overrides(current_overrides);
        let eff_vis = self.effective_visibility_for(&vis);
        let (blink_open, blink_close) = self.resolve_blink_paths(&PsdBlinkConfig::default());
        let (lip_close, lip_open) = self.resolve_lipsync_paths(&PsdLipSyncConfig::default());

        // サブフォルダを直接持つ「コンテナフォルダ」を判定（例: ルートの「立ち絵」や「顔」フォルダ）
        let mut folder_has_subfolders = vec![false; self.layers.len()];
        for l in &self.layers {
            if matches!(l.kind, PsdLayerKind::FolderStart { .. }) {
                if let Some(pid) = l.parent_id {
                    if pid < folder_has_subfolders.len() {
                        folder_has_subfolders[pid] = true;
                    }
                }
            }
        }

        // ボトムアップ順に走査し、各ピクセルレイヤーを「末端パーツフォルダ」または「単独ベースレイヤー(+クリッピング)」にグループ化
        #[derive(Clone)]
        enum GroupKey {
            Folder(usize),
            SingleBase(usize),
        }

        let mut groups: Vec<(GroupKey, Vec<usize>)> = Vec::new();
        let mut folder_to_group_idx: HashMap<usize, usize> = HashMap::new();
        let mut last_single_group_idx: Option<usize> = None;

        for layer in &self.layers {
            if layer.kind != PsdLayerKind::Pixel || layer.pixels.is_empty() {
                continue;
            }
            if layer.width() == 0 || layer.height() == 0 {
                continue;
            }

            // 親フォルダがあり、その親フォルダがコンテナ（サブフォルダ持ち）ではない場合は親フォルダ単位でまとめる
            if let Some(pid) = layer.parent_id {
                let parent_vis = vis.get(pid).copied().unwrap_or(true);
                if parent_vis && !folder_has_subfolders[pid] {
                    let g_idx = *folder_to_group_idx.entry(pid).or_insert_with(|| {
                        let idx = groups.len();
                        groups.push((GroupKey::Folder(pid), Vec::new()));
                        idx
                    });
                    groups[g_idx].1.push(layer.id);
                    last_single_group_idx = None;
                    continue;
                }
            }

            // ルート直下またはコンテナフォルダ直下のピクセルレイヤー
            if layer.clipping {
                if let Some(g_idx) = last_single_group_idx {
                    if eff_vis[layer.id] {
                        groups[g_idx].1.push(layer.id);
                    }
                    continue;
                }
            }
            if eff_vis[layer.id] {
                let idx = groups.len();
                groups.push((GroupKey::SingleBase(layer.id), vec![layer.id]));
                last_single_group_idx = Some(idx);
            } else {
                last_single_group_idx = None;
            }
        }

        let mut parts = Vec::new();
        for (key, member_ids) in groups {
            // グループ内に現在表示されているピクセルレイヤーが1つも無ければスキップ
            let has_any_visible = member_ids.iter().any(|&id| eff_vis[id]);
            if !has_any_visible {
                continue;
            }

            // バウンディングボックスはグループ内の全候補レイヤー（目パチ・口パクの非表示差分も含む）の統合矩形を計算
            let mut min_l = i32::MAX;
            let mut min_t = i32::MAX;
            let mut max_r = i32::MIN;
            let mut max_b = i32::MIN;
            for &id in &member_ids {
                let l = &self.layers[id];
                min_l = min_l.min(l.left);
                min_t = min_t.min(l.top);
                max_r = max_r.max(l.right);
                max_b = max_b.max(l.bottom);
            }
            if min_l >= max_r || min_t >= max_b {
                continue;
            }

            let (crop_rect, center_offset) = self.clamp_bbox(min_l, min_t, max_r, max_b, tight_crop);

            let mut tokens = vec!["*=0".to_string()];
            if tight_crop {
                tokens.push(format!(
                    "@crop={},{},{},{}",
                    crop_rect[0], crop_rect[1], crop_rect[2], crop_rect[3]
                ));
            }

            let (label, blend_mode) = match key {
                GroupKey::Folder(fid) => {
                    let fl = &self.layers[fid];
                    let mut ancestors = Vec::new();
                    let mut cur_p = Some(fid);
                    while let Some(pid) = cur_p {
                        if let Some(pl) = self.layers.get(pid) {
                            if !pl.path.is_empty() {
                                ancestors.push(pl.path.clone());
                            }
                            cur_p = pl.parent_id;
                        } else {
                            break;
                        }
                    }
                    for a_path in ancestors.into_iter().rev() {
                        tokens.push(format!("{a_path}=1"));
                    }
                    for &mid in &member_ids {
                        if eff_vis[mid] {
                            tokens.push(format!("{}=1", self.layers[mid].path));
                        }
                    }
                    let bm = if fl.blend_mode == PsdBlendMode::PassThrough {
                        PsdBlendMode::Normal
                    } else {
                        fl.blend_mode
                    };
                    (fl.path.clone(), bm)
                }
                GroupKey::SingleBase(lid) => {
                    let bl = &self.layers[lid];
                    let mut ancestors = Vec::new();
                    let mut cur_p = bl.parent_id;
                    while let Some(pid) = cur_p {
                        if let Some(pl) = self.layers.get(pid) {
                            if !pl.path.is_empty() {
                                ancestors.push(pl.path.clone());
                            }
                            cur_p = pl.parent_id;
                        } else {
                            break;
                        }
                    }
                    for a_path in ancestors.into_iter().rev() {
                        tokens.push(format!("{a_path}=1"));
                    }
                    for &mid in &member_ids {
                        if eff_vis[mid] {
                            tokens.push(format!("{}=1", self.layers[mid].path));
                        }
                    }
                    (bl.path.clone(), bl.blend_mode)
                }
            };

            let inherit_blink = !blink_open.is_empty()
                && member_ids
                    .iter()
                    .any(|&id| self.layers[id].path == blink_open || self.layers[id].path == blink_close);
            let inherit_lipsync = !lip_open.is_empty()
                && member_ids
                    .iter()
                    .any(|&id| self.layers[id].path == lip_close || self.layers[id].path == lip_open);

            parts.push(PsdDecomposedPart {
                label,
                psd_layers_param: tokens.join(";"),
                crop_rect,
                center_offset,
                blend_mode,
                clipping: false,
                inherit_blink,
                inherit_lipsync,
            });
        }

        parts
    }

    /// 各分解パーツ（レイヤー単位またはフォルダ単位）を透過 PNG ファイルとして指定フォルダに一括書き出しします。
    pub fn export_parts_as_png(
        &self,
        current_overrides: &str,
        out_dir: &Path,
        by_folders: bool,
        tight_crop: bool,
    ) -> Result<usize, String> {
        std::fs::create_dir_all(out_dir)
            .map_err(|e| format!("出力先フォルダを作成できません ({}): {e}", out_dir.display()))?;
        let parts = if by_folders {
            self.decompose_by_folders(current_overrides, tight_crop)
        } else {
            self.decompose_by_layers(current_overrides, tight_crop)
        };
        let mut count = 0;
        for (idx, part) in parts.iter().enumerate() {
            let vis = self.compute_visibility_from_overrides(&part.psd_layers_param);
            let crop = self.parse_crop_rect(&part.psd_layers_param);
            let (_, rgba) = self.composite_with_visibility_and_crop(&vis, crop);
            let safe_name: String = part
                .label
                .chars()
                .map(|c| match c {
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                    _ => c,
                })
                .collect();
            let file_name = format!("{:02}_{}.png", idx + 1, safe_name);
            let out_path = out_dir.join(file_name);
            image::save_buffer(
                &out_path,
                &rgba.pixels,
                rgba.width,
                rgba.height,
                image::ColorType::Rgba8,
            )
            .map_err(|e| format!("PNG の保存に失敗しました ({}): {e}", out_path.display()))?;
            count += 1;
        }
        Ok(count)
    }
}

/// 1枚のレイヤーをキャンバス（切り抜き矩形 `cl..cr, ct..cb`）にブレンド合成します。
#[allow(clippy::too_many_arguments)]
fn blend_layer_onto_canvas(
    canvas: &mut [u8],
    cl: i32,
    ct: i32,
    cr: i32,
    cb: i32,
    cw: usize,
    layer: &PsdLayer,
    eff_opacity: u8,
    clip_base: Option<&(i32, i32, i32, i32, Vec<u8>)>,
) {
    let lw = layer.width() as i32;
    let lh = layer.height() as i32;
    if lw <= 0 || lh <= 0 {
        return;
    }

    let y_start = layer.top.max(ct);
    let y_end = layer.bottom.min(cb);
    let x_start = layer.left.max(cl);
    let x_end = layer.right.min(cr);
    if y_start >= y_end || x_start >= x_end {
        return;
    }

    let lw_usize = lw as usize;
    let mode = layer.blend_mode;

    for cy in y_start..y_end {
        let ly = (cy - layer.top) as usize;
        let row_src_base = ly * lw_usize * 4;
        let row_dst_base = ((cy - ct) as usize) * cw * 4;

        for cx in x_start..x_end {
            let lx = (cx - layer.left) as usize;
            let src_idx = row_src_base + lx * 4;
            let raw_sa = layer.pixels[src_idx + 3];
            if raw_sa == 0 {
                continue;
            }

            let mut sa = ((raw_sa as u32 * eff_opacity as u32 + 127) / 255) as u8;

            // クリッピングマスク適用時はベースレイヤーのアルファでマスク
            if let Some(&(bt, bl, bb, br, ref base_a)) = clip_base {
                if cx >= bl && cx < br && cy >= bt && cy < bb {
                    let bx = (cx - bl) as usize;
                    let by = (cy - bt) as usize;
                    let bw = (br - bl) as usize;
                    let ba = base_a[by * bw + bx];
                    sa = ((sa as u32 * ba as u32 + 127) / 255) as u8;
                } else {
                    continue;
                }
            }
            if sa == 0 {
                continue;
            }

            let sr = layer.pixels[src_idx];
            let sg = layer.pixels[src_idx + 1];
            let sb = layer.pixels[src_idx + 2];

            let dst_idx = row_dst_base + ((cx - cl) as usize) * 4;
            let da = canvas[dst_idx + 3];

            if da == 0 {
                canvas[dst_idx] = sr;
                canvas[dst_idx + 1] = sg;
                canvas[dst_idx + 2] = sb;
                canvas[dst_idx + 3] = sa;
                continue;
            }

            let dr = canvas[dst_idx];
            let dg = canvas[dst_idx + 1];
            let db = canvas[dst_idx + 2];

            let (br, bg, bb_col) = match mode {
                PsdBlendMode::Normal | PsdBlendMode::PassThrough => (sr, sg, sb),
                _ => (
                    blend_channel(dr, sr, mode),
                    blend_channel(dg, sg, mode),
                    blend_channel(db, sb, mode),
                ),
            };

            // Porter-Duff Source-Over with backdrop blend mode:
            // out_a = sa + da * (1 - sa)
            // C_out * out_a = sa * (1 - da) * Cs + sa * da * B(Cd, Cs) + (1 - sa) * da * Cd
            let sa_f = sa as u32;
            let da_f = da as u32;
            let inv_sa = 255 - sa_f;
            let inv_da = 255 - da_f;

            let out_a_255 = sa_f * 255 + da_f * inv_sa;
            let out_a = ((out_a_255 + 127) / 255) as u8;
            if out_a == 0 {
                continue;
            }

            let w_src = sa_f * inv_da;
            let w_bld = sa_f * da_f;
            let w_dst = inv_sa * da_f;

            let r_num = w_src * (sr as u32) + w_bld * (br as u32) + w_dst * (dr as u32);
            let g_num = w_src * (sg as u32) + w_bld * (bg as u32) + w_dst * (dg as u32);
            let b_num = w_src * (sb as u32) + w_bld * (bb_col as u32) + w_dst * (db as u32);

            canvas[dst_idx] = ((r_num + out_a_255 / 2) / out_a_255).min(255) as u8;
            canvas[dst_idx + 1] = ((g_num + out_a_255 / 2) / out_a_255).min(255) as u8;
            canvas[dst_idx + 2] = ((b_num + out_a_255 / 2) / out_a_255).min(255) as u8;
            canvas[dst_idx + 3] = out_a;
        }
    }
}

#[inline]
fn blend_channel(d: u8, s: u8, mode: PsdBlendMode) -> u8 {
    let d32 = d as i32;
    let s32 = s as i32;
    match mode {
        PsdBlendMode::Normal | PsdBlendMode::PassThrough => s,
        PsdBlendMode::Multiply => ((d32 * s32 + 127) / 255) as u8,
        PsdBlendMode::Screen => (255 - ((255 - d32) * (255 - s32) + 127) / 255) as u8,
        PsdBlendMode::Overlay => {
            if d32 < 128 {
                ((2 * d32 * s32 + 127) / 255).clamp(0, 255) as u8
            } else {
                (255 - (2 * (255 - d32) * (255 - s32) + 127) / 255).clamp(0, 255) as u8
            }
        }
        PsdBlendMode::HardLight => {
            if s32 < 128 {
                ((2 * d32 * s32 + 127) / 255).clamp(0, 255) as u8
            } else {
                (255 - (2 * (255 - d32) * (255 - s32) + 127) / 255).clamp(0, 255) as u8
            }
        }
        PsdBlendMode::SoftLight => {
            let df = d32 as f32 / 255.0;
            let sf = s32 as f32 / 255.0;
            let res = if sf <= 0.5 {
                df - (1.0 - 2.0 * sf) * df * (1.0 - df)
            } else {
                let g = if df <= 0.25 {
                    ((16.0 * df - 12.0) * df + 4.0) * df
                } else {
                    df.sqrt()
                };
                df + (2.0 * sf - 1.0) * (g - df)
            };
            (res * 255.0).round().clamp(0.0, 255.0) as u8
        }
        PsdBlendMode::Darken => d.min(s),
        PsdBlendMode::Lighten => d.max(s),
        PsdBlendMode::LinearDodge => (d32 + s32).min(255) as u8,
        PsdBlendMode::ColorDodge => {
            if s32 >= 255 {
                255
            } else {
                ((d32 * 255) / (255 - s32)).min(255) as u8
            }
        }
        PsdBlendMode::LinearBurn => (d32 + s32 - 255).max(0) as u8,
        PsdBlendMode::ColorBurn => {
            if s32 <= 0 {
                0
            } else {
                (255 - ((255 - d32) * 255) / s32).max(0) as u8
            }
        }
        PsdBlendMode::Difference => (d32 - s32).unsigned_abs() as u8,
        PsdBlendMode::Exclusion => (d32 + s32 - (2 * d32 * s32 + 127) / 255).clamp(0, 255) as u8,
        PsdBlendMode::Subtract => (d32 - s32).max(0) as u8,
    }
}

/// チャンネルデータを展開します（0 = Raw, 1 = PackBits RLE）。
fn decode_channel_slice(slice: &[u8], width: usize, height: usize, depth: u16, is_psb: bool) -> Option<Vec<u8>> {
    if slice.len() < 2 {
        return None;
    }
    let compression = u16::from_be_bytes([slice[0], slice[1]]);
    let payload = &slice[2..];
    let bytes_per_sample = (depth as usize) / 8;
    let row_bytes = width * bytes_per_sample;
    let total_bytes = row_bytes * height;

    let raw_bytes = match compression {
        0 => {
            // Raw Uncompressed
            let mut out = vec![0u8; total_bytes];
            let copy_len = payload.len().min(total_bytes);
            out[..copy_len].copy_from_slice(&payload[..copy_len]);
            out
        }
        1 => {
            // PackBits RLE: 先頭に height 行分のバイト数テーブル（PSD: u16 BE, PSB: u32 BE）
            let table_entry_size = if is_psb { 4 } else { 2 };
            let table_bytes = height * table_entry_size;
            if payload.len() < table_bytes {
                return None;
            }
            let rle_data = &payload[table_bytes..];
            unpack_packbits(rle_data, total_bytes)
        }
        _ => return None,
    };

    if depth == 16 {
        // 16-bit BE -> 8-bit 変換（上位バイトを取得）
        let mut out8 = Vec::with_capacity(width * height);
        for chunk in raw_bytes.chunks_exact(2) {
            out8.push(chunk[0]);
        }
        Some(out8)
    } else {
        Some(raw_bytes)
    }
}

/// PSD 末尾の Merged Image Data セクションをデコードします。
fn decode_merged_image(
    slice: &[u8],
    width: usize,
    height: usize,
    num_channels: usize,
    depth: u16,
    color_mode: u16,
    is_psb: bool,
) -> Option<Arc<RgbaFrame>> {
    if slice.len() < 2 || num_channels == 0 {
        return None;
    }
    let compression = u16::from_be_bytes([slice[0], slice[1]]);
    let payload = &slice[2..];
    let bytes_per_sample = (depth as usize) / 8;
    let plane_samples = width * height;
    let plane_bytes = plane_samples * bytes_per_sample;
    let ch_to_read = num_channels.min(4);
    let total_bytes = plane_bytes * ch_to_read;

    let raw_bytes = match compression {
        0 => {
            if payload.len() < plane_bytes {
                return None;
            }
            let mut out = vec![0u8; total_bytes];
            let copy_len = payload.len().min(total_bytes);
            out[..copy_len].copy_from_slice(&payload[..copy_len]);
            out
        }
        1 => {
            let table_entry_size = if is_psb { 4 } else { 2 };
            let total_rows = height * num_channels;
            let table_bytes = total_rows * table_entry_size;
            if payload.len() < table_bytes {
                return None;
            }
            let rle_data = &payload[table_bytes..];
            unpack_packbits(rle_data, total_bytes)
        }
        _ => return None,
    };

    let sample_at = |ch: usize, idx: usize| -> u8 {
        let byte_offset = (ch * plane_samples + idx) * bytes_per_sample;
        raw_bytes.get(byte_offset).copied().unwrap_or(if ch == 3 { 255 } else { 0 })
    };

    let mut rgba = vec![0u8; plane_samples * 4];
    if color_mode == 1 {
        for i in 0..plane_samples {
            let v = sample_at(0, i);
            let a = if ch_to_read >= 2 { sample_at(1, i) } else { 255 };
            rgba[i * 4] = v;
            rgba[i * 4 + 1] = v;
            rgba[i * 4 + 2] = v;
            rgba[i * 4 + 3] = a;
        }
    } else {
        for i in 0..plane_samples {
            rgba[i * 4] = sample_at(0, i);
            rgba[i * 4 + 1] = if ch_to_read >= 2 { sample_at(1, i) } else { 0 };
            rgba[i * 4 + 2] = if ch_to_read >= 3 { sample_at(2, i) } else { 0 };
            rgba[i * 4 + 3] = if ch_to_read >= 4 { sample_at(3, i) } else { 255 };
        }
    }

    Some(Arc::new(RgbaFrame {
        width: width as u32,
        height: height as u32,
        original_width: width as u32,
        original_height: height as u32,
        pixels: rgba,
    }))
}

/// PackBits RLE 圧縮データを高速展開します。
pub fn unpack_packbits(src: &[u8], expected_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(expected_len);
    let mut i = 0;
    while i < src.len() && out.len() < expected_len {
        let header = src[i] as i8;
        i += 1;
        if header >= 0 {
            let count = (header as usize) + 1;
            let avail = (src.len() - i).min(count).min(expected_len - out.len());
            out.extend_from_slice(&src[i..i + avail]);
            i += count.min(src.len() - i);
        } else if header != -128 {
            let count = (1 - (header as isize)) as usize;
            if i < src.len() {
                let val = src[i];
                i += 1;
                let repeat = count.min(expected_len - out.len());
                out.resize(out.len() + repeat, val);
            }
        }
    }
    if out.len() < expected_len {
        out.resize(expected_len, 0);
    }
    out
}

/// PSD 内の文字列（UTF-8 または Shift_JIS）をデコードします。
fn decode_psd_string(bytes: &[u8]) -> String {
    let trimmed = bytes.split(|&b| b == 0).next().unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(trimmed) {
        return s.to_string();
    }
    encoding_rs::SHIFT_JIS.decode(trimmed).0.into_owned()
}

/// PSDToolKit のプレフィックス規則（`*` 必須、`!` 排他ラジオ）を解析します。
fn parse_psdtoolkit_prefix(raw_name: &str) -> (bool, bool, String) {
    let trimmed = raw_name.trim();
    if let Some(rest) = trimmed.strip_prefix('*') {
        (true, false, rest.trim().to_string())
    } else if let Some(rest) = trimmed.strip_prefix('!') {
        (false, true, rest.trim().to_string())
    } else {
        (false, false, trimmed.to_string())
    }
}

fn build_layer_path(layers: &[PsdLayer], folder_stack: &[usize], current_name: &str) -> String {
    let mut parts = Vec::with_capacity(folder_stack.len() + 1);
    for &fid in folder_stack {
        parts.push(layers[fid].name.as_str());
    }
    parts.push(current_name);
    parts.join("/")
}

fn strip_prefixes_in_path(path: &str) -> String {
    path.split('/')
        .map(|seg| seg.trim().trim_start_matches(['!', '*']).trim())
        .collect::<Vec<_>>()
        .join("/")
}

/// レイヤー一覧から「目パチ（開き目・閉じ目）」候補を自動検出します。
fn detect_blink_layers(layers: &[PsdLayer]) -> Option<(String, String)> {
    // 「目」「眼」「Eye」フォルダを探す
    for folder in layers.iter().rev() {
        if !matches!(folder.kind, PsdLayerKind::FolderStart { .. }) {
            continue;
        }
        let clean = folder.display_name.to_ascii_lowercase();
        let is_eye_folder = clean == "目"
            || clean == "眼"
            || clean.contains("目")
            || clean.contains("eye")
            || clean.contains("まばたき");
        if !is_eye_folder {
            continue;
        }

        let children: Vec<&PsdLayer> = layers
            .iter()
            .filter(|l| l.parent_id == Some(folder.id) && l.kind == PsdLayerKind::Pixel && !l.clipping)
            .collect();
        if children.len() < 2 {
            continue;
        }

        // 開き目: 現在初期表示されているレイヤー、または「通常」「普通」「開き」「open」
        let open_layer = children
            .iter()
            .copied()
            .find(|l| l.default_visible)
            .or_else(|| {
                children.iter().copied().find(|l| {
                    let n = l.display_name.to_ascii_lowercase();
                    n.contains("通常") || n.contains("普通") || n.contains("開") || n.contains("open") || n.contains("normal")
                })
            })
            .or_else(|| children.last().copied());

        // 閉じ目: 「閉じ」「閉」「まばたき」「笑顔」「なごみ」「close」「blink」
        let close_layer = children
            .iter()
            .copied()
            .find(|l| {
                let n = l.display_name.to_ascii_lowercase();
                n.contains("閉") || n.contains("まばたき") || n.contains("close") || n.contains("blink") || n == "ー" || n == "-"
            })
            .or_else(|| {
                children.iter().copied().find(|l| {
                    let n = l.display_name.to_ascii_lowercase();
                    n.contains("笑") || n.contains("にっこり") || n.contains("smile")
                })
            })
            .or_else(|| {
                children
                    .iter()
                    .copied()
                    .find(|l| Some(l.id) != open_layer.map(|o| o.id))
            });

        if let (Some(o), Some(c)) = (open_layer, close_layer) {
            if o.id != c.id {
                return Some((o.path.clone(), c.path.clone()));
            }
        }
    }
    None
}

/// レイヤー一覧から「口パク（閉じ口・開き口）」候補を自動検出します。
fn detect_lipsync_layers(layers: &[PsdLayer]) -> Option<(String, String)> {
    // 「口」「リップ」「Mouth」フォルダを探す
    for folder in layers.iter().rev() {
        if !matches!(folder.kind, PsdLayerKind::FolderStart { .. }) {
            continue;
        }
        let clean = folder.display_name.to_ascii_lowercase();
        let is_mouth_folder =
            clean == "口" || clean.contains("口") || clean.contains("リップ") || clean.contains("mouth") || clean.contains("lip");
        if !is_mouth_folder {
            continue;
        }

        let children: Vec<&PsdLayer> = layers
            .iter()
            .filter(|l| l.parent_id == Some(folder.id) && l.kind == PsdLayerKind::Pixel && !l.clipping)
            .collect();
        if children.len() < 2 {
            continue;
        }

        // 閉じ口: 「閉じ」「閉」「ん」「む」「通常」「普通」「close」または初期表示レイヤー
        let close_layer = children
            .iter()
            .copied()
            .find(|l| {
                let n = l.display_name.to_ascii_lowercase();
                n.contains("閉")
                    || n == "ん"
                    || n == "む"
                    || n.contains("むっ")
                    || n.contains("close")
                    || n == "ー"
                    || n == "-"
            })
            .or_else(|| children.iter().copied().find(|l| l.default_visible))
            .or_else(|| children.last().copied());

        // 開き口: 「あ」「開」「わ」「お」「笑顔」「open」「talk」
        let open_layer = children
            .iter()
            .copied()
            .find(|l| {
                let n = l.display_name.to_ascii_lowercase();
                n == "あ"
                    || n.contains("あ")
                    || n.contains("開")
                    || n.contains("open")
                    || n == "わ"
                    || n == "お"
                    || n.contains("笑")
            })
            .or_else(|| {
                children
                    .iter()
                    .copied()
                    .find(|l| Some(l.id) != close_layer.map(|c| c.id))
            });

        if let (Some(c), Some(o)) = (close_layer, open_layer) {
            if c.id != o.id {
                return Some((c.path.clone(), o.path.clone()));
            }
        }
    }
    None
}

/// `.pfv` (PSDToolKit Favorites) バイト列を解析してお気に入りプリセット一覧を返します。
pub fn parse_pfv_bytes(bytes: &[u8]) -> Vec<PsdFavoritePreset> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        s.to_owned()
    } else {
        encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned()
    };
    parse_pfv_str(&text)
}

/// `.pfv` (PSDToolKit Favorites) テキストを解析します。
pub fn parse_pfv_str(text: &str) -> Vec<PsdFavoritePreset> {
    let mut presets: Vec<PsdFavoritePreset> = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut cur_paths: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.eq_ignore_ascii_case("[PSDToolKit Favorites]") {
            continue;
        }
        if let Some(header) = line.strip_prefix("//") {
            if let Some(name) = cur_name.take() {
                if !cur_paths.is_empty() {
                    presets.push(PsdFavoritePreset {
                        name,
                        active_paths: std::mem::take(&mut cur_paths),
                    });
                }
            }
            let clean_name = header.trim().trim_start_matches('/').trim();
            if !clean_name.is_empty() {
                cur_name = Some(clean_name.to_string());
            }
        } else if cur_name.is_some() && !line.starts_with(';') && !line.starts_with('#') {
            // PSDToolKit のパス表記（`v1.` や `%2F` 等を正規化）
            let p = line
                .trim_start_matches("v1.")
                .replace("\\", "/")
                .replace("%2F", "/")
                .replace("%2f", "/");
            cur_paths.push(p);
        }
    }
    if let Some(name) = cur_name.take() {
        if !cur_paths.is_empty() {
            presets.push(PsdFavoritePreset {
                name,
                active_paths: cur_paths,
            });
        }
    }
    presets
}

/// グループ化されたパーツ一覧（フォルダID -> フォルダ内の子ピクセルレイヤー一覧）を返します。
pub fn collect_part_groups(doc: &PsdDocument) -> Vec<(usize, Vec<usize>)> {
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut order = Vec::new();
    for l in doc.layers.iter().rev() {
        if matches!(l.kind, PsdLayerKind::FolderStart { .. }) {
            order.push(l.id);
            groups.insert(l.id, Vec::new());
        }
    }
    for l in doc.layers.iter().rev() {
        if l.kind == PsdLayerKind::Pixel && !l.clipping {
            if let Some(pid) = l.parent_id {
                if let Some(list) = groups.get_mut(&pid) {
                    list.push(l.id);
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|fid| {
            let children = groups.remove(&fid)?;
            (children.len() >= 2).then_some((fid, children))
        })
        .collect()
}

fn read_u16_be(data: &[u8], offset: usize) -> Result<u16, String> {
    let s = data.get(offset..offset + 2).ok_or("PSD データの末尾を超えました (u16)")?;
    Ok(u16::from_be_bytes([s[0], s[1]]))
}

fn read_i16_be(data: &[u8], offset: usize) -> Result<i16, String> {
    let s = data.get(offset..offset + 2).ok_or("PSD データの末尾を超えました (i16)")?;
    Ok(i16::from_be_bytes([s[0], s[1]]))
}

fn read_u32_be(data: &[u8], offset: usize) -> Result<u32, String> {
    let s = data.get(offset..offset + 4).ok_or("PSD データの末尾を超えました (u32)")?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_i32_be(data: &[u8], offset: usize) -> Result<i32, String> {
    let s = data.get(offset..offset + 4).ok_or("PSD データの末尾を超えました (i32)")?;
    Ok(i32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_u64_be(data: &[u8], offset: usize) -> Result<u64, String> {
    let s = data.get(offset..offset + 8).ok_or("PSD データの末尾を超えました (u64)")?;
    Ok(u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

/// テストやサンプル生成用に、フォルダ階層・目パチ・口パクレイヤーを備えた有効な PSD バイナリをメモリ上で構築します。
pub fn build_synthetic_character_psd(width: u32, height: u32) -> Vec<u8> {
    struct LayerSpec<'a> {
        name: &'a str,
        visible: bool,
        div_type: Option<u32>,
        rect: (i32, i32, i32, i32),
        color: [u8; 4],
    }

    let w = width as i32;
    let h = height as i32;

    // ボトムアップ順（0 = 最背面 -> 末尾 = 最前面）
    let specs = vec![
        // 1. 素体 (*体)
        LayerSpec {
            name: "*体",
            visible: true,
            div_type: None,
            rect: (0, 0, h, w),
            color: [60, 110, 210, 255],
        },
        // 2. 口フォルダ終了マーカー
        LayerSpec {
            name: "</Layer group>",
            visible: true,
            div_type: Some(3),
            rect: (0, 0, 0, 0),
            color: [0, 0, 0, 0],
        },
        // 口/!あ (初期非表示)
        LayerSpec {
            name: "!あ",
            visible: false,
            div_type: None,
            rect: (h / 2, w / 4, h * 3 / 4, w * 3 / 4),
            color: [240, 70, 90, 255],
        },
        // 口/!閉じ (初期表示)
        LayerSpec {
            name: "!閉じ",
            visible: true,
            div_type: None,
            rect: (h / 2, w / 4, h * 3 / 4, w * 3 / 4),
            color: [220, 140, 150, 255],
        },
        // 口フォルダ開始ヘッダ
        LayerSpec {
            name: "口",
            visible: true,
            div_type: Some(1),
            rect: (0, 0, 0, 0),
            color: [0, 0, 0, 0],
        },
        // 3. 目フォルダ終了マーカー
        LayerSpec {
            name: "</Layer group>",
            visible: true,
            div_type: Some(3),
            rect: (0, 0, 0, 0),
            color: [0, 0, 0, 0],
        },
        // 目/!閉じ (初期非表示)
        LayerSpec {
            name: "!閉じ",
            visible: false,
            div_type: None,
            rect: (h / 4, w / 4, h / 2, w * 3 / 4),
            color: [40, 40, 60, 255],
        },
        // 目/!通常 (初期表示)
        LayerSpec {
            name: "!通常",
            visible: true,
            div_type: None,
            rect: (h / 4, w / 4, h / 2, w * 3 / 4),
            color: [255, 215, 60, 255],
        },
        // 目フォルダ開始ヘッダ
        LayerSpec {
            name: "目",
            visible: true,
            div_type: Some(1),
            rect: (0, 0, 0, 0),
            color: [0, 0, 0, 0],
        },
    ];

    let mut out = Vec::new();
    // PSD Header (26 bytes)
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes()); // version 1
    out.extend_from_slice(&[0u8; 6]); // reserved
    out.extend_from_slice(&4u16.to_be_bytes()); // 4 channels (RGBA)
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&8u16.to_be_bytes()); // 8-bit depth
    out.extend_from_slice(&3u16.to_be_bytes()); // RGB color mode

    // Color Mode Data (0) & Image Resources (0)
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());

    // Build Layer Info
    let mut layer_records = Vec::new();
    let mut channel_data = Vec::new();

    layer_records.extend_from_slice(&(specs.len() as i16).to_be_bytes());

    for sp in &specs {
        let (t, l, b, r) = sp.rect;
        let lw = (r - l).max(0) as usize;
        let lh = (b - t).max(0) as usize;
        let ch_data_len = (2 + lw * lh) as u32; // compression (2 bytes) + raw pixels

        layer_records.extend_from_slice(&t.to_be_bytes());
        layer_records.extend_from_slice(&l.to_be_bytes());
        layer_records.extend_from_slice(&b.to_be_bytes());
        layer_records.extend_from_slice(&r.to_be_bytes());
        layer_records.extend_from_slice(&4u16.to_be_bytes()); // 4 channels: R(0), G(1), B(2), A(-1)
        for ch_id in [0i16, 1, 2, -1] {
            layer_records.extend_from_slice(&ch_id.to_be_bytes());
            layer_records.extend_from_slice(&ch_data_len.to_be_bytes());
        }
        layer_records.extend_from_slice(b"8BIM");
        layer_records.extend_from_slice(b"norm");
        layer_records.push(255); // opacity
        layer_records.push(0); // clipping
        layer_records.push(if sp.visible { 0 } else { 0x02 }); // flags
        layer_records.push(0); // filler

        // Extra Data
        let mut extra = Vec::new();
        extra.extend_from_slice(&0u32.to_be_bytes()); // mask len = 0
        extra.extend_from_slice(&0u32.to_be_bytes()); // blending ranges len = 0

        // Pascal string (ASCII fallback) + pad to 4 bytes
        let ascii_bytes = b"layer";
        extra.push(ascii_bytes.len() as u8);
        extra.extend_from_slice(ascii_bytes);
        while (extra.len() - 8) % 4 != 0 {
            extra.push(0);
        }

        // Additional Layer Info: "luni" (UTF-16BE name)
        let utf16: Vec<u16> = sp.name.encode_utf16().collect();
        let mut luni_block = Vec::new();
        luni_block.extend_from_slice(&(utf16.len() as u32).to_be_bytes());
        for code in utf16 {
            luni_block.extend_from_slice(&code.to_be_bytes());
        }
        if luni_block.len() % 2 != 0 {
            luni_block.push(0);
        }
        extra.extend_from_slice(b"8BIMluni");
        extra.extend_from_slice(&(luni_block.len() as u32).to_be_bytes());
        extra.extend_from_slice(&luni_block);

        // Additional Layer Info: "lsct" if folder start/end
        if let Some(div) = sp.div_type {
            extra.extend_from_slice(b"8BIMlsct");
            extra.extend_from_slice(&4u32.to_be_bytes());
            extra.extend_from_slice(&div.to_be_bytes());
        }

        layer_records.extend_from_slice(&(extra.len() as u32).to_be_bytes());
        layer_records.extend_from_slice(&extra);

        // Channel image data (4 channels, raw = 0)
        for c_idx in 0..4 {
            channel_data.extend_from_slice(&0u16.to_be_bytes()); // Raw compression
            if lw * lh > 0 {
                channel_data.resize(channel_data.len() + lw * lh, sp.color[c_idx]);
            }
        }
    }

    let mut layer_info = Vec::new();
    layer_info.extend_from_slice(&layer_records);
    layer_info.extend_from_slice(&channel_data);
    if layer_info.len() % 2 != 0 {
        layer_info.push(0);
    }

    let mut layer_mask_sec = Vec::new();
    layer_mask_sec.extend_from_slice(&(layer_info.len() as u32).to_be_bytes());
    layer_mask_sec.extend_from_slice(&layer_info);

    out.extend_from_slice(&(layer_mask_sec.len() as u32).to_be_bytes());
    out.extend_from_slice(&layer_mask_sec);

    // Merged image data (Raw = 0)
    out.extend_from_slice(&0u16.to_be_bytes());
    let total_px = (width * height) as usize;
    out.resize(out.len() + total_px * 4, 128);

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unpack_packbits_rle() {
        // 3 literal bytes [10, 20, 30], then 4 repeated bytes of 99 (header = 1 - 4 = -3)
        let rle = [2u8, 10, 20, 30, (-3i8) as u8, 99];
        let out = unpack_packbits(&rle, 7);
        assert_eq!(out, vec![10, 20, 30, 99, 99, 99, 99]);
    }

    #[test]
    fn test_parse_synthetic_character_psd_and_composite() {
        let psd_bytes = build_synthetic_character_psd(32, 32);
        let doc = PsdDocument::from_bytes(&psd_bytes).expect("PSD should parse");
        assert_eq!(doc.width, 32);
        assert_eq!(doc.height, 32);

        // 自動検出で「目/!通常」「目/!閉じ」および「口/!閉じ」「口/!あ」が見つかること
        assert_eq!(
            doc.detected_blink,
            Some(("目/!通常".to_string(), "目/!閉じ".to_string()))
        );
        assert_eq!(
            doc.detected_lipsync,
            Some(("口/!閉じ".to_string(), "口/!あ".to_string()))
        );

        // 初期状態での合成（目が黄色 [255, 215, 60]）
        let def_vis = doc.default_visibility();
        let (_, frame_default) = doc.composite_with_visibility(&def_vis);
        let eye_px_idx = ((10 * 32 + 16) * 4) as usize;
        assert_eq!(
            &frame_default.pixels[eye_px_idx..eye_px_idx + 3],
            &[255, 215, 60],
            "初期状態では『目/!通常』が表示されていること"
        );

        // PSDToolKit の `!` ラジオ排他規則で『目/!閉じ』を ON にすると『目/!通常』が自動で OFF になること
        let vis_closed = doc.compute_visibility_from_overrides("目/!閉じ=1");
        let (_, frame_closed) = doc.composite_with_visibility(&vis_closed);
        assert_eq!(
            &frame_closed.pixels[eye_px_idx..eye_px_idx + 3],
            &[40, 40, 60],
            "『目/!閉じ=1』指定で『目/!通常』が排他非表示になり閉じ目色になること"
        );

        // 目パチ＆音声連動口パクのアニメーション評価テスト
        let blink = PsdBlinkConfig {
            enabled: true,
            interval_sec: 2.0,
            close_frames: 3,
            open_path: String::new(),
            close_path: String::new(),
        };
        let lipsync = PsdLipSyncConfig {
            mode: PsdLipSyncMode::AudioSync,
            param: 1.0,
            close_path: String::new(),
            open_path: String::new(),
        };

        // 音声振幅が大きいフレーム (0.5) では口が『口/!あ』[240, 70, 90] になること
        let vis_talking = doc.evaluate_animated_visibility("", &blink, &lipsync, 0, 30.0, 0.5);
        let (_, frame_talking) = doc.composite_with_visibility(&vis_talking);
        let mouth_px_idx = ((20 * 32 + 16) * 4) as usize;
        assert_eq!(
            &frame_talking.pixels[mouth_px_idx..mouth_px_idx + 3],
            &[240, 70, 90],
            "音声振幅がある時は口が『!あ』に開くこと"
        );
    }

    #[test]
    fn test_parse_pfv_favorites() {
        let pfv = "[PSDToolKit Favorites]\r\n//笑顔セット\r\n眉/!通常\r\n目/!閉じ\r\n口/!あ\r\n//通常セット\r\n眉/!通常\r\n目/!通常\r\n口/!閉じ\r\n";
        let presets = parse_pfv_str(pfv);
        assert_eq!(presets.len(), 2);
        assert_eq!(presets[0].name, "笑顔セット");
        assert_eq!(presets[0].active_paths, vec!["眉/!通常", "目/!閉じ", "口/!あ"]);
    }

    #[test]
    fn test_psd_decomposition_by_folders_and_layers() {
        let psd_bytes = build_synthetic_character_psd(32, 32);
        let doc = PsdDocument::from_bytes(&psd_bytes).expect("PSD should parse");

        // 1. フォルダ単位分解: `*体`, `口`, `目` の3パーツ（背面→前面順）に分解されること
        let folder_parts = doc.decompose_by_folders("", true);
        assert_eq!(folder_parts.len(), 3);
        assert_eq!(folder_parts[0].label, "*体");
        assert_eq!(folder_parts[1].label, "口");
        assert!(folder_parts[1].inherit_lipsync);
        assert!(!folder_parts[1].inherit_blink);
        assert_eq!(folder_parts[2].label, "目");
        assert!(folder_parts[2].inherit_blink);
        assert!(!folder_parts[2].inherit_lipsync);

        // 分解された『目』パーツは `*体` を含まず、目自身のバウンディングボックス (16×8) に切り抜かれること
        let eye_part = &folder_parts[2];
        let eye_vis = doc.compute_visibility_from_overrides(&eye_part.psd_layers_param);
        let eye_crop = doc.parse_crop_rect(&eye_part.psd_layers_param);
        let (_, eye_frame) = doc.composite_with_visibility_and_crop(&eye_vis, eye_crop);
        assert_eq!(eye_frame.width, 16);
        assert_eq!(eye_frame.height, 8);
        assert_eq!(&eye_frame.pixels[0..4], &[255, 215, 60, 255]);

        // 分解後の『口』パーツに対して音声連動口パクを評価すると、『!あ』[240, 70, 90] に切り替わること
        let lipsync = PsdLipSyncConfig {
            mode: PsdLipSyncMode::AudioSync,
            param: 1.0,
            close_path: String::new(),
            open_path: String::new(),
        };
        let mouth_part = &folder_parts[1];
        let mouth_vis = doc.evaluate_animated_visibility(
            &mouth_part.psd_layers_param,
            &PsdBlinkConfig::default(),
            &lipsync,
            0,
            30.0,
            0.5,
        );
        let mouth_crop = doc.parse_crop_rect(&mouth_part.psd_layers_param);
        let (_, mouth_frame) = doc.composite_with_visibility_and_crop(&mouth_vis, mouth_crop);
        assert_eq!(&mouth_frame.pixels[0..4], &[240, 70, 90, 255]);

        // 2. レイヤー単位分解: 現在表示中の3枚のピクセルレイヤー (`*体`, `口/!閉じ`, `目/!通常`) に分解されること
        let layer_parts = doc.decompose_by_layers("", true);
        assert_eq!(layer_parts.len(), 3);
        assert_eq!(layer_parts[0].label, "*体");
        assert_eq!(layer_parts[1].label, "口/!閉じ");
        assert_eq!(layer_parts[2].label, "目/!通常");
    }
}
