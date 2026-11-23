//! 初代 AviUtl (.aup) および拡張編集 0.92 (80EE) のネイティブバイナリエンコーダ。

use au2_core::{Scene, SceneSettings};

use super::rle::{compress_aup, encode_sjis_fixed};
use super::tables::{
    get_checkbox_name, get_trackbar_name, get_trackbar_scale, BUILTIN_ANM_NAMES, BUILTIN_CAM_NAMES,
    BUILTIN_OBJ_NAMES, BUILTIN_SCN_NAMES, BUILTIN_SCN_TYPES, DEFAULT_EFFECT_TYPES,
};

/// AviUtl 本体で「拡張編集」および「拡張編集(音声)」を有効化する 2005 バイトの `#temp0` `FilterConfig` を生成します。
fn build_default_filter_config(width: u32, height: u32) -> Vec<u8> {
    let mut cfg = vec![0u8; 2005];
    let patches: &[(usize, &[u8])] = &[
        (0, &[0x41, 0x76, 0x69, 0x55, 0x74, 0x6C, 0x20, 0x46, 0x69, 0x6C, 0x74, 0x65, 0x72, 0x43, 0x6F, 0x6E, 0x66, 0x69, 0x67, 0x46, 0x69, 0x6C, 0x65, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x30, 0x2E, 0x31, 0x00, 0x2F, 0x00]),
        (40, &[0x8D, 0xC4, 0x90, 0xB6, 0x83, 0x45, 0x83, 0x42, 0x83, 0x93, 0x83, 0x68, 0x83, 0x45, 0x00, 0x04, 0x00]),
        (59, &[0x04, 0x00]),
        (63, &[0x04, 0x00]),
        (75, &[0x05, 0xD0, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0x00]),
        (87, &[0x83, 0x63, 0x81, 0x5B, 0x83, 0x8B, 0x83, 0x45, 0x83, 0x42, 0x83, 0x93, 0x83, 0x68, 0x83, 0x45, 0x00, 0x04, 0x00]),
        (108, &[0x04, 0x00]),
        (112, &[0x04, 0x00]),
        (124, &[0x05, 0xD0, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x2D, 0x00]),
        (136, &[0x83, 0x71, 0x83, 0x58, 0x83, 0x67, 0x83, 0x4F, 0x83, 0x89, 0x83, 0x80, 0x00, 0x04, 0x00]),
        (153, &[0x04, 0x00]),
        (157, &[0x04, 0x00]),
        (169, &[0x05, 0xD0, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x3F, 0x00]),
        (181, &[0x83, 0x6D, 0x83, 0x43, 0x83, 0x59, 0x8F, 0x9C, 0x8B, 0x8E, 0x83, 0x74, 0x83, 0x42, 0x83, 0x8B, 0x83, 0x5E, 0x00, 0x04, 0x00]),
        (204, &[0x04, 0x00]),
        (208, &[0x04, 0x00]),
        (212, &[0x0C, 0x00]),
        (223, &[0x02, 0x01, 0x00]),
        (229, &[0x01, 0x00]),
        (232, &[0x02, 0x00]),
        (236, &[0x18, 0x00]),
        (240, &[0x47, 0x00]),
        (244, &[0x83, 0x6D, 0x83, 0x43, 0x83, 0x59, 0x8F, 0x9C, 0x8B, 0x8E, 0x28, 0x8E, 0x9E, 0x8A, 0xD4, 0x8E, 0xB2, 0x29, 0x83, 0x74, 0x83, 0x42, 0x83, 0x8B, 0x83, 0x5E, 0x00, 0x04, 0x00]),
        (275, &[0x04, 0x00]),
        (279, &[0x04, 0x00]),
        (283, &[0x0C, 0x00]),
        (292, &[0x08, 0x00, 0x0A, 0x00]),
        (300, &[0x01, 0x00]),
        (303, &[0x03, 0x00]),
        (307, &[0x24, 0x00]),
        (311, &[0x41, 0x00]),
        (315, &[0x83, 0x56, 0x83, 0x83, 0x81, 0x5B, 0x83, 0x76, 0x83, 0x74, 0x83, 0x42, 0x83, 0x8B, 0x83, 0x5E, 0x00, 0x04, 0x00]),
        (336, &[0x04, 0x00]),
        (340, &[0x04, 0x00]),
        (344, &[0x10, 0x00]),
        (355, &[0x02, 0x02, 0x00]),
        (361, &[0x01, 0x00]),
        (364, &[0x02, 0x00]),
        (368, &[0x80, 0x00]),
        (373, &[0x04, 0x00]),
        (376, &[0x3F, 0x00]),
        (380, &[0x82, 0xDA, 0x82, 0xA9, 0x82, 0xB5, 0x83, 0x74, 0x83, 0x42, 0x83, 0x8B, 0x83, 0x5E, 0x00, 0x04, 0x00]),
        (399, &[0x04, 0x00]),
        (403, &[0x04, 0x00]),
        (407, &[0x10, 0x00]),
        (418, &[0x02, 0x03, 0x00]),
        (424, &[0x01, 0x00]),
        (427, &[0x02, 0x00]),
        (436, &[0x04, 0x00]),
        (439, &[0x57, 0x00]),
        (443, &[0x83, 0x4E, 0x83, 0x8A, 0x83, 0x62, 0x83, 0x73, 0x83, 0x93, 0x83, 0x4F, 0x81, 0x95, 0x83, 0x8A, 0x83, 0x54, 0x83, 0x43, 0x83, 0x59, 0x00, 0x05, 0x00]),
        (470, &[0x04, 0x00]),
        (474, &[0x04, 0x00]),
        (478, &[0x10, 0x00]),
        (486, &[0x0C, 0x00]),
        (491, &[0x44, 0x00, 0x02, 0x04, 0x00]),
        (526, &[0x45, 0x00]),
        (530, &[0x89, 0x8F, 0x93, 0x68, 0x82, 0xE8, 0x82, 0xC2, 0x82, 0xD4, 0x82, 0xB5, 0x00, 0x04, 0x00]),
        (547, &[0x04, 0x00]),
        (551, &[0x04, 0x00]),
        (555, &[0x10, 0x00]),
        (559, &[0x08, 0x00]),
        (566, &[0x02, 0x05, 0x00]),
        (595, &[0x41, 0x00]),
        (599, &[0x90, 0x46, 0x92, 0xB2, 0x95, 0xE2, 0x90, 0xB3, 0x00, 0x04, 0x00]),
        (612, &[0x04, 0x00]),
        (616, &[0x04, 0x00]),
        (620, &[0x18, 0x00]),
        (629, &[0x10, 0x00, 0x02, 0x7F, 0x00]),
        (660, &[0x75, 0x00]),
        (664, &[0x8A, 0x67, 0x92, 0xA3, 0x90, 0x46, 0x92, 0xB2, 0x95, 0xE2, 0x90, 0xB3, 0x00, 0x04, 0x00]),
        (681, &[0x04, 0x00]),
        (685, &[0x04, 0x00]),
        (689, &[0x3C, 0x00]),
        (693, &[0x0C, 0x00]),
        (698, &[0x10, 0x00, 0x02, 0x7F, 0x00]),
        (777, &[0x33, 0x00]),
        (781, &[0x89, 0xB9, 0x90, 0xBA, 0x82, 0xCC, 0x88, 0xCA, 0x92, 0x75, 0x92, 0xB2, 0x90, 0xAE, 0x00, 0x04, 0x00]),
        (800, &[0x04, 0x00]),
        (804, &[0x04, 0x00]),
        (808, &[0x04, 0x00]),
        (817, &[0x40, 0x20, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (828, &[0x2F, 0x00]),
        (832, &[0x89, 0xB9, 0x97, 0xCA, 0x82, 0xCC, 0x92, 0xB2, 0x90, 0xAE, 0x00, 0x04, 0x00]),
        (847, &[0x04, 0x00]),
        (851, &[0x04, 0x00]),
        (855, &[0x04, 0x00]),
        (865, &[0x20, 0x02, 0x06, 0x00]),
        (875, &[0x31, 0x00]),
        (879, &[0x89, 0xB9, 0x97, 0xCA, 0x82, 0xCC, 0x8D, 0xC5, 0x91, 0xE5, 0x89, 0xBB, 0x00, 0x04, 0x00]),
        (896, &[0x04, 0x00]),
        (900, &[0x04, 0x00]),
        (904, &[0x04, 0x00]),
        (913, &[0x08, 0x20, 0x02, 0x00]),
        (921, &[0x01, 0x00]),
        (924, &[0x55, 0x00]),
        (928, &[0x83, 0x54, 0x83, 0x43, 0x83, 0x59, 0x82, 0xCC, 0x95, 0xCF, 0x8D, 0x58, 0x00, 0x05, 0x00]),
        (945, &[0x04, 0x00]),
        (949, &[0x04, 0x00]),
        (957, &[0x1C, 0x00]),
        (961, &[0x08, 0x00]),
        (965, &[0x2D, 0x06, 0x00, 0x02, 0x07, 0x00]),
        (973, &[0x01, 0x00]),
        (1009, &[0x4D, 0x00]),
        (1013, &[0x83, 0x74, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x80, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x67, 0x82, 0xCC, 0x95, 0xCF, 0x8D, 0x58, 0x00, 0x04, 0x00]),
        (1038, &[0x04, 0x00]),
        (1042, &[0x04, 0x00]),
        (1050, &[0x18, 0x00]),
        (1054, &[0xAD, 0x02, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00]),
        (1086, &[0x39, 0x00]),
        (1090, &[0x83, 0x74, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x80, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x67, 0x82, 0xCC, 0x90, 0xDD, 0x92, 0xE8, 0x00, 0x04, 0x00]),
        (1115, &[0x04, 0x00]),
        (1119, &[0x04, 0x00]),
        (1127, &[0x04, 0x00]),
        (1131, &[0x15, 0x03, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (1143, &[0x51, 0x01, 0x00]),
        (1147, &[0x83, 0x43, 0x83, 0x93, 0x83, 0x5E, 0x81, 0x5B, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x58, 0x82, 0xCC, 0x89, 0xF0, 0x8F, 0x9C, 0x00, 0x05, 0x00]),
        (1172, &[0x04, 0x00]),
        (1176, &[0x04, 0x00]),
        (1184, &[0x18, 0x00]),
        (1189, &[0x01, 0x00]),
        (1192, &[0xAD, 0x06, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00]),
        (1224, &[0x82, 0xC8, 0x82, 0xB5, 0x00]),
        (1480, &[0x3B, 0x00]),
        (1484, &[0x83, 0x74, 0x83, 0x42, 0x81, 0x5B, 0x83, 0x8B, 0x83, 0x68, 0x83, 0x49, 0x81, 0x5B, 0x83, 0x5F, 0x81, 0x5B, 0x00, 0x04, 0x00]),
        (1507, &[0x04, 0x00]),
        (1511, &[0x04, 0x00]),
        (1519, &[0x08, 0x00]),
        (1523, &[0xA5, 0x03, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (1535, &[0x01, 0x00]),
        (1539, &[0x25, 0x00]),
        (1543, &[0x82, 0xC8, 0x82, 0xB5, 0x00, 0x04, 0x00]),
        (1552, &[0x04, 0x00]),
        (1556, &[0x04, 0x00]),
        (1568, &[0x05, 0x00, 0x10, 0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x2D, 0x00]),
        (1580, &[0x8A, 0xEF, 0x90, 0x94, 0x00, 0x04, 0x00]),
        (1589, &[0x04, 0x00]),
        (1593, &[0x04, 0x00]),
        (1601, &[0x08, 0x00]),
        (1605, &[0x05, 0x00]),
        (1608, &[0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00]),
        (1617, &[0x01, 0x00]),
        (1621, &[0x2D, 0x00]),
        (1625, &[0x8B, 0xF4, 0x90, 0x94, 0x00, 0x04, 0x00]),
        (1634, &[0x04, 0x00]),
        (1638, &[0x04, 0x00]),
        (1646, &[0x08, 0x00]),
        (1650, &[0x05, 0x00]),
        (1653, &[0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00]),
        (1662, &[0x01, 0x00]),
        (1666, &[0x27, 0x00]),
        (1670, &[0x93, 0xF1, 0x8F, 0x64, 0x89, 0xBB, 0x00, 0x04, 0x00]),
        (1681, &[0x04, 0x00]),
        (1685, &[0x04, 0x00]),
        (1697, &[0x05, 0x00, 0x10, 0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x36, 0x00]),
        (1709, &[0x8E, 0xA9, 0x93, 0xAE, 0x32, 0x34, 0x66, 0x70, 0x73, 0x00, 0x04, 0x00]),
        (1723, &[0x04, 0x00]),
        (1727, &[0x04, 0x00]),
        (1731, &[0x08, 0x00]),
        (1735, &[0x04, 0x00]),
        (1739, &[0x05, 0x00]),
        (1742, &[0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x40, 0x00]),
        (1751, &[0x10, 0x00]),
        (1759, &[0x25, 0x00]),
        (1763, &[0x8E, 0xA9, 0x93, 0xAE, 0x00, 0x04, 0x00]),
        (1772, &[0x04, 0x00]),
        (1776, &[0x04, 0x00]),
        (1788, &[0x05, 0x00, 0x10, 0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x3D, 0x00]),
        (1800, &[0x90, 0x46, 0x95, 0xCF, 0x8A, 0xB7, 0x82, 0xCC, 0x90, 0xDD, 0x92, 0xE8, 0x00, 0x04, 0x00]),
        (1817, &[0x04, 0x00]),
        (1821, &[0x04, 0x00]),
        (1829, &[0x10, 0x00]),
        (1833, &[0xAD, 0x02, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (1853, &[0x01, 0x00]),
        (1857, &[0x3C, 0x00]),
        (1861, &[0x5F, 0x5F, 0x43, 0x4F, 0x4C, 0x4F, 0x52, 0x5F, 0x4F, 0x55, 0x54, 0x00, 0x04, 0x00]),
        (1877, &[0x04, 0x00]),
        (1881, &[0x04, 0x00]),
        (1889, &[0x10, 0x00]),
        (1893, &[0x25, 0x03, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (1913, &[0x01, 0x00]),
        (1917, &[0x29, 0x00]),
        (1921, &[0x8A, 0x67, 0x92, 0xA3, 0x95, 0xD2, 0x8F, 0x57, 0x00, 0x04, 0x00]),
        (1934, &[0x04, 0x00]),
        (1938, &[0x04, 0x00]),
        (1950, &[0x07, 0x20, 0x06, 0x40, 0x08, 0x00]),
        (1958, &[0x2F, 0x00]),
        (1962, &[0x8A, 0x67, 0x92, 0xA3, 0x95, 0xD2, 0x8F, 0x57, 0x28, 0x89, 0xB9, 0x90, 0xBA, 0x29, 0x00, 0x04, 0x00]),
        (1981, &[0x04, 0x00]),
        (1985, &[0x04, 0x00]),
        (1997, &[0x05, 0x00, 0x35, 0x00, 0x09, 0x00]),
    ];
    for &(off, bytes) in patches {
        cfg[off..off + bytes.len()].copy_from_slice(bytes);
    }
    cfg[1001..1005].copy_from_slice(&(width as i32).to_le_bytes());
    cfg[1005..1009].copy_from_slice(&(height as i32).to_le_bytes());
    cfg
}

/// 単一シーンを本家 AviUtl 1.10 + 拡張編集 0.92 互換の `.aup` バイナリとして出力します。
pub fn write_legacy_aup(scene: &Scene) -> Vec<u8> {
    let mut scenes = std::collections::BTreeMap::new();
    scenes.insert(0, scene.clone());
    write_legacy_aup_multi_scene(&scenes, 0, 0)
}

/// マルチシーンプロジェクトを本家 AviUtl 1.10 (`EditHandle` + `#temp0`) + 拡張編集 0.92 (`80EE`) 完全互換の `.aup` バイナリとして出力します。
pub fn write_legacy_aup_multi_scene(
    scenes: &std::collections::BTreeMap<u32, Scene>,
    editing_scene: u32,
    cursor_frame: au2_core::Frame,
) -> Vec<u8> {
    let fallback_scene = Scene::new("Root", SceneSettings::default());
    let main_scene = scenes
        .get(&editing_scene)
        .or_else(|| scenes.get(&0))
        .or_else(|| scenes.values().next())
        .unwrap_or(&fallback_scene);

    let width = main_scene.settings.width.max(16);
    let height = main_scene.settings.height.max(16);
    let video_rate = main_scene.settings.frame_rate.num.max(1);
    let video_scale = main_scene.settings.frame_rate.den.max(1);
    let audio_rate = main_scene.settings.audio_rate.max(8000);
    let audio_ch = main_scene.settings.audio_channels.max(1);
    let frame_num = main_scene
        .settings
        .length
        .max(main_scene.content_end())
        .max(1);

    let edit_filename = if video_scale == 1 {
        format!("{}x{}_{}fps_{}Hz.exedit", width, height, video_rate, audio_rate)
    } else {
        format!(
            "{}x{}_{}-{}fps_{}Hz.exedit",
            width, height, video_rate, video_scale, audio_rate
        )
    };

    let mut out = Vec::with_capacity(65536);

    // 1. 先頭ヘッダ (32 bytes)
    out.extend_from_slice(crate::AUP_HEADER_018);

    // 2. EditHandle チャンク
    out.extend_from_slice(&0x004C_09E8i32.to_le_bytes()); // Size
    out.extend_from_slice(&0x0000_0001u32.to_le_bytes()); // Flag (EDIT_HANDLE_FLAG_OPEN)
    out.extend_from_slice(&encode_sjis_fixed(&edit_filename, 260)); // EditFilename
    out.extend_from_slice(&[0u8; 260]); // OutputFilename

    // EditHandle.Data (0x4C09E8 - 0x20C = 0x4C07DC bytes)
    const EH_UNCOMP_OFFSET: usize = 0x20C;
    const EH_DATA_LEN: usize = 0x4C09E8 - EH_UNCOMP_OFFSET;
    let mut eh_data = vec![0u8; EH_DATA_LEN];
    let put_u32 = |buf: &mut [u8], struct_off: usize, val: u32| {
        let rel = struct_off - EH_UNCOMP_OFFSET;
        buf[rel..rel + 4].copy_from_slice(&val.to_le_bytes());
    };
    let samples_per_frame =
        ((audio_rate as u64 * video_scale as u64) / (video_rate as u64).max(1)) as u32;

    put_u32(&mut eh_data, 0x000310, width);
    put_u32(&mut eh_data, 0x000314, height);
    put_u32(&mut eh_data, 0x000318, frame_num);
    put_u32(&mut eh_data, 0x00031C, 0);
    put_u32(&mut eh_data, 0x000320, frame_num.saturating_sub(1));
    put_u32(&mut eh_data, 0x000328, width);
    put_u32(&mut eh_data, 0x00032C, height);
    put_u32(&mut eh_data, 0x000330, cursor_frame.min(frame_num.saturating_sub(1)));
    put_u32(&mut eh_data, 0x000338, 40);
    put_u32(&mut eh_data, 0x00033C, width);
    put_u32(&mut eh_data, 0x000340, height);
    put_u32(&mut eh_data, 0x000344, 0x0018_0001);
    put_u32(&mut eh_data, 0x00034C, width.saturating_mul(height).saturating_mul(3));
    put_u32(&mut eh_data, 0x000368, samples_per_frame.saturating_mul(2));
    put_u32(&mut eh_data, 0x00036C, 0x0008_000D);
    put_u32(&mut eh_data, 0x000378, width / 2);
    put_u32(&mut eh_data, 0x00037C, 636);
    put_u32(&mut eh_data, 0x000380, 1);
    put_u32(&mut eh_data, 0x0003A8, width / 2);
    put_u32(&mut eh_data, 0x0003AC, height / 2);
    put_u32(&mut eh_data, 0x0003B0, width);
    put_u32(&mut eh_data, 0x0003B4, height);

    for base in [0x0003C0usize, 0x026F9Cusize] {
        put_u32(&mut eh_data, base + 0x0000, frame_num);
        put_u32(&mut eh_data, base + 0x0004, width);
        put_u32(&mut eh_data, base + 0x0008, height);
        put_u32(&mut eh_data, base + 0x0010, 40);
        put_u32(&mut eh_data, base + 0x0014, width);
        put_u32(&mut eh_data, base + 0x0018, height);
        put_u32(&mut eh_data, base + 0x001C, 0x0030_0001);
        put_u32(&mut eh_data, base + 0x0020, 0x3834_4359); // 'YC48'
        put_u32(&mut eh_data, base + 0x0024, width.saturating_mul(height).saturating_mul(6));
        put_u32(&mut eh_data, base + 0x0038, 0x0001 | (audio_ch << 16));
        put_u32(&mut eh_data, base + 0x003C, audio_rate);
        put_u32(&mut eh_data, base + 0x0040, audio_rate * audio_ch * 2);
        put_u32(&mut eh_data, base + 0x0044, (audio_ch * 2) | (16 << 16));
        put_u32(&mut eh_data, base + 0x0050, 3);
        put_u32(&mut eh_data, base + 0x0054, video_rate);
        put_u32(&mut eh_data, base + 0x0058, 1);
        put_u32(&mut eh_data, base + 0x005C, 1);
        put_u32(&mut eh_data, base + 0x0064, 40);
        put_u32(&mut eh_data, base + 0x0068, 0x7FFF_FFFF);
        put_u32(&mut eh_data, base + 0x0070, 18);
        put_u32(&mut eh_data, base + 0x0074, 0x3834_4359); // 'YC48'
        put_u32(&mut eh_data, base + 0x0094, 0x7364_6976); // 'vids'
        put_u32(&mut eh_data, base + 0x0098, 0x3834_4359); // 'YC48'
        put_u32(&mut eh_data, base + 0x00A8, video_scale);
        put_u32(&mut eh_data, base + 0x00AC, video_rate);
        put_u32(&mut eh_data, base + 0x00B4, 1);
        put_u32(&mut eh_data, base + 0x0120, 0x7364_7561); // 'auds'
        put_u32(&mut eh_data, base + 0x0134, audio_ch * 2);
        put_u32(&mut eh_data, base + 0x0138, audio_rate * audio_ch * 2);
        put_u32(&mut eh_data, base + 0x0140, 0x7FFF_FFFF);
        put_u32(&mut eh_data, base + 0x0150, audio_ch * 2);
        put_u32(&mut eh_data, base + 0x01AC, 40);
        put_u32(&mut eh_data, base + 0x01B0, width);
        put_u32(&mut eh_data, base + 0x01B4, height);
        put_u32(&mut eh_data, base + 0x01B8, 0x0030_0001);
        put_u32(&mut eh_data, base + 0x01BC, 0x3834_4359);
        put_u32(&mut eh_data, base + 0x01C0, width.saturating_mul(height).saturating_mul(6));
        put_u32(&mut eh_data, base + 0x21AC, 40);
        put_u32(&mut eh_data, base + 0x21B0, 0x0001 | (audio_ch << 16));
        put_u32(&mut eh_data, base + 0x21B4, audio_rate);
        put_u32(&mut eh_data, base + 0x21B8, audio_rate * audio_ch * 2);
        put_u32(&mut eh_data, base + 0x21BC, (audio_ch * 2) | (16 << 16));
        put_u32(&mut eh_data, base + 0x41B0, 18);
    }

    put_u32(&mut eh_data, 0x00C75C, samples_per_frame);
    put_u32(&mut eh_data, 0x020CF4, 1);
    eh_data[0x020D18 - EH_UNCOMP_OFFSET..0x020D18 - EH_UNCOMP_OFFSET + 7]
        .copy_from_slice(b"#temp0\0");
    let fn_bytes = encode_sjis_fixed(&edit_filename, 260);
    eh_data[0x026E98 - EH_UNCOMP_OFFSET..0x026E98 - EH_UNCOMP_OFFSET + 260]
        .copy_from_slice(&fn_bytes);
    eh_data[0x4BBD98 - EH_UNCOMP_OFFSET..0x4BC198 - EH_UNCOMP_OFFSET].fill(0xFF);
    let out_plugin_sjis: &[u8] = &[
        0x8A, 0x67, 0x92, 0xA3, 0x95, 0xD2, 0x8F, 0x57, 0x41, 0x56, 0x49, 0x2F, 0x42, 0x4D, 0x50,
        0x8F, 0x6F, 0x97, 0xCD, 0x20, 0x28, 0x52, 0x47, 0x42, 0x41, 0x29, 0x00,
    ];
    eh_data[0x4BC49C - EH_UNCOMP_OFFSET..0x4BC49C - EH_UNCOMP_OFFSET + out_plugin_sjis.len()]
        .copy_from_slice(out_plugin_sjis);

    out.extend_from_slice(&compress_aup(&eh_data));

    // FrameStatus (10 配列)
    let fn_usize = frame_num as usize;
    out.extend_from_slice(&(frame_num as i32).to_le_bytes());
    let zero_u32_bytes = vec![0u8; fn_usize * 4];
    let mut audio_u32_bytes = Vec::with_capacity(fn_usize * 4);
    for i in 0..frame_num {
        audio_u32_bytes.extend_from_slice(&i.to_le_bytes());
    }
    let zero_u8_bytes = vec![0u8; fn_usize];
    let mut edit_flag_bytes = vec![0u8; fn_usize];
    if !edit_flag_bytes.is_empty() {
        edit_flag_bytes[0] = 1; // 先頭フレームにキーフレームフラグ
    }

    out.extend_from_slice(&compress_aup(&zero_u32_bytes)); // videos
    out.extend_from_slice(&compress_aup(&audio_u32_bytes)); // audios
    out.extend_from_slice(&compress_aup(&zero_u32_bytes)); // field2
    out.extend_from_slice(&compress_aup(&zero_u32_bytes)); // field3
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // inter
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // index24fps
    out.extend_from_slice(&compress_aup(&edit_flag_bytes)); // editFlag
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // config (#temp0 = 0)
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // vcm
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // clip

    // FilterConfigs[0] (#temp0)
    let filter_cfg = build_default_filter_config(width, height);
    out.extend_from_slice(&(filter_cfg.len() as i32).to_le_bytes());
    out.extend_from_slice(&filter_cfg);

    // EditHandle 終端フッタ (32 bytes)
    out.extend_from_slice(crate::AUP_HEADER_018);

    // 3. FilterProject 0.1 "拡張編集" (80EE) チャンク
    out.extend_from_slice(b"FilterProject 0.1\0");
    let exedit_name_sjis: &[u8] = &[0x8A, 0x67, 0x92, 0xA3, 0x95, 0xD2, 0x8F, 0x57, 0x00];
    out.extend_from_slice(&(exedit_name_sjis.len() as i32).to_le_bytes());
    out.extend_from_slice(exedit_name_sjis);

    let exedit_data = build_exedit_80ee_bytes(scenes, editing_scene, cursor_frame);
    out.extend_from_slice(&(exedit_data.len() as i32).to_le_bytes());
    out.extend_from_slice(&compress_aup(&exedit_data));

    out
}

fn build_exedit_80ee_bytes(
    scenes: &std::collections::BTreeMap<u32, Scene>,
    editing_scene: u32,
    cursor_frame: au2_core::Frame,
) -> Vec<u8> {
    // 1. Layers (76 bytes each)
    let mut layer_records: Vec<[u8; 76]> = Vec::new();
    for (&sc_id, sc) in scenes {
        if sc_id >= 50 {
            continue;
        }
        for (li, layer) in sc.layers.iter().enumerate() {
            if li >= 100 {
                break;
            }
            let has_name = layer.name.as_ref().is_some_and(|n| !n.is_empty());
            if has_name || !layer.visible || layer.locked || layer.coordinate_link || layer.clip_above {
                let mut rec = [0u8; 76];
                rec[0..4].copy_from_slice(&sc_id.to_le_bytes());
                rec[4..8].copy_from_slice(&(li as u32).to_le_bytes());
                let mut flag = 0u32;
                if !layer.visible {
                    flag |= 1;
                }
                if layer.locked {
                    flag |= 2;
                }
                if layer.coordinate_link {
                    flag |= 0x10;
                }
                if layer.clip_above {
                    flag |= 0x20;
                }
                rec[8..12].copy_from_slice(&flag.to_le_bytes());
                if let Some(name) = &layer.name {
                    let sjis = encode_sjis_fixed(name, 64);
                    rec[12..76].copy_from_slice(&sjis);
                }
                layer_records.push(rec);
            }
        }
    }

    // 2. Scenes (50 scenes * 220 bytes each)
    let mut scene_records: Vec<[u8; 220]> = Vec::with_capacity(50);
    for sc_id in 0..50u32 {
        let mut rec = [0u8; 220];
        rec[0..4].copy_from_slice(&sc_id.to_le_bytes());
        let flag = 1u32; // SceneFlag.Base
        rec[4..8].copy_from_slice(&flag.to_le_bytes());
        if let Some(sc) = scenes.get(&sc_id) {
            let default_name = format!("Scene {}", sc_id);
            if sc_id > 0 && !sc.name.is_empty() && sc.name != "Root" && sc.name != default_name {
                let sjis = encode_sjis_fixed(&sc.name, 64);
                rec[8..72].copy_from_slice(&sjis);
            }
            if sc_id > 0 {
                rec[0x48..0x4C].copy_from_slice(&sc.settings.width.to_le_bytes());
                rec[0x4C..0x50].copy_from_slice(&sc.settings.height.to_le_bytes());
                rec[0x50..0x54].copy_from_slice(&sc.settings.length.to_le_bytes());
            }
        }
        let cur = if sc_id == editing_scene { cursor_frame } else { 0 };
        rec[0x54..0x58].copy_from_slice(&cur.to_le_bytes());
        rec[0x58..0x5C].copy_from_slice(&12u32.to_le_bytes()); // Zoom
        rec[0x60..0x64].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // EditingObject
        rec[0x70..0x74].copy_from_slice(&1_200_000u32.to_le_bytes()); // BpmGridTempo
        rec[0x74..0x78].copy_from_slice(&1u32.to_le_bytes()); // BpmGridOffset
        rec[0x7C..0x80].copy_from_slice(&200u32.to_le_bytes()); // XYGridWidth
        rec[0x80..0x84].copy_from_slice(&200u32.to_le_bytes()); // XYGridHeight
        rec[0x88..0x8C].copy_from_slice(&200u32.to_le_bytes()); // CameraGridSize
        rec[0x8C..0x90].copy_from_slice(&10u32.to_le_bytes()); // CameraGridNum
        rec[0x94..0x98].copy_from_slice(&18u32.to_le_bytes()); // OutsideFrameScale
        rec[0x98..0x9C].copy_from_slice(&4u32.to_le_bytes()); // BpmGridBeat
        scene_records.push(rec);
    }

    // 3. TrackbarScripts & Objects
    let mut trackbar_scripts: Vec<String> = vec!["補間移動".to_string(), "回転".to_string()];
    let mut obj_blobs: Vec<Vec<u8>> = Vec::new();
    let mut obj_counter: u32 = 0;

    for (&sc_id, sc) in scenes {
        if sc_id >= 50 {
            continue;
        }
        let fps = sc.settings.frame_rate.as_f64().max(1.0);
        for (li, layer) in sc.layers.iter().enumerate() {
            if li >= 100 {
                break;
            }
            let mut sorted_objs: Vec<&au2_core::TimelineObject> = layer.objects.iter().collect();
            sorted_objs.sort_by_key(|o| o.frame_start);
            for obj in sorted_objs {
                encode_timeline_object_to_80ee(
                    obj,
                    sc_id,
                    li as u32,
                    editing_scene,
                    fps,
                    &mut trackbar_scripts,
                    &mut obj_blobs,
                    &mut obj_counter,
                );
            }
        }
    }

    // 4. EffectTypes (108 * 112 bytes)
    let mut effect_type_bytes = Vec::with_capacity(DEFAULT_EFFECT_TYPES.len() * 112);
    for &(_id, flag, tb_num, cb_num, ext_size, name) in DEFAULT_EFFECT_TYPES {
        let mut et = [0u8; 112];
        et[0..4].copy_from_slice(&flag.to_le_bytes());
        et[4..8].copy_from_slice(&(tb_num as u32).to_le_bytes());
        et[8..12].copy_from_slice(&(cb_num as u32).to_le_bytes());
        et[12..16].copy_from_slice(&(ext_size as u32).to_le_bytes());
        let sjis = encode_sjis_fixed(name, 96);
        et[16..112].copy_from_slice(&sjis);
        effect_type_bytes.extend_from_slice(&et);
    }

    // 5. 80EE 全体バッファの構築
    let total_size = 0x100
        + layer_records.len() * 76
        + scene_records.len() * 220
        + trackbar_scripts.len() * 128
        + effect_type_bytes.len()
        + obj_blobs.iter().map(|b| b.len()).sum::<usize>();
    let mut data = vec![0u8; 0x100];
    data.reserve(total_size.saturating_sub(0x100));

    data[0..4].copy_from_slice(b"80EE");
    data[4..8].copy_from_slice(&(DEFAULT_EFFECT_TYPES.len() as u32).to_le_bytes());
    data[8..12].copy_from_slice(&(obj_blobs.len() as u32).to_le_bytes());
    data[0x0C..0x10].copy_from_slice(&0x0001_1A8Bu32.to_le_bytes());
    data[0x10..0x14].copy_from_slice(&12u32.to_le_bytes()); // Zoom
    data[0x14..0x18].copy_from_slice(&400u32.to_le_bytes());
    data[0x18..0x1C].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // EditingObject
    data[0x1C..0x20].copy_from_slice(&0x0000_05C8u32.to_le_bytes()); // BaseSize = 0x5C8
    data[0x20..0x24].copy_from_slice(&5000u32.to_le_bytes());
    data[0x24..0x28].copy_from_slice(&1_000_000u32.to_le_bytes());
    data[0x28..0x2C].copy_from_slice(&50u32.to_le_bytes());
    data[0x2C..0x30].copy_from_slice(&0x0000_23F0u32.to_le_bytes()); // Version 0.92 (9200)
    data[0x30..0x34].copy_from_slice(&0u32.to_le_bytes()); // EnableBpmGrid
    data[0x34..0x38].copy_from_slice(&1_200_000u32.to_le_bytes()); // BpmGridTempo
    data[0x38..0x3C].copy_from_slice(&1u32.to_le_bytes()); // BpmGridOffset
    data[0x3C..0x40].copy_from_slice(&0u32.to_le_bytes()); // EnableXYGrid
    data[0x40..0x44].copy_from_slice(&200u32.to_le_bytes()); // XYGridWidth
    data[0x44..0x48].copy_from_slice(&200u32.to_le_bytes()); // XYGridHeight
    data[0x48..0x4C].copy_from_slice(&0u32.to_le_bytes()); // EnableCameraGrid
    data[0x4C..0x50].copy_from_slice(&200u32.to_le_bytes()); // CameraGridWidth
    data[0x50..0x54].copy_from_slice(&10u32.to_le_bytes()); // CameraGridHeight
    data[0x54..0x58].copy_from_slice(&0u32.to_le_bytes()); // ShowOutsideFrame
    data[0x58..0x5C].copy_from_slice(&18u32.to_le_bytes()); // OutsideFrameScale
    data[0x5C..0x60].copy_from_slice(&4u32.to_le_bytes()); // BpmGridBeat
    data[0x60..0x64].copy_from_slice(&0u32.to_le_bytes()); // Field0x60
    data[0x64..0x68].copy_from_slice(&editing_scene.to_le_bytes());
    data[0x68..0x6C].copy_from_slice(&(scene_records.len() as u32).to_le_bytes());
    data[0x6C..0x70].copy_from_slice(&(layer_records.len() as u32).to_le_bytes());
    data[0x70..0x74].copy_from_slice(&220u32.to_le_bytes()); // Scene.Size
    data[0x74..0x78].copy_from_slice(&76u32.to_le_bytes()); // Layer.Size
    data[0x78..0x7C].copy_from_slice(&2u32.to_le_bytes()); // Field0x78
    data[0x7C..0x80].copy_from_slice(&(trackbar_scripts.len() as u32).to_le_bytes());

    for rec in &layer_records {
        data.extend_from_slice(rec);
    }
    for rec in &scene_records {
        data.extend_from_slice(rec);
    }
    for ts in &trackbar_scripts {
        data.extend_from_slice(&encode_sjis_fixed(ts, 128));
    }
    data.extend_from_slice(&effect_type_bytes);
    for blob in &obj_blobs {
        data.extend_from_slice(blob);
    }

    data
}

/// OpenAvi / `.aup2` のエフェクトを拡張編集 0.92 の `EffectType` に解決します。
fn map_effect_to_legacy_type(
    eff: &au2_core::Effect,
) -> Option<(u32, u32, usize, usize, usize, &'static str, Option<String>)> {
    let raw_name = eff.name.as_str();
    let (legacy_name, implicit_script_name): (&str, Option<String>) = match raw_name {
        "映像再生" => ("標準描画", None),
        "音声再生" | "基本出力" => ("標準再生", None),
        "ドロップシャドウ" => ("シャドー", None),
        "時間制御(オブジェクト)" => ("時間制御", None),
        "グループ制御(音声)" => ("グループ制御", None),
        "パーティクル出力(カメラ制御)" => ("パーティクル出力", None),
        other => {
            if BUILTIN_ANM_NAMES.contains(&other) {
                ("アニメーション効果", Some(other.to_string()))
            } else if BUILTIN_OBJ_NAMES.contains(&other) {
                ("カスタムオブジェクト", Some(other.to_string()))
            } else if BUILTIN_CAM_NAMES.contains(&other) {
                ("カメラ効果", Some(other.to_string()))
            } else if BUILTIN_SCN_TYPES.contains(&other) || BUILTIN_SCN_NAMES.contains(&other) {
                ("シーンチェンジ", Some(other.to_string()))
            } else {
                (other, None)
            }
        }
    };

    DEFAULT_EFFECT_TYPES
        .iter()
        .find(|&&(_, _, _, _, _, n)| n == legacy_name)
        .map(|&(id, flag, tb_num, cb_num, ext_size, name)| {
            (id, flag, tb_num, cb_num, ext_size, name, implicit_script_name)
        })
}

fn encode_timeline_object_to_80ee(
    obj: &au2_core::TimelineObject,
    scene_id: u32,
    layer_id: u32,
    editing_scene: u32,
    fps: f64,
    trackbar_scripts: &mut Vec<String>,
    obj_blobs: &mut Vec<Vec<u8>>,
    obj_counter: &mut u32,
) {
    // 1. エフェクト列を ExEdit 0.92 の最大 12 エフェクトに変換
    let mut resolved_effects: Vec<(
        u32,
        u32,
        usize,
        usize,
        usize,
        &'static str,
        Option<String>,
        std::borrow::Cow<'_, au2_core::Effect>,
    )> = Vec::new();

    for eff in &obj.effects {
        if let Some((id, flag, tb, cb, ext, name, script_name)) = map_effect_to_legacy_type(eff) {
            resolved_effects.push((
                id,
                flag,
                tb,
                cb,
                ext,
                name,
                script_name,
                std::borrow::Cow::Borrowed(eff),
            ));
            if resolved_effects.len() >= 12 {
                break;
            }
        }
    }

    if resolved_effects.is_empty() {
        return;
    }

    // メディアオブジェクトまたは音声オブジェクトに描画/再生フィルタが欠けている場合は自動補完
    let first_id = resolved_effects[0].0;
    let is_visual_media = matches!(first_id, 0 | 1 | 3 | 4 | 5 | 6 | 7 | 9 | 80);
    let is_audio_media = matches!(first_id, 2 | 8);
    let has_draw = resolved_effects.iter().any(|e| matches!(e.0, 10 | 11 | 13));
    let has_playback = resolved_effects.iter().any(|e| e.0 == 12);

    if is_visual_media && !has_draw && resolved_effects.len() < 12 {
        let mut draw_eff = au2_core::Effect::new("標準描画");
        draw_eff.push_track("X", au2_core::Track::constant(0.0, 1));
        draw_eff.push_track("Y", au2_core::Track::constant(0.0, 1));
        draw_eff.push_track("Z", au2_core::Track::constant(0.0, 1));
        draw_eff.push_track("拡大率", au2_core::Track::constant(100.0, 2));
        draw_eff.push_track("透明度", au2_core::Track::constant(0.0, 1));
        draw_eff.push_track("回転", au2_core::Track::constant(0.0, 2));
        if let Some((id, flag, tb, cb, ext, name, sn)) = map_effect_to_legacy_type(&draw_eff) {
            resolved_effects.push((
                id,
                flag,
                tb,
                cb,
                ext,
                name,
                sn,
                std::borrow::Cow::Owned(draw_eff),
            ));
        }
    } else if is_audio_media && !has_playback && resolved_effects.len() < 12 {
        let mut play_eff = au2_core::Effect::new("標準再生");
        play_eff.push_track("音量", au2_core::Track::constant(100.0, 1));
        play_eff.push_track("左右", au2_core::Track::constant(0.0, 1));
        if let Some((id, flag, tb, cb, ext, name, sn)) = map_effect_to_legacy_type(&play_eff) {
            resolved_effects.push((
                id,
                flag,
                tb,
                cb,
                ext,
                name,
                sn,
                std::borrow::Cow::Owned(play_eff),
            ));
        }
    }

    // トラックバー数上限(64)およびチェックボックス数上限(48)に収まるように調整
    let mut tb_sum = 0usize;
    let mut cb_sum = 0usize;
    let mut valid_len = 0usize;
    for (_, _, tb, cb, _, _, _, _) in &resolved_effects {
        if tb_sum + tb > 64 || cb_sum + cb > 48 {
            break;
        }
        tb_sum += tb;
        cb_sum += cb;
        valid_len += 1;
    }
    resolved_effects.truncate(valid_len);
    if resolved_effects.is_empty() {
        return;
    }

    // 2. オブジェクトフラグの構築
    let mut obj_flag: u32 = 0x0000_0001; // TimelineObjectFlag.Enable
    if obj.clipping {
        obj_flag |= 0x0000_0100;
    }
    if obj.camera_target {
        obj_flag |= 0x0000_0200;
    }
    match first_id {
        0 | 1 | 3 | 4 | 5 | 6 | 7 | 9 | 80 => {
            obj_flag |= 0x0001_0000; // Media
        }
        2 | 8 => {
            obj_flag |= 0x0001_0000 | 0x0002_0000; // Media | Audio
        }
        93 | 95 | 96 | 97 | 98 | 99 => {
            obj_flag |= 0x0008_0000; // Control
            let range = resolved_effects[0]
                .7
                .text("range")
                .or_else(|| resolved_effects[0].7.text("対象レイヤー数"))
                .and_then(|s| s.trim().parse::<i32>().ok())
                .unwrap_or(0);
            if range > 0 {
                obj_flag |= 0x0010_0000; // Range
            }
        }
        94 => {
            obj_flag |= 0x0004_0000 | 0x0008_0000; // MediaFilter | Control (グループ制御)
            let range = resolved_effects[0]
                .7
                .text("range")
                .or_else(|| resolved_effects[0].7.text("対象レイヤー数"))
                .and_then(|s| s.trim().parse::<i32>().ok())
                .unwrap_or(0);
            if range > 0 {
                obj_flag |= 0x0010_0000;
            }
        }
        _ => {
            obj_flag |= 0x0004_0000; // MediaFilter (フィルタオブジェクト)
        }
    }
    if obj.audio {
        obj_flag |= 0x0002_0000;
    }

    // プレビュー文字列
    let preview_name = resolved_effects[0].5;
    let preview_sjis = encode_sjis_fixed(preview_name, 64);

    // 3. 中間点セグメントへの分割
    let keys = obj.key_frames();
    let seg_count = keys.len().saturating_sub(1).max(1);
    let chain_group = if seg_count > 1 {
        *obj_counter
    } else {
        0xFFFF_FFFF
    };

    for seg_idx in 0..seg_count {
        let is_chain = seg_idx > 0;
        let rel_start = keys.get(seg_idx).copied().unwrap_or(0);
        let rel_end = keys
            .get(seg_idx + 1)
            .copied()
            .unwrap_or_else(|| obj.frame_end.saturating_sub(obj.frame_start));
        let abs_start = obj.frame_start + rel_start;
        let abs_end = if seg_idx + 1 < seg_count {
            (obj.frame_start + rel_end).saturating_sub(1).max(abs_start)
        } else {
            obj.frame_end.max(abs_start)
        };

        let total_ext_size: usize = if is_chain {
            0
        } else {
            resolved_effects.iter().map(|e| e.4).sum()
        };

        let mut blob = vec![0u8; 0x5C8 + total_ext_size];
        blob[0..4].copy_from_slice(&obj_flag.to_le_bytes());
        let field0x4 = if scene_id == editing_scene {
            layer_id
        } else {
            0xFFFF_FFFF
        };
        blob[4..8].copy_from_slice(&field0x4.to_le_bytes());
        blob[8..12].copy_from_slice(&abs_start.to_le_bytes());
        blob[12..16].copy_from_slice(&abs_end.to_le_bytes());
        blob[0x10..0x50].copy_from_slice(&preview_sjis);
        blob[0x50..0x54].copy_from_slice(&chain_group.to_le_bytes());
        blob[0xF4..0xF8].copy_from_slice(&(total_ext_size as u32).to_le_bytes());
        blob[0x4B8..0x4BC].copy_from_slice(&0u32.to_le_bytes());
        blob[0x4BC..0x4C0].copy_from_slice(&obj.group.unwrap_or(0).to_le_bytes());
        blob[0x5C0..0x5C4].copy_from_slice(&layer_id.to_le_bytes());
        blob[0x5C4..0x5C8].copy_from_slice(&scene_id.to_le_bytes());

        let mut ext_cursor = 0usize;
        let mut trackbar_count = 0usize;
        let mut checkbox_count = 0usize;

        for (ei, (eff_id, _eff_type_flag, tb_num, cb_num, ext_size, eff_name, script_name, eff)) in
            resolved_effects.iter().enumerate()
        {
            let hdr_off = 0x54 + ei * 12;
            blob[hdr_off..hdr_off + 4].copy_from_slice(&(*eff_id as i32).to_le_bytes());
            blob[hdr_off + 4..hdr_off + 6].copy_from_slice(&(trackbar_count as u16).to_le_bytes());
            blob[hdr_off + 6..hdr_off + 8].copy_from_slice(&(checkbox_count as u16).to_le_bytes());
            blob[hdr_off + 8..hdr_off + 12].copy_from_slice(&(ext_cursor as i32).to_le_bytes());

            let mut e_flag: u8 = if eff.enabled { 1 } else { 0 };
            if matches!(*eff_id, 10 | 11 | 13 | 88 | 94 | 95 | 96) {
                e_flag |= 4; // EffectFlag.Interactive
            }
            blob[0xE4 + ei] = e_flag;

            let scale_fn = |tb_i: usize| get_trackbar_scale(eff_name, tb_i);
            for tb_i in 0..*tb_num {
                let idx = trackbar_count + tb_i;
                let scale = scale_fn(tb_i);
                let (cur, next, trans, param) = encode_trackbar_for_segment(
                    eff_name,
                    tb_i,
                    scale,
                    eff,
                    &keys,
                    seg_idx,
                    fps,
                    trackbar_scripts,
                );
                blob[0x0F8 + idx * 4..0x0F8 + idx * 4 + 4].copy_from_slice(&cur.to_le_bytes());
                blob[0x1F8 + idx * 4..0x1F8 + idx * 4 + 4].copy_from_slice(&next.to_le_bytes());
                blob[0x2F8 + idx * 4..0x2F8 + idx * 4 + 4].copy_from_slice(&trans.to_le_bytes());
                blob[0x4C0 + idx * 4..0x4C0 + idx * 4 + 4].copy_from_slice(&param.to_le_bytes());
            }

            for cb_i in 0..*cb_num {
                let idx = checkbox_count + cb_i;
                let cb_val = encode_checkbox_value(eff_name, cb_i, eff);
                blob[0x3F8 + idx * 4..0x3F8 + idx * 4 + 4].copy_from_slice(&cb_val.to_le_bytes());
            }

            if !is_chain && *ext_size > 0 {
                let ext_bytes = build_ext_data(eff_name, *ext_size, script_name.as_deref(), eff);
                blob[0x5C8 + ext_cursor..0x5C8 + ext_cursor + *ext_size]
                    .copy_from_slice(&ext_bytes[..*ext_size]);
                ext_cursor += *ext_size;
            }

            trackbar_count += *tb_num;
            checkbox_count += *cb_num;
        }

        blob[0xF0..0xF2].copy_from_slice(&(trackbar_count as u16).to_le_bytes());
        blob[0xF2..0xF4].copy_from_slice(&(checkbox_count as u16).to_le_bytes());
        for ei in resolved_effects.len()..12 {
            let hdr_off = 0x54 + ei * 12;
            blob[hdr_off..hdr_off + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        }

        obj_blobs.push(blob);
        *obj_counter += 1;
    }
}

fn get_default_trackbar_value(eff_name: &str, tb_i: usize) -> f64 {
    match (eff_name, tb_i) {
        ("標準描画" | "拡張描画", 3) => 100.0, // 拡大率
        ("グループ制御", 3) => 100.0,           // 拡大率
        ("動画ファイル" | "シーン", 0) => 1.0,  // 再生位置 (1フレーム目)
        ("動画ファイル" | "音声ファイル" | "シーン" | "シーン(音声)", 1) => 100.0, // 再生速度
        ("標準再生", 0) => 100.0,               // 音量
        ("図形", 0) => 100.0,                   // サイズ
        ("図形", 2) => 4000.0,                  // ライン幅
        ("テキスト", 0) => 34.0,                // サイズ
        ("拡大率" | "リサイズ", _) => 100.0,
        ("カメラ制御", 2) => -1024.0,           // Z
        ("カメラ制御", 9) => 30.0,              // 視野角
        ("グロー", 0) => 50.0,
        ("グロー", 1) => 60.0,
        ("グロー", 2) => 50.0,
        ("グロー", 3) => 20.0,
        ("発光", 0) => 100.0,
        ("発光", 1) => 75.0,
        ("発光", 2) => 75.0,
        ("ぼかし", 0) => 5.0,
        ("モザイク", 0) => 10.0,
        ("シャドー", 0) => 5.0,
        ("シャドー", 1) => 5.0,
        ("シャドー", 2) => 40.0,
        ("シャドー", 3) => 10.0,
        ("縁取り", 0) => 3.0,
        ("縁取り", 1) => 10.0,
        ("色調補正", 1 | 3 | 4) => 100.0,       // コントラスト・輝度・彩度
        ("グラデーション", 0) => 100.0,
        ("グラデーション", 4) => 100.0,
        ("単色化", 0) => 100.0,
        ("マスク", 3) => 100.0,
        _ => 0.0,
    }
}

fn find_track_param<'a>(
    eff_name: &str,
    tb_i: usize,
    eff: &'a au2_core::Effect,
) -> Option<&'a au2_core::ParamValue> {
    let primary_name = get_trackbar_name(eff_name, tb_i);
    let fallback_track = format!("track{}", tb_i);
    for p in &eff.params {
        if p.key == primary_name || p.key == fallback_track {
            return Some(&p.value);
        }
        if (eff_name == "標準描画" && tb_i == 5 || eff_name == "拡張描画" && tb_i == 8)
            && matches!(p.key.as_str(), "回転" | "Z軸回転")
        {
            return Some(&p.value);
        }
        if eff_name == "色調補正" && tb_i == 1 && matches!(p.key.as_str(), "ｺﾝﾄﾗｽﾄ" | "コントラスト") {
            return Some(&p.value);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn encode_trackbar_for_segment(
    eff_name: &str,
    tb_i: usize,
    scale: i32,
    eff: &au2_core::Effect,
    keys: &[au2_core::Frame],
    seg_idx: usize,
    fps: f64,
    trackbar_scripts: &mut Vec<String>,
) -> (i32, i32, u32, i32) {
    let scale_f = (scale.max(1)) as f64;
    let Some(pv) = find_track_param(eff_name, tb_i, eff) else {
        let def = (get_default_trackbar_value(eff_name, tb_i) * scale_f).round() as i32;
        return (def, def, 0, 0);
    };

    match pv {
        au2_core::ParamValue::Text(s) => {
            let v = s
                .split(',')
                .next()
                .and_then(|t| t.trim().parse::<f64>().ok())
                .unwrap_or_else(|| get_default_trackbar_value(eff_name, tb_i));
            let scaled = (v * scale_f).round() as i32;
            (scaled, scaled, 0, 0)
        }
        au2_core::ParamValue::Track(track) => {
            // AviUtl2 の「再生範囲」指定 (秒単位) が入っている場合の変換
            if tb_i == 0
                && matches!(eff_name, "動画ファイル" | "音声ファイル")
                && track
                    .interpolation
                    .as_ref()
                    .is_some_and(|it| matches!(&it.kind, au2_core::MoveKind::Script(s) if s == "再生範囲"))
            {
                let start_sec = track.start_value().max(0.0);
                let val = if eff_name == "動画ファイル" {
                    (start_sec * fps).round() as i32 + 1
                } else {
                    (start_sec * scale_f).round() as i32
                };
                return (val, val, 0, 0);
            }

            let Some(interp) = &track.interpolation else {
                let v = (track.start_value() * scale_f).round() as i32;
                return (v, v, 0, 0);
            };
            if track.keyframes.len() < 2 {
                let v = (track.start_value() * scale_f).round() as i32;
                return (v, v, 0, 0);
            }

            let val_at_idx = |k_idx: usize| -> f64 {
                if let Some(&f) = keys.get(k_idx) {
                    track
                        .keyframes
                        .iter()
                        .find(|k| k.frame == f)
                        .map(|k| k.value)
                        .unwrap_or_else(|| track.evaluate(f))
                } else {
                    track
                        .keyframes
                        .get(k_idx)
                        .or_else(|| track.keyframes.last())
                        .map(|k| k.value)
                        .unwrap_or(0.0)
                }
            };

            let cur = (val_at_idx(seg_idx) * scale_f).round() as i32;
            let next = (val_at_idx(seg_idx + 1) * scale_f).round() as i32;

            let mut easing_flags = 0u32;
            if interp.easing.accel() {
                easing_flags |= 0x40;
            }
            if interp.easing.decel() {
                easing_flags |= 0x20;
            }

            let mut get_or_insert_script = |name: &str| -> u32 {
                if let Some(pos) = trackbar_scripts.iter().position(|s| s == name) {
                    pos as u32
                } else {
                    let idx = trackbar_scripts.len() as u32;
                    trackbar_scripts.push(name.to_string());
                    idx
                }
            };

            let trans_code: u32 = match &interp.kind {
                au2_core::MoveKind::Linear => 1 | easing_flags,
                au2_core::MoveKind::Curve => {
                    if interp.legacy_code == Some(2) {
                        2 | easing_flags
                    } else {
                        let sc_idx = get_or_insert_script("補間移動");
                        0x0F | easing_flags | (sc_idx << 16)
                    }
                }
                au2_core::MoveKind::Instant => 3 | easing_flags,
                au2_core::MoveKind::IgnoreMidpoint => 4 | easing_flags,
                au2_core::MoveKind::MoveAmount => 5 | easing_flags,
                au2_core::MoveKind::Random => 6 | easing_flags,
                au2_core::MoveKind::Other(c) => (c & 0x0F) | easing_flags,
                au2_core::MoveKind::Script(s) => {
                    let sc_idx = get_or_insert_script(s);
                    0x0F | easing_flags | (sc_idx << 16)
                }
            };

            let param = interp
                .param
                .as_deref()
                .and_then(|p| p.trim().parse::<i32>().ok())
                .unwrap_or(0);

            (cur, next, trans_code, param)
        }
    }
}

fn encode_checkbox_value(eff_name: &str, cb_i: usize, eff: &au2_core::Effect) -> i32 {
    let cb_name = get_checkbox_name(eff_name, cb_i);
    let fallback_key = format!("check{}", cb_i);
    for p in &eff.params {
        if cb_name.as_deref() == Some(p.key.as_str()) || p.key == fallback_key {
            return match &p.value {
                au2_core::ParamValue::Text(s) => s.trim().parse::<i32>().unwrap_or(0),
                au2_core::ParamValue::Track(t) => t.start_value().round() as i32,
            };
        }
    }
    0
}

fn parse_rgb_hex(s: &str, default_rgb: [u8; 3]) -> [u8; 3] {
    let trimmed = s.trim().trim_start_matches('#');
    if trimmed.len() >= 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&trimmed[0..2], 16),
            u8::from_str_radix(&trimmed[2..4], 16),
            u8::from_str_radix(&trimmed[4..6], 16),
        ) {
            return [r, g, b];
        }
    }
    default_rgb
}

fn get_param_i32(eff: &au2_core::Effect, key: &str, default_val: i32) -> i32 {
    eff.params
        .iter()
        .find(|p| p.key == key)
        .map(|p| match &p.value {
            au2_core::ParamValue::Text(s) => s.trim().parse::<i32>().unwrap_or(default_val),
            au2_core::ParamValue::Track(t) => t.start_value().round() as i32,
        })
        .unwrap_or(default_val)
}

fn build_ext_data(
    eff_name: &str,
    ext_size: usize,
    implicit_script_name: Option<&str>,
    eff: &au2_core::Effect,
) -> Vec<u8> {
    let mut ext = vec![0u8; ext_size];
    if ext_size == 0 {
        return ext;
    }

    match eff_name {
        "動画ファイル" | "音声ファイル" => {
            if let Some(path) = eff.text("file").or_else(|| eff.text("ファイル")) {
                let sjis = encode_sjis_fixed(path, 260.min(ext_size));
                ext[..sjis.len()].copy_from_slice(&sjis);
            }
        }
        "画像ファイル" => {
            if let Some(path) = eff.text("file").or_else(|| eff.text("ファイル")) {
                if ext_size >= 264 {
                    let sjis = encode_sjis_fixed(path, 260);
                    ext[4..264].copy_from_slice(&sjis);
                }
            }
        }
        "図形" => {
            if ext_size >= 8 {
                let raw_type = eff
                    .text("type")
                    .or_else(|| eff.text("図形の種類"))
                    .unwrap_or("2");
                let fig_type: i32 = match raw_type.trim() {
                    "背景" | "0" => 0,
                    "円" | "1" => 1,
                    "四角形" | "2" => 2,
                    "三角形" | "3" => 3,
                    "五角形" | "4" => 4,
                    "六角形" | "5" => 5,
                    "星型" | "6" => 6,
                    other => other.parse().unwrap_or(2),
                };
                ext[0..4].copy_from_slice(&fig_type.to_le_bytes());
                let rgb = parse_rgb_hex(
                    eff.text("color").or_else(|| eff.text("色")).unwrap_or("ffffff"),
                    [0xFF, 0xFF, 0xFF],
                );
                ext[4..7].copy_from_slice(&rgb);
                if ext_size >= 72 {
                    if let Some(name) = eff.text("name") {
                        let sjis = encode_sjis_fixed(name, 64);
                        ext[8..72].copy_from_slice(&sjis);
                    }
                }
            }
        }
        "テキスト" => {
            if ext_size >= 0x30 + 2048 {
                let raw_type = eff
                    .text("type")
                    .or_else(|| eff.text("文字装飾"))
                    .unwrap_or("0");
                let t_type: u8 = match raw_type.trim() {
                    "標準文字" | "0" => 0,
                    "影付き文字" | "1" => 1,
                    "影付き文字(薄)" | "2" => 2,
                    "縁取り文字" | "3" => 3,
                    "縁取り文字(細)" | "4" => 4,
                    "縁取り文字(太)" | "5" => 3,
                    "縁取り文字(角)" | "6" => 4,
                    other => other.parse().unwrap_or(0),
                };
                let autoadjust = get_param_i32(eff, "autoadjust", 0) as u8;
                let soft = get_param_i32(eff, "soft", 1) as u8;
                let monospace = get_param_i32(eff, "monospace", 0) as u8;
                let raw_align = eff
                    .text("align")
                    .or_else(|| eff.text("文字揃え"))
                    .unwrap_or("0");
                let align: u8 = match raw_align.trim() {
                    "左寄せ[上]" | "0" => 0,
                    "中央揃え[上]" | "1" => 1,
                    "右寄せ[上]" | "2" => 2,
                    "左寄せ[中]" | "3" => 3,
                    "中央揃え[中]" | "4" => 4,
                    "右寄せ[中]" | "5" => 5,
                    "左寄せ[下]" | "6" => 6,
                    "中央揃え[下]" | "7" => 7,
                    "右寄せ[下]" | "8" => 8,
                    other => other.parse().unwrap_or(0),
                };
                let spacing_x = get_param_i32(eff, "spacing_x", get_param_i32(eff, "字間", 0)) as i8 as u8;
                let spacing_y = get_param_i32(eff, "spacing_y", get_param_i32(eff, "行間", 0)) as i8 as u8;
                let precision = get_param_i32(eff, "precision", 1) as u8;

                ext[0] = t_type;
                ext[1] = autoadjust;
                ext[2] = soft;
                ext[3] = monospace;
                ext[4] = align;
                ext[5] = spacing_x;
                ext[6] = spacing_y;
                ext[7] = precision;

                let rgb1 = parse_rgb_hex(
                    eff.text("color").or_else(|| eff.text("文字色")).unwrap_or("ffffff"),
                    [0xFF, 0xFF, 0xFF],
                );
                let rgb2 = parse_rgb_hex(
                    eff.text("color2").or_else(|| eff.text("影・縁色")).unwrap_or("000000"),
                    [0x00, 0x00, 0x00],
                );
                ext[8..11].copy_from_slice(&rgb1);
                ext[12..15].copy_from_slice(&rgb2);

                let font = eff
                    .text("font")
                    .or_else(|| eff.text("フォント"))
                    .unwrap_or("MS UI Gothic");
                let font_sjis = encode_sjis_fixed(font, 32);
                ext[0x10..0x30].copy_from_slice(&font_sjis);

                let raw_text = eff
                    .text("text")
                    .or_else(|| eff.text("テキスト"))
                    .unwrap_or("");
                let decoded_text = if raw_text.len() >= 16
                    && raw_text.len() % 4 == 0
                    && raw_text.chars().all(|c| c.is_ascii_hexdigit())
                {
                    au2_exo::decode_text_hex(raw_text).unwrap_or_else(|| raw_text.to_string())
                } else {
                    raw_text.to_string()
                };
                let mut utf16_pos = 0x30usize;
                for code_unit in decoded_text.encode_utf16() {
                    if utf16_pos + 4 > 0x30 + 2048 {
                        break;
                    }
                    ext[utf16_pos..utf16_pos + 2].copy_from_slice(&code_unit.to_le_bytes());
                    utf16_pos += 2;
                }
            }
        }
        "標準描画" | "拡張描画" | "パーティクル出力" => {
            if ext_size >= 4 {
                let raw_blend = eff
                    .text("blend")
                    .or_else(|| eff.text("合成モード"))
                    .unwrap_or("0");
                let blend: i32 = crate::blend_aup2_to_code(raw_blend).parse().unwrap_or(0);
                ext[0..4].copy_from_slice(&blend.to_le_bytes());
            }
        }
        "シーン" | "シーン(音声)" => {
            if ext_size >= 4 {
                let sc_idx = get_param_i32(eff, "scene", 1);
                ext[0..4].copy_from_slice(&sc_idx.to_le_bytes());
            }
        }
        "時間制御" | "グループ制御" | "カメラ制御" => {
            if ext_size >= 4 {
                let range = get_param_i32(
                    eff,
                    "range",
                    get_param_i32(eff, "対象レイヤー数", 0),
                );
                ext[0..4].copy_from_slice(&range.to_le_bytes());
            }
        }
        "グロー" | "発光" | "拡散光" | "グロー(フィルタ)" | "発光(フィルタ)" | "拡散光(フィルタ)" => {
            if ext_size >= 4 {
                let color_opt = eff.text("color").or_else(|| eff.text("光色"));
                let no_color = get_param_i32(
                    eff,
                    "no_color",
                    if color_opt.is_some_and(|c| c.is_empty()) { 1 } else { 0 },
                ) as u8;
                let rgb = parse_rgb_hex(color_opt.unwrap_or("ffffff"), [0xFF, 0xFF, 0xFF]);
                ext[0..3].copy_from_slice(&rgb);
                ext[3] = no_color;
            }
            if ext_size >= 8 {
                let t = get_param_i32(eff, "type", 0);
                ext[4..8].copy_from_slice(&t.to_le_bytes());
            }
            if ext_size >= 12 {
                let blend = get_param_i32(eff, "blend", 0);
                ext[8..12].copy_from_slice(&blend.to_le_bytes());
            }
        }
        "閃光" | "閃光(フィルタ)" | "ライト" | "ライト(フィルタ)" => {
            if ext_size >= 4 {
                let rgb = parse_rgb_hex(
                    eff.text("color").or_else(|| eff.text("光色")).unwrap_or("ffffff"),
                    [0xFF, 0xFF, 0xFF],
                );
                ext[0..3].copy_from_slice(&rgb);
            }
            if ext_size >= 8 {
                let blend = get_param_i32(eff, "blend", 0);
                ext[4..8].copy_from_slice(&blend.to_le_bytes());
            }
        }
        "シャドー" | "縁取り" | "単色化" | "単色化(フィルタ)" => {
            if ext_size >= 4 {
                let rgb = parse_rgb_hex(
                    eff.text("color")
                        .or_else(|| eff.text("影色"))
                        .or_else(|| eff.text("縁色"))
                        .unwrap_or("000000"),
                    [0x00, 0x00, 0x00],
                );
                ext[0..3].copy_from_slice(&rgb);
            }
            if ext_size >= 264 {
                if let Some(path) = eff.text("file").or_else(|| eff.text("ファイル")) {
                    let sjis = encode_sjis_fixed(path, 260);
                    ext[4..264].copy_from_slice(&sjis);
                }
            }
        }
        "グラデーション" | "グラデーション(フィルタ)" => {
            if ext_size >= 16 {
                let blend = get_param_i32(eff, "blend", 0);
                ext[0..4].copy_from_slice(&blend.to_le_bytes());
                let rgb1 = parse_rgb_hex(eff.text("color").unwrap_or("ffffff"), [0xFF, 0xFF, 0xFF]);
                ext[4..7].copy_from_slice(&rgb1);
                ext[7] = get_param_i32(eff, "no_color", 0) as u8;
                let rgb2 = parse_rgb_hex(eff.text("color2").unwrap_or("000000"), [0x00, 0x00, 0x00]);
                ext[8..11].copy_from_slice(&rgb2);
                ext[11] = get_param_i32(eff, "no_color2", 0) as u8;
                let g_type = get_param_i32(eff, "type", 0);
                ext[12..16].copy_from_slice(&g_type.to_le_bytes());
            }
        }
        "ワイプ" | "マスク" | "ワイプ(フィルタ)" | "ディスプレイスメントマップ" => {
            if ext_size >= 4 {
                let def_t = if eff_name == "ワイプ" || eff_name == "ワイプ(フィルタ)" {
                    0
                } else {
                    1
                };
                let t = get_param_i32(eff, "type", def_t) as i16;
                ext[0..2].copy_from_slice(&t.to_le_bytes());
                if ext_size >= 260 {
                    if let Some(name) = eff.text("name") {
                        let sjis = encode_sjis_fixed(name, 256);
                        ext[4..260].copy_from_slice(&sjis);
                    }
                }
                if ext_size >= 264 {
                    let mode = get_param_i32(eff, "mode", 0);
                    ext[260..264].copy_from_slice(&mode.to_le_bytes());
                }
            }
        }
        "動画ファイル合成" | "画像ファイル合成" => {
            if ext_size >= 4 {
                let mode = get_param_i32(eff, "mode", 0);
                ext[0..4].copy_from_slice(&mode.to_le_bytes());
            }
            if ext_size >= 264 {
                if let Some(path) = eff.text("file").or_else(|| eff.text("ファイル")) {
                    let sjis = encode_sjis_fixed(path, 260);
                    ext[4..264].copy_from_slice(&sjis);
                }
            }
        }
        "音声波形表示" => {
            if ext_size >= 14 {
                let t = get_param_i32(eff, "type", 0) as i16;
                ext[0..2].copy_from_slice(&t.to_le_bytes());
                let rgb = parse_rgb_hex(eff.text("color").unwrap_or("ffffff"), [0xFF, 0xFF, 0xFF]);
                ext[4..7].copy_from_slice(&rgb);
                let sample_n = get_param_i32(eff, "sample_n", 1024) as i16;
                let res_w = get_param_i32(eff, "res_w", 100) as i16;
                let res_h = get_param_i32(eff, "res_h", 100) as i16;
                ext[8..10].copy_from_slice(&sample_n.to_le_bytes());
                ext[10..12].copy_from_slice(&res_w.to_le_bytes());
                ext[12..14].copy_from_slice(&res_h.to_le_bytes());
            }
        }
        "アニメーション効果"
        | "カスタムオブジェクト"
        | "カメラ効果"
        | "スクリプト(カメラ制御)"
        | "シーンチェンジ" => {
            if ext_size >= 516 {
                let raw_name = implicit_script_name
                    .or_else(|| eff.text("name"))
                    .unwrap_or("");
                let explicit_type = eff
                    .text("type")
                    .and_then(|s| s.trim().parse::<i16>().ok());
                let dir = get_param_i32(eff, "filter", 0) as i16;

                let builtin_table: &[&str] = match eff_name {
                    "アニメーション効果" => BUILTIN_ANM_NAMES,
                    "カスタムオブジェクト" => BUILTIN_OBJ_NAMES,
                    "カメラ効果" | "スクリプト(カメラ制御)" => BUILTIN_CAM_NAMES,
                    "シーンチェンジ" => BUILTIN_SCN_TYPES,
                    _ => &[],
                };

                let (script_id, stored_name) = if let Some(pos) =
                    builtin_table.iter().position(|&b| b == raw_name)
                {
                    (pos as i16, "")
                } else if eff_name == "シーンチェンジ" {
                    if let Some(pos) = BUILTIN_SCN_NAMES.iter().position(|&b| b == raw_name) {
                        (pos as i16, "")
                    } else {
                        (explicit_type.unwrap_or(0), raw_name)
                    }
                } else if let Some(t) = explicit_type {
                    if raw_name.is_empty() || builtin_table.get(t as usize).copied() == Some(raw_name) {
                        (t, "")
                    } else {
                        (t, raw_name)
                    }
                } else {
                    (0, raw_name)
                };

                ext[0..2].copy_from_slice(&script_id.to_le_bytes());
                ext[2..4].copy_from_slice(&dir.to_le_bytes());
                if !stored_name.is_empty() {
                    let sjis = encode_sjis_fixed(stored_name, 256);
                    ext[4..260].copy_from_slice(&sjis);
                }
                if let Some(param_str) = eff.text("param") {
                    let sjis = encode_sjis_fixed(param_str, 256);
                    ext[260..516].copy_from_slice(&sjis);
                }
            }
        }
        _ => {
            if ext_size >= 4 {
                if let Some(c) = eff.text("color") {
                    let rgb = parse_rgb_hex(c, [0xFF, 0xFF, 0xFF]);
                    ext[0..3].copy_from_slice(&rgb);
                } else {
                    let t = get_param_i32(eff, "type", 0);
                    ext[0..4].copy_from_slice(&t.to_le_bytes());
                }
            }
        }
    }

    ext
}
