# OpenAvi

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2021_Edition-orange.svg)](https://www.rust-lang.org/)

**OpenAvi** は、既存の「AviUtl」および「AviUtl2（拡張編集）」が持つ軽快な操作感と豊富なユーザーメイド・エコシステムを引き継ぎつつ、現代のハードウェア環境に最適化した **64bit ネイティブ・オープンソース動画編集ソフトウェア** です。

---

## 主な特徴

- **完全 64bit 化 & マルチコア最適化**:
  - 初代 32bit AviUtl の 4GB メモリ制限を撤廃。Rust によるメモリ安全性とマルチスレッド設計により、高解像度編集でも安定して動作します。
- **GPU アクセラレーション (`wgpu`)**:
  - DirectX 12 / Vulkan / Metal バックエンド上で動作する低遅延な UI・プレビュー基盤。
- **画像・動画・音声ファイルのネイティブ読み込み & プレビュー描画 (`au2-media`)**:
  - **画像**: `.png`, `.jpg`, `.jpeg`, `.bmp`, `.webp`, `.gif`（アニメーション GIF 含む）
  - **動画**: `.mp4`, `.avi`, `.mov`, `.mkv`, `.webm`, `.wmv` 等
  - **音声**: `.wav`, `.mp3`, `.ogg`, `.flac`
  - 入力プラグインの追加設定不要で、ドラッグ＆ドロップまたはタイムライン配置で即座に高速デコード・描画されます。
- **初代 AviUtl (`.aup`) & AviUtl2 (`.aup2`) & 拡張編集 (`.exo`) の完全互換**:
  - **`.aup2`**: AviUtl2 新プロジェクト形式の読み込み・保存に完全対応。
  - **`.aup`**: 初代 AviUtl 0.18 バイナリプロジェクトの相互読み込み・保存。
  - **`.exo`**: 拡張編集オブジェクトファイルの Shift_JIS (CP932) / UTF-8 自動判別、中間点・トラックバー・エフェクトのラウンドトリップ保存。
- **初代 32-bit / AviUtl2 64-bit プラグイン・スクリプト両対応 (`au2-plugin`)**:
  - **拡張編集スクリプト**: `.anm`, `.obj`, `.cam`, `.scn`, `.tra`（および `.anm2`, `.obj2`, `.cam2`, `.scn2`, `.tra2`）の Lua 5.1 / ExEdit スクリプト実行エンジンを内蔵。`obj.ox`, `obj.draw()`, `obj.load()`, `obj.mes()`, `obj.effect()` 等の演出をそのままプレビューに反映。
  - **ネイティブ DLL プラグイン**: 32-bit x86 および 64-bit x64 の `.auf`, `.aui`, `.auo`, `.auc` の PE バイナリ解析ブリッジを搭載。`FILTER_DLL` 構造体からトラックバー・チェックボックスを抽出し、タイムライン・設定ダイアログ・プレビュー描画で動作します。
- **AviUtl 流の操作感 & モダンな UI**:
  - レイヤー右クリックからのオブジェクト追加（テキスト、図形、フレームバッファ、色調補正、グループ制御、カメラ制御、カスタムオブジェクト）。
  - 中間点追加・分割・削除、Shift+ホイール横スクロール、オートスクラブ。
  - 未保存の変更がある状態でソフトを終了しようとした際の**保存確認ダイアログ**。
  - シングルウィンドウ（ドッキング）と、AviUtl 伝統のマルチウィンドウ（分離）の即時切替。

---

## ディレクトリ構成

```text
openavi/
├── Cargo.toml                # Cargo Workspace 定義
├── LICENSE                   # MIT License
├── CONTRIBUTING.md           # コントリビューションガイド
├── package-release.ps1       # リリースビルド & パッケージングスクリプト
├── assets/
│   └── openavi.ico           # アプリアイコン
├── plugins/                  # プラグイン・スクリプト配置フォルダ
│   ├── openavi_effects.anm   # サンプルアニメーション効果スクリプト (揺れ, 点滅, 回転, バウンス, 円形複製)
│   ├── openavi_custom.obj    # サンプルカスタムオブジェクトスクリプト (リングパーティクル, デジタルタイマー等)
│   ├── openavi_transitions.scn # サンプルシーンチェンジスクリプト (クロスフェード, 回転ズームアウト)
│   └── openavi_camera.cam    # サンプルカメラ効果スクリプト (手ブレカメラ)
├── samples/
│   ├── sample.exo            # 動作確認用 Shift_JIS サンプルオブジェクトファイル
│   ├── sample_image.png      # 動作確認用サンプル画像
│   └── sample_video.mp4      # 動作確認用サンプル動画
└── crates/
    ├── au2-core/             # コアデータモデル (Project / Scene / Layer / Object / Track / Keyframe)
    ├── au2-aup/              # AviUtl / AviUtl2 プロジェクトファイル (.aup / .aup2) パーサー & ライター
    ├── au2-exo/              # 拡張編集オブジェクトファイル (.exo) パーサー & シリアライザ
    ├── au2-media/            # 画像・動画・音声デコーダ & GPU フレームキャッシュ
    ├── au2-plugin/           # Lua スクリプト実行エンジン & 32bit/64bit PE プラグインブリッジ
    └── au2-app/              # デスクトップ GUI アプリケーション本体 (openavi)
```

---

## プラグインのご利用と再配布に関する方針

> [!IMPORTANT]
> **他者製プラグインの無断再配布は行いません**
> OpenAvi は、各プラグイン・スクリプト開発者様の著作権および利用規約を尊重するため、L-SMASH Works、patch.aul、x264guiEx 等を含む**他者製のプラグイン・バイナリ・スクリプトを本体に同梱・再配布していません**。
>
> 初代 AviUtl（32-bit）および AviUtl2（64-bit）向けに公開されている各種プラグインやスクリプトをご利用になる場合は、**各開発者様の正規配布ページより入手いただき、本ソフトの `plugins/` フォルダに配置するか、ウィンドウ上にドラッグ＆ドロップしてインストール**してください。OpenAvi のハイブリッド互換エンジンを通じて自動的に認識・実行されます。

---

## ダウンロード＆ワンクリック起動（一般ユーザー向け）

1. 配布パッケージ **`OpenAvi-windows-x64.zip`** を解凍します。
2. フォルダ内の **`install.bat`** を実行すると：
   - `%LOCALAPPDATA%\Programs\OpenAvi` にアプリ本体がインストールされます。
   - **デスクトップ** と **スタートメニュー** に **「OpenAvi」ショートカット** が自動作成されます。
3. デスクトップの **「OpenAvi」アイコンをダブルクリックするだけ** で起動できます（フォルダ内の `openavi.exe` または `Launch-OpenAvi.bat` から直接ポータブル起動することも可能です）。

---

## 開発者向けビルド・テスト手順

```bash
# 全クレートの単体・結合テスト実行
cargo test --workspace

# OpenAvi の開発起動
cargo run --release -p au2-app

# 配布用 ZIP パッケージ (dist/OpenAvi-windows-x64.zip) の生成
powershell -ExecutionPolicy Bypass -File .\package-release.ps1 -SkipBuild
```

---

## ライセンス

本ソフトウェアは [MIT License](LICENSE) の下で公開されています。
