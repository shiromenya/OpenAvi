<p align="center">
  <img src="assets/openavi_logo.png" alt="OpenAvi Logo" width="480">
</p>

# OpenAvi

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2021_Edition-orange.svg)](https://www.rust-lang.org/)

**OpenAvi** は、既存の「AviUtl」および「AviUtl2（拡張編集）」のワークフローを参考に、Rust と `egui` / `wgpu` で構築している **64bit オープンソース動画編集ソフトウェア** です。

---

## 主な特徴

- **64bit ネイティブ設計**:
  - Rust によるメモリ安全性とマルチスレッド構成で動作するデスクトップ動画編集環境です。
- **GPU アクセラレーション (`wgpu`)**:
  - DirectX 12 / Vulkan / Metal バックエンド上で動作する UI・プレビュー基盤。
- **画像・動画・音声ファイルの読み込み & プレビュー描画 (`au2-media`)**:
  - **画像**: `.png`, `.jpg`, `.jpeg`, `.bmp`, `.webp`, `.gif`（アニメーション GIF 含む）, `.psd` / `.psb`（レイヤー・フォルダ分解対応）
  - **動画**: `.mp4`, `.avi`, `.mov`, `.mkv`, `.webm`, `.wmv` 等（FFmpeg 連携）
  - **音声**: `.wav`, `.mp3`, `.ogg`, `.flac`
  - ドラッグ＆ドロップまたはタイムライン配置でデコード・描画されます。
- **プロジェクト・オブジェクト形式の読み書き (`au2-aup` / `au2-exo`)**:
  - **`.aup2`**: AviUtl2 形式プロジェクトの基本構造（シーン・レイヤー・オブジェクト・パラメータ）の読み込み・保存。
  - **`.aup`**: 初代 AviUtl のバイナリプロジェクト（拡張編集 0.90〜0.92、複数シーン・レイヤー名/ロック/非表示/座標のリンク/クリッピング・主要標準オブジェクトを含む）の読み込み・保存。
  - **`.exo` / `.exa`**: 拡張編集オブジェクトファイル（Shift_JIS (CP932) / UTF-8 自動判別、中間点・トラックバー・主要標準エフェクトの保持）およびエイリアスファイルの読み込み。`.txt` はテキストオブジェクトとして取り込み。
  - **拡張編集オブジェクト・テキスト制御タグの描画**:
    - テキスト制御タグ（`<#rrggbb>` / `<#rrggbb,rrggbb>` 文字・影縁色変更、`<sN,フォント名>` 文字サイズ・フォント変更、`<rN>` 表示速度変更、`<wN>` / `<*wN>` 表示ウェイト、`<cN>` クリア待機、`<pX,Y>` / `<p+X,+Y>` 座標オフセット、`<?=式?>` 数式評価）に対応。
    - レイヤー単位の「上のオブジェクトでクリッピング」「座標のリンク」、オブジェクト単位の「直前オブジェクト」「音声波形表示（実音声 PCM 連動）」に対応。
- **拡張編集スクリプト実行 & プラグイン定義の静的読み取り (`au2-plugin`)**:
  - **拡張編集スクリプト (`.anm` / `.obj` / `.cam` / `.scn` / `.tra`)**: 独自の組み込み Lua サブセットインタプリタを内蔵し、基本的な座標・回転・拡大率・透明度制御（`obj.ox`, `obj.draw()`, `obj.load()`, `obj.mes()`, `obj.effect()` 等）をプレビューに反映します。
  - **ネイティブ DLL プラグイン (`.auf` / `.aui` / `.auo` / `.auc` / `.auf2` 等)**: PE バイナリ内の `FILTER_DLL` / `FILTER_PLUGIN_TABLE` 構造体からトラックバーやチェックボックス等の**パラメータ定義を静的に抽出**します（※ DLL の動的ロードや機械語コードの実行は行いません。詳細は下記の制限事項を参照してください）。
- **AviUtl 風の操作感 & タイムライン UI**:
  - 日本語フォント「**LINE Seed JP**」をバイナリに同梱。システムフォントの選択・TTC インデックス識別にも対応。
  - **タイムライン横縮尺の拡大縮小（ズーム）**:
    - 右下ズームバーの対数スライダー・`[-]`・`[+]` ボタン・全体フィットボタン
    - **`Ctrl + マウスホイール`** によるカーソル位置中心のズームイン／ズームアウト
    - キーボードショートカット（`Ctrl + +` / `Ctrl + -` / `Ctrl + 0`）
  - 新規プロジェクトへ動画・画像を配置した際の自動解像度・フレームレート同期。
  - レイヤー右クリックからのオブジェクト追加（テキスト、図形、フレームバッファ、色調補正、グループ制御、カメラ制御、カスタムオブジェクト）。
  - 中間点追加・分割・削除、In/Out 範囲指定、Shift+ホイール横スクロール。
  - シングルウィンドウ（ドッキング）とマルチウィンドウ（分離）の切替。

---

## 現在の対応範囲と制限事項 (Limitations)

OpenAvi は現在開発中のプロジェクトであり、本家 AviUtl / AviUtl2 のすべての機能やプラグインと完全互換ではありません。主な制限事項は以下の通りです。

1. **ネイティブ DLL プラグイン (`.auf` / `.aui` / `.auo` / `.auc` / `.aux2` / `.mod2`) はコード実行非対応**
   - `LoadLibrary` / `libloading` 等による DLL の動的ロードや、ネイティブ機械語（`func_proc` によるピクセル処理・フック処理・Win32 独自ウィンドウ `HWND` の埋め込み表示）の実行は**行っていません**。
   - `au2-plugin` の PE パーサーは、バイナリ内の `FILTER_DLL` / `FILTER_PLUGIN_TABLE` テーブルから「フィルタ名・トラックバー名・最小/最大/初期値・チェックボックス定義」を静的に読み取る機能のみを提供します。
   - 付属の「`.auf` パラメータ抽出・雛形生成」ツールは、抽出したパラメータ定義と名称からの簡易推論をもとに手動移植用の `.anm` / WGSL / Rust ボイラープレートを生成する補助機能であり、元の DLL のアルゴリズムや描画結果を再現するものではありません。
2. **Lua スクリプトエンジンは独自サブセット実装**
   - `crates/au2-plugin/src/lua_vm.rs` は Rust による軽量な独自サブセット実装であり、公式の Lua 5.1 VM や LuaJIT ではありません。
   - 基本的な変数・演算・条件分岐・ループ・主要 `obj.*` API には対応していますが、複雑なメタテーブル操作、コルーチン、`require` / `package.loadlib` を介した外部 C/DLL モジュール（`rikky_module`, `T_ogo_Module` 等）、および AviUtl プロセスメモリへの直接アクセスには対応していません。
3. **プロジェクト・オブジェクト互換性 (`.aup` / `.aup2` / `.exo`)**
   - 標準的な動画・画像・音声・テキスト・図形オブジェクトおよび主要な標準フィルタ効果の読み書きに対応していますが、サードパーティ製プラグイン固有の拡張バイナリデータや未実装のフィルタ効果については、パラメータ保持のみとなるか描画に反映されない場合があります。

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
    ├── au2-plugin/           # 組み込み Lua サブセット実行エンジン & PE パラメータ定義抽出
    ├── au2-platform/         # OS 依存処理の窓口 (ホーム/データ領域・実行ファイル名など)
    └── au2-app/              # デスクトップ GUI アプリケーション本体 (openavi)
```

設計の詳細・移植状況は [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) を参照してください。

> **対応 OS**: Windows / macOS / Linux（クロスプラットフォーム対応）。音声出力（Windows: `winmm` / macOS: `AudioToolbox` / Linux: `PipeWire・PulseAudio・ALSA`）および FFmpeg の自動検出・自動ダウンロードも全 OS に対応しています。

---

## ダウンロード＆起動方法（一般ユーザー向け）

インストール不要のポータブル設計（AviUtl スタイル）です。

1. [Releases](../../releases) から **`OpenAvi-windows-x64.zip`** をダウンロードして解凍します。
2. フォルダ内の **`openavi.exe` をダブルクリックするだけ** で即座に起動します。
   - インストーラーによるレジストリ登録等は行わないため、フォルダごと自由な場所に移動したり、USB メモリに入れて持ち運ぶことも可能です。
   - 動画・音声の読み込みに必要な FFmpeg 等もあらかじめ同梱・自動連携されているため、追加設定なしですぐにお使いいただけます。

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

## トラブルシューティング / FAQ

**Q. 動画が映らない（ファイル名しか表示されない）／MP3 などの音が出ない**
動画・音声のデコードには FFmpeg (`bin/ffmpeg.exe`) を使用します。Releases 配布の ZIP には同梱されています。
FFmpeg が無い場合（古い版・ソース ZIP・自前ビルドなど）は、起動後に画面上部へ表示される **「⬇ FFmpeg を自動ダウンロード」** ボタンを押すと、`%LOCALAPPDATA%\OpenAvi\bin` に自動導入されます。手動で `ffmpeg.exe` / `ffprobe.exe` を `bin/` に置くか、PATH を通しても動作します。

**Q. `install.bat` でフォルダが作られない**
ZIP を **「すべて展開」してから** 展開先フォルダ内で実行してください（ZIP 内から直接実行すると失敗します）。インストール不要で、展開フォルダの `openavi.exe` を直接起動することもできます。

**Q. 配布フォルダに `crates` フォルダがない**
正常です。`crates/` はソースコードで、ビルド済みの配布 ZIP には含まれません（実行に不要です）。

---

## ライセンス

本ソフトウェアは [MIT License](LICENSE) の下で公開されています。
