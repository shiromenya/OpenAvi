# OpenAvi アーキテクチャ概要

新しくコードを読む人向けの地図です。変更前にまずここを確認してください。

## クレート依存関係

```text
au2-app ──► au2-aup ──► au2-exo ──► au2-core
   │          └────────────────────► au2-core
   ├────────► au2-plugin ─► au2-core
   ├────────► au2-media
   └────────► au2-platform   (au2-media / au2-plugin からも利用)
```

| クレート | 責務 | 備考 |
|---|---|---|
| `au2-core` | プロジェクト / シーン / レイヤー / オブジェクト / トラック（補間）のデータモデル | UI・ファイル形式・OS に非依存 |
| `au2-exo` | `.exo` / `.exa`（拡張編集エイリアス）の読み書き、文字コード判別 | ラウンドトリップ保持が必須 |
| `au2-aup` | `.aup2`（AviUtl2）と `.aup`（初代バイナリ。`legacy_aup.rs`）の読み書き | 旧 `.aup` は内部で `.exo` テキストに変換して `au2-exo` に渡す |
| `au2-plugin` | `.anm/.obj/.cam/.scn/.tra`（Lua 互換 VM: `lua_vm.rs`）と PE プラグイン解析（`pe_loader.rs`） | PE 解析は純粋なバイト列処理（OS 非依存） |
| `au2-media` | 画像・動画・音声のデコード、FFmpeg 探索、音声出力エンジン | **音声出力のみ Windows 専用（下記）** |
| `au2-platform` | **OS 依存の窓口**（ホーム/デスクトップ/ユーザーデータ領域、`.exe` 拡張子、コンソール非表示） | 他クレートは環境変数や `cfg(windows)` を直接使わずここを経由する |
| `au2-app` | eframe/egui の GUI（`state.rs` = 編集状態、`panels/` = 各パネル、`app.rs` = ウィンドウ/メニュー） | |

## クロスプラットフォーム対応状況

| 領域 | 状態 |
|---|---|
| データモデル / `.aup` `.aup2` `.exo` `.exa` / Lua VM / PE 解析 | 移植済み（OS 非依存） |
| GUI（eframe + wgpu） / ファイルダイアログ（rfd） / フォント | 移植可能（OS 別フォールバックあり） |
| FFmpeg 探索 | OS 非依存（Windows 固有の探索先は `cfg!(windows)` で分離） |
| FFmpeg 自動ダウンロード | Windows のみ（他 OS は案内メッセージを返す） |
| 保存確認ダイアログ | Windows: Win32 API でボタン文言を変更 / 他 OS: `rfd` 標準ダイアログ（`app.rs`） |
| **音声出力** | **Windows のみ**（`au2-media/src/audio.rs` の winmm `waveOut`）。他 OS は無音のスタブ |

> 他 OS 対応で最優先の課題は音声出力です。`AudioEngine::worker_loop` の `#[cfg(not(windows))]` 実装を
> `cpal` 等で置き換えれば、残りのコードは変更不要です。

## 規約

- OS 依存コードは `au2-platform` に追加する。やむを得ず他クレートに置く場合は `#[cfg(windows)]` と非 Windows 実装を必ず対で用意する。
- テストに個人環境の絶対パスを書かない。リポジトリ同梱データは `CARGO_MANIFEST_DIR` 相対、個人サンプルは `au2_platform::desktop_dir()` 経由にし、存在しなければスキップする。
- 巨大ファイル（`preview.rs` / `legacy_aup.rs` / `lua_vm.rs` など）に機能を足す際は、先頭のモジュールドキュメントに概要を追記し、可能なら責務単位でモジュール分割する。
- `.aup`（旧バイナリ）はバージョン差（ExEdit 0.90 / 0.91 / 0.92）で構造体サイズが異なる。オフセットは固定値ではなく `base_size` から算出し、スライスアクセスは長さを確認する（過去にここで範囲外パニックが発生）。
