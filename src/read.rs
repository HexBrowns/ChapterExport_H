//! 本体から読む（`call_read_section` だけ。本体の Undo に触れない）。

use aviutl2::generic::{ObjectHandle, ReadSection};

use crate::format::Chapter;
use crate::settings::{LayerBy, Settings, Source};
use crate::text;
use crate::EDIT_HANDLE;

/// 本体のテキストオブジェクトの効果名と、本文の項目名
const TEXT_EFFECT: &str = "テキスト";
const TEXT_ITEM: &str = "テキスト";

/// 1 回読んだ結果
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub chapters: Vec<Chapter>,
    pub warnings: Vec<String>,
    /// fps（`rate / scale`）
    pub rate: i64,
    pub scale: i64,
    /// フレームの範囲選択（あれば）
    pub range: Option<(usize, usize)>,
    /// シーンの最後のフレーム
    pub frame_max: usize,
    /// 今のレイヤー（マークの位置へ移動するときに使う）
    pub cursor_layer: usize,
    /// 名前のあるレイヤー（0 始まりの番号, 名前）。レイヤーを選ぶ一覧に出す
    pub named_layers: Vec<(usize, String)>,
    /// 読んだレイヤー（0 始まり）
    pub layer: Option<usize>,
    /// 読んだシーンの名前（別のシーンを開いたままコピーしないよう、上に出す）
    pub scene_name: String,
}

/// 読む。登録が終わる前なら None
pub fn read(settings: &Settings) -> Option<Result<Snapshot, String>> {
    if !EDIT_HANDLE.is_ready() {
        return None;
    }
    let info = EDIT_HANDLE.get_edit_info();
    let settings = settings.clone();
    let layer_max = info.layer_max;
    let mut snap = Snapshot {
        rate: *info.fps.numer() as i64,
        scale: *info.fps.denom() as i64,
        range: info.select_range.as_ref().map(|r| (*r.start(), *r.end())),
        frame_max: info.frame_max,
        cursor_layer: info.layer,
        ..Default::default()
    };
    let res = EDIT_HANDLE.call_read_section(move |s| {
        let mut chapters = Vec::new();
        let mut warnings = Vec::new();
        let mut named = Vec::new();
        let scene_name = s.get_scene_name().unwrap_or_default();
        for l in (0..=layer_max).filter(|_| settings.source == Source::Layer) {
            if let Ok(Some(name)) = s.get_layer_name(l) {
                if !name.is_empty() {
                    named.push((l, name));
                }
            }
        }
        let mut layer = None;
        match settings.source {
            Source::Mark => read_marks(s, &mut chapters, &mut warnings),
            Source::Layer => {
                layer = match settings.layer_by {
                    LayerBy::Name => {
                        let want = settings.layer_name.trim();
                        let hit = named.iter().find(|(_, n)| n.trim() == want).map(|(l, _)| *l);
                        if hit.is_none() {
                            warnings.push(format!("名前が「{want}」のレイヤーがありません"));
                        }
                        hit
                    }
                    LayerBy::Number => {
                        if settings.layer_number >= 1 && settings.layer_number - 1 <= layer_max {
                            Some(settings.layer_number - 1)
                        } else {
                            warnings.push(format!("レイヤー {} はありません", settings.layer_number));
                            None
                        }
                    }
                };
                if let Some(l) = layer {
                    read_layer(s, l, &mut chapters, &mut warnings);
                }
            }
        }
        (chapters, warnings, named, layer, scene_name)
    });
    Some(match res {
        Ok((chapters, warnings, named, layer, scene_name)) => {
            snap.scene_name = scene_name;
            snap.chapters = chapters;
            snap.warnings = warnings;
            snap.named_layers = named;
            snap.layer = layer;
            Ok(snap)
        }
        Err(e) => Err(format!("本体から読めませんでした: {e}")),
    })
}

fn read_marks(s: &ReadSection, chapters: &mut Vec<Chapter>, warnings: &mut Vec<String>) {
    let frames = match s.get_mark_frame_list() {
        Ok(f) => f,
        Err(e) => {
            warnings.push(format!("マークを読めませんでした: {e}"));
            return;
        }
    };
    if frames.is_empty() {
        warnings.push("マークがありません（レイヤー編集の上部の右クリックメニューからフレームをマークし、メモにタイトルを書く）".into());
    }
    for f in frames {
        let memo = s.get_mark_frame_memo(f).unwrap_or_default();
        // メモの 1 行目をタイトルにする（改行は \n の 2 文字で返る見込み。推測なので両方を扱う）
        let title = text::unescape(&memo).lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string();
        chapters.push(Chapter { frame: f, title, layer: None });
    }
}

fn read_layer(s: &ReadSection, layer: usize, chapters: &mut Vec<Chapter>, warnings: &mut Vec<String>) {
    let mut n = 0;
    let mut unnamed = Vec::new();
    for (lf, h) in s.objects_in_layer(layer) {
        n += 1;
        match title_of(s, h) {
            Some(title) => chapters.push(Chapter { frame: lf.start, title, layer: Some(layer) }),
            None => unnamed.push(format!("レイヤー {} の {} フレーム目", layer + 1, lf.start)),
        }
    }
    crate::format::summarize(warnings, &unnamed, "オブジェクト名が無いので出していません（テキストなら本文の 1 行目を使います）");
    if n == 0 {
        warnings.push(format!("レイヤー {} にオブジェクトがありません", layer + 1));
    }
}

/// オブジェクト名。標準の名前（null）ならテキストの本文の 1 行目。どちらも無ければ None
fn title_of(s: &ReadSection, h: ObjectHandle) -> Option<String> {
    if let Ok(Some(name)) = s.get_object_name(h) {
        if !name.trim().is_empty() {
            return Some(name.trim().to_string());
        }
    }
    let first = s.get_first_effect(h).ok()?;
    if s.get_effect_name(first).ok()?.as_str() != TEXT_EFFECT {
        return None;
    }
    let raw = s.get_object_effect_item(h, TEXT_EFFECT, 0, TEXT_ITEM).ok()?;
    let t = text::title_from_text(&raw);
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}
