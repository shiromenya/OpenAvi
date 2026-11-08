//! キーボードショートカット一覧モーダルダイアログ。

use eframe::egui::{self, Grid, RichText, ScrollArea, Window};

use crate::app::OpenAviApp;

impl OpenAviApp {
    /// キーボードショートカット一覧ウィンドウを描画します。
    pub fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }

        Window::new("⌨ キーボードショートカット一覧 (F1)")
            .frame(egui::Frame::window(&ctx.style()).fill(crate::theme::surface()))
            .open(&mut self.show_shortcuts)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .default_height(420.0)
            .show(ctx, |ui| {
                ui.heading("キーボードショートカット一覧 (AviUtl準拠)");
                ui.label("頻繁に使用する主要なキー操作の一覧です（F1キーで開閉可能）。");
                ui.add_space(8.0);
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let render_section = |ui: &mut egui::Ui, title: &str, items: &[(&str, &str)]| {
                        ui.label(RichText::new(title).strong().color(crate::theme::accent_text()));
                        Grid::new(title).num_columns(2).striped(true).spacing([24.0, 6.0]).show(ui, |ui| {
                            for (key, desc) in items {
                                ui.label(RichText::new(*key).strong().monospace());
                                ui.label(*desc);
                                ui.end_row();
                            }
                        });
                        ui.add_space(10.0);
                    };

                    render_section(ui, "▶ 再生・タイムライン移動", &[
                        ("Space", "再生 / 一時停止"),
                        ("← / →", "1 フレーム戻る / 進む"),
                        ("Shift + ← / →", "10 フレーム戻る / 進む"),
                        ("Home / End", "先頭フレーム / 末尾フレームへ移動"),
                        ("Ctrl + ホイール", "タイムラインの拡大 / 縮小"),
                        ("Shift + ホイール", "タイムラインの横スクロール"),
                    ]);

                    render_section(ui, "✂ 編集・オブジェクト操作", &[
                        ("S", "現在フレーム位置でオブジェクトを分割 (Split)"),
                        ("D", "中間点を追加（タイムラインの選択位置）"),
                        ("Delete", "選択中オブジェクトを削除"),
                        ("Ctrl + D", "選択中オブジェクトを複製"),
                        ("🧲 (Sキー)", "スナップ（吸着）の ON / OFF 切り替え"),
                    ]);

                    render_section(ui, "📋 選択・履歴・クリップボード", &[
                        ("Ctrl + Z", "元に戻す (Undo)"),
                        ("Ctrl + Y / Ctrl + Shift + Z", "やり直し (Redo)"),
                        ("Ctrl + A", "タイムライン上の全オブジェクトを選択"),
                        ("Ctrl + C", "選択中オブジェクトをコピー"),
                        ("Ctrl + V", "クリップボードから貼り付け"),
                        ("Escape", "選択解除"),
                    ]);

                    render_section(ui, "📁 ファイル・プロジェクト", &[
                        ("Ctrl + N", "新規プロジェクト作成"),
                        ("Ctrl + O", "プロジェクト / メディアを開く"),
                        ("Ctrl + S", "プロジェクトを上書き保存"),
                        ("Ctrl + Shift + S", "プロジェクトに名前を付けて保存"),
                        ("Ctrl + I", "メディアファイルを追加"),
                        ("F1", "このショートカット一覧を表示"),
                    ]);
                });
            });
    }
}
