# ChapterExport_H（チャプター書き出し）

本体のマーク（マークリストのメモ）か、指定したレイヤーに置いたオブジェクトから、YouTube・ニコニコ動画の概要欄に貼る
チャプター（`0:00 タイトル` の並び）を作ってコピーする AviUtl2 の汎用プラグイン。プロジェクトは書き換えない。

- 仕様: `AI/specifications/20261009_ChapterExport_H_spec.md`
- 本体 2.1.11 以降（aviutl2-rs 0.48）

## ビルドと配置

```bash
python AI/tools/au2_build.py ChapterExport_H                 # 本番へ
python AI/tools/au2_build.py ChapterExport_H --no-deploy     # 本番に書かず、違いだけ出す
```

## 作りの要点

| ファイル | 中身 |
|---|---|
| `src/format.rs` | フレーム → 秒（切り捨て）、時刻の形、書式への流し込み、YouTube の条件の検査（cargo test の中心） |
| `src/text.rs` | テキストの本文から制御文字を除いて 1 行目を取る（TextReplace_H の `text.rs` を写した部分がある） |
| `src/settings.rs` | 設定（プロジェクトごとに `.aup2` へ残す） |
| `src/read.rs` | `call_read_section` でマークかレイヤーのオブジェクトを読む |
| `src/gui.rs` | ウィンドウ（egui） |

- 読むのは `call_read_section` だけ。書き込みは行の「移動」ボタン（今のフレームを動かす）のときの `call_edit_section` だけ
- 見えている間は、本体のイベント（オブジェクトの更新・シーンの変更）と 1 秒ごとに読み直す（マークの変更はイベントで来ないことがあるため）
- 自分で起こすスレッドは無い
