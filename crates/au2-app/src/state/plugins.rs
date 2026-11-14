//! プラグインインストール・AUFポーター（.auf / .auf2 変換）処理。

use super::EditorState;

impl EditorState {
    /// プラグイン・スクリプトを複数選択して一括インストールするダイアログ。
    pub fn install_plugin_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("プラグイン・スクリプトをインストール (初代AviUtl 32bit / AviUtl2 64bit 両対応)")
            .add_filter(
                "初代AviUtl / AviUtl2 スクリプト・プラグイン",
                &[
                    "anm", "obj", "cam", "scn", "tra", "auf", "aui", "auo", "auc", "anm2", "obj2",
                    "cam2", "scn2", "tra2", "auf2", "aui2", "auo2", "auc2", "mod2", "aux2",
                ],
            )
            .add_filter("すべてのファイル", &["*"])
            .pick_files()
        {
            let mut installed = Vec::new();
            for path in &paths {
                if let Ok(name) = self.plugins.install_file(path) {
                    installed.push(name);
                }
            }
            if !installed.is_empty() {
                self.status = format!(
                    "プラグイン {} 件 ({}) を plugins/ にインストールしました",
                    installed.len(),
                    installed.join(", ")
                );
            }
        }
    }

    /// `.auf` / `.auf2` 自動解析＆クロスプラットフォーム移植・変換スタジオを開きます。
    pub fn open_auf_porter(&mut self, plugin_idx: Option<usize>) {
        if let Some(idx) = plugin_idx {
            self.auf_porter.selected_plugin_idx = idx;
        } else if let Some((idx, _)) = self
            .plugins
            .plugins
            .iter()
            .enumerate()
            .find(|(_, p)| matches!(p.kind, au2_plugin::PluginKind::FilterDll))
        {
            self.auf_porter.selected_plugin_idx = idx;
        }
        self.auf_porter.is_open = true;
    }

    /// 指定した `.auf` / `.auf2` プラグインから `.anm` 代替スクリプト雛形を生成して `plugins/` に登録します。
    pub fn convert_auf_to_anm(&mut self, plugin_idx: usize) {
        match self.plugins.convert_plugin_to_anm(plugin_idx) {
            Ok(out_path) => {
                let fname = out_path
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.status = format!(
                    "パラメータ定義から代替 `.anm` スクリプト雛形 `{fname}` を生成して登録しました"
                );
            }
            Err(e) => {
                self.status = format!("⚠ .anm 雛形の生成に失敗しました: {e}");
            }
        }
    }

    /// 登録済みの全 `.auf` / `.auf2` プラグインから一括で `.anm` 代替スクリプト雛形を生成します。
    pub fn convert_all_auf_to_anm(&mut self) {
        match self.plugins.convert_all_auf_to_anm() {
            Ok(paths) => {
                self.status = format!(
                    "{} 件の .auf / .auf2 から代替 .anm スクリプト雛形を一括生成しました",
                    paths.len()
                );
            }
            Err(e) => {
                self.status = format!("⚠ 一括生成に失敗しました: {e}");
            }
        }
    }
}
