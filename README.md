# OpenAvi

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/openavi/openavi/actions/workflows/ci.yml/badge.svg)](https://github.com/openavi/openavi/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/Rust-2021_Edition-orange.svg)](https://www.rust-lang.org/)

**OpenAvi** は、既存の「AviUtl（および拡張編集プラグイン）」が持つ軽快な操作性と豊富なユーザーメイド・エコシステムを引き継ぎつつ、現代のハードウェア環境に最適化した **64bit ネイティブ・オープンソース動画編集ソフトウェア** です。

---

## 主な特徴

- **完全 64bit 化 & マルチスレッド設計**: 従来の 4GB メモリ制限を撤廃し、Rust によるメモリ安全性とマルチコア CPU 性能を活かしたコア設計。
- **GPU アクセラレーション (`wgpu`)**: DirectX 12 / Vulkan / Metal バックエンド上で動作する低遅延な UI・プレビュー基盤。
- **画像・動画・音声ファイルの読み込み & プレビュー描画 (`au2-media`)**:
  - **画像ファイル**: `.png`, `.jpg`, `.jpeg`, `.bmp`, `.webp`, `.gif`（アニメーション GIF 含む）のネイティブデコード＆ GPU テクスチャ描画。
  - **動画ファイル**: `.mp4`, `.avi`, `.mov`, `.mkv`, `.webm`, `.wmv` 等のフレームデコード＆キャッシュ描画（再生位置・再生速度・X/Y 座標・拡大率・回転・透明度・フェード反映）。
  - ウィンドウへのドラッグ＆ドロップ、またはタイムラインの右クリックメニューから直接配置可能。
- **`.aup` (AviUtl プロジェクトファイル) & `.exo` (拡張編集オブジェクトファイル) 対応**:
  - `.aup` バイナリプロジェクトの読み込み・保存（`AviUtl ProjectFile version 0.18` ヘッダ検証、OpenAvi 拡張編集チャンク保存、レガシー `.aup` からのメディア参照抽出）。
  - `.exo` の Shift_JIS (CP932) / UTF-8 自動判別、`text=` (UTF-16LE 16進ダンプ) 変換、`chain=1` 中間点オブジェクトの統合・再出力（ラウンドトリップ保証）。
- **AviUtl 流のタイムライン操作 & 右クリックメニュー**:
  - **レイヤー右クリックメニュー**:
    - メディアオブジェクトの追加（動画・画像・音声・テキスト・図形・フレームバッファ・グループ制御・カメラ制御）
    - フィルタオブジェクトの追加（色調補正）
    - オブジェクト操作（**中間点を追加**、**ここで分割**、**削除**）
    - レイヤー表示 / 非表示・ロック切替
  - モダンな **シングルウィンドウ（ドッキング）モード** と、AviUtl 伝統の **マルチウィンドウ（分離）モード** をメニューから即座に切り替え可能。
- **プラグイン・スクリプト管理機能 (`au2-plugin`)**:
  - `plugins/` フォルダ内の `.anm`, `.obj`, `.cam`, `.scn`, `.tra` スクリプトおよび `.auf`, `.aui`, `.auo`, `.auc` プラグインを自動スキャン。

---

## ディレクトリ構成

```text
openavi/
├── .github/workflows/ci.yml  # GitHub Actions (Windows / Linux / macOS 自動ビルド・テスト)
├── Cargo.toml                # Cargo Workspace 定義
├── LICENSE                   # MIT License
├── CONTRIBUTING.md           # コントリビューションガイド
├── plugins/
│   └── openavi_effects.anm   # 同梱アニメーション効果スクリプト (@揺れ, @点滅, @回転)
├── samples/
│   ├── sample.exo            # 動作確認用 Shift_JIS サンプルオブジェクトファイル
│   ├── sample_image.png      # 動作確認用サンプル画像
│   └── sample_video.mp4      # 動作確認用サンプル動画
└── crates/
    ├── au2-core/             # コアデータモデル (Project / Scene / Layer / Object / Track / Keyframe)
    ├── au2-aup/              # AviUtl プロジェクトファイル (.aup) バイナリパーサー & ライター
    ├── au2-exo/              # 拡張編集オブジェクトファイル (.exo) パーサー & シリアライザ
    ├── au2-media/            # 画像・動画デコーダ & フレームキャッシュ
    ├── au2-plugin/           # スクリプト (.anm/.obj等) & DLLプラグイン (.auf/.aui等) ローダー・検査器
    └── au2-app/              # デスクトップ GUI アプリケーション (バイナリ名: openavi)
```

---

## ダウンロード＆ワンクリック起動（一般ユーザー向け）

1. GitHub の **Releases** ページから最新の **`OpenAvi-windows-x64.zip`** をダウンロードして解凍します。
2. 解凍したフォルダ内の **`install.bat`** をダブルクリックすると：
   - `%LOCALAPPDATA%\Programs\OpenAvi` にアプリ本体・サンプル・プラグインがインストールされます。
   - **デスクトップ** と **スタートメニュー** に **「OpenAvi」アイコン** が自動作成されます。
3. 以降はデスクトップの **「OpenAvi」アイコンをワンクリック（ダブルクリック）するだけ** で即座に起動できます（ポータブル版としてフォルダ内の `openavi.exe` または `Launch-OpenAvi.bat` を直接クリックして起動することも可能です）。

---

## 開発者向けビルド・パッケージ作成方法

```bash
# 全クレートの単体・結合テスト実行
cargo test --workspace

# OpenAvi の起動
cargo run --release -p au2-app

# 配布用 ZIP パッケージ (dist/OpenAvi-windows-x64.zip) の生成 & デスクトップショートカット作成
powershell -ExecutionPolicy Bypass -File .\package-release.ps1
```

---

## ライセンス

本プロジェクトは [MIT License](LICENSE) の下で公開されています。
