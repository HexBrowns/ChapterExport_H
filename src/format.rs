//! 時刻の整形・書式・検査（本体と関係なく、cargo test の中心）。
//!
//! YouTube のチャプターの条件（YouTube ヘルプ「動画のチャプター」、2026-10-10 に確認）:
//! 先頭が 00:00 / 3 つ以上 / 各チャプター 10 秒以上 / 昇順。
//! ニコニコ動画の条件は確かめていないので検査しない。

use serde::{Deserialize, Serialize};

/// 1 つのチャプター（読み取った元）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    /// シーンの先頭からのフレーム
    pub frame: usize,
    pub title: String,
    /// レイヤーのオブジェクトから取ったならそのレイヤー（0 始まり）。マークなら None
    pub layer: Option<usize>,
}

/// 書式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Style {
    #[default]
    YouTube,
    Niconico,
    Custom,
}

impl Style {
    pub fn label(self) -> &'static str {
        match self {
            Style::YouTube => "YouTube",
            Style::Niconico => "ニコニコ動画",
            Style::Custom => "カスタム",
        }
    }
}

/// `{time}` の形
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimeForm {
    /// `m:ss`（1 時間以上は `h:mm:ss`）
    #[default]
    MSS,
    /// `mm:ss`（1 時間以上は `h:mm:ss`）
    MMSS,
    /// `hh:mm:ss`
    HHMMSS,
}

impl TimeForm {
    pub fn label(self) -> &'static str {
        match self {
            TimeForm::MSS => "m:ss",
            TimeForm::MMSS => "mm:ss",
            TimeForm::HHMMSS => "hh:mm:ss",
        }
    }
}

/// 書き出しの設定
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub style: Style,
    /// カスタムのテンプレート（`{time}` `{title}` `{index}`）
    pub template: String,
    /// カスタムの `{time}` の形
    pub form: TimeForm,
}

pub const DEFAULT_TEMPLATE: &str = "{time} {title}";

impl Options {
    fn template(&self) -> &str {
        match self.style {
            Style::Custom => &self.template,
            _ => DEFAULT_TEMPLATE,
        }
    }

    fn form(&self) -> TimeForm {
        match self.style {
            Style::YouTube => TimeForm::MSS,
            Style::Niconico => TimeForm::MMSS,
            Style::Custom => self.form,
        }
    }
}

/// フレームを秒にする（切り捨て）。fps は `rate / scale`（`EDIT_INFO` の形）。
/// 概要欄のリンクは秒単位なので、切り上げると頭が欠ける
pub fn frame_to_secs(frame: usize, rate: i64, scale: i64) -> u64 {
    if rate <= 0 || scale <= 0 {
        return 0;
    }
    ((frame as u128 * scale as u128) / rate as u128) as u64
}

pub fn format_time(secs: u64, form: TimeForm) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    match form {
        TimeForm::HHMMSS => format!("{h:02}:{m:02}:{s:02}"),
        _ if h > 0 => format!("{h}:{m:02}:{s:02}"),
        TimeForm::MSS => format!("{m}:{s:02}"),
        TimeForm::MMSS => format!("{m:02}:{s:02}"),
    }
}

/// 一覧の 1 行
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// 元のフレーム（シーンの先頭から）。移動に使う
    pub frame: usize,
    pub layer: Option<usize>,
    /// 書き出しの起点からの秒
    pub secs: u64,
    pub time: String,
    pub title: String,
    /// 書き出す行
    pub text: String,
    /// この行の警告
    pub warnings: Vec<String>,
}

/// 書き出しの結果
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Output {
    pub lines: Vec<Line>,
    /// 全体の警告
    pub warnings: Vec<String>,
}

impl Output {
    /// 概要欄に貼る文字列（行を改行でつなぐ）
    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    }
}

/// 書き出しの範囲
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// この フレームを 0 秒にする
    pub start: usize,
    /// 最後のフレーム（最後のチャプターの長さを測るため）
    pub end: usize,
}

/// チャプターを並べて書式に流し込み、警告を付ける
pub fn build(chapters: &[Chapter], span: Span, rate: i64, scale: i64, opts: &Options) -> Output {
    let mut out = Output::default();
    let mut list: Vec<&Chapter> = chapters.iter().filter(|c| c.frame >= span.start && c.frame <= span.end).collect();
    let outside = chapters.len() - list.len();
    if outside > 0 {
        out.warnings.push(format!("範囲の外の {outside} 件は出していません"));
    }
    list.sort_by_key(|c| c.frame);

    let form = opts.form();
    let template = opts.template();
    let mut index = 0;
    let mut empty = Vec::new();
    for c in list {
        let title = c.title.trim();
        if title.is_empty() {
            empty.push(describe(c));
            continue;
        }
        index += 1;
        let secs = frame_to_secs(c.frame - span.start, rate, scale);
        let time = format_time(secs, form);
        let text = template.replace("{time}", &time).replace("{title}", title).replace("{index}", &index.to_string());
        out.lines.push(Line {
            frame: c.frame,
            layer: c.layer,
            secs,
            time,
            title: title.to_string(),
            text,
            warnings: Vec::new(),
        });
    }

    summarize(&mut out.warnings, &empty, "タイトルが空なので出していません");

    // 同じ時刻（秒）が続くもの
    for i in 1..out.lines.len() {
        if out.lines[i].secs == out.lines[i - 1].secs {
            let t = out.lines[i].time.clone();
            out.lines[i].warnings.push(format!("前と同じ時刻 {t}"));
        }
    }

    if opts.style == Style::YouTube {
        check_youtube(&mut out, span, rate, scale);
    }
    out
}

/// 同じ理由の警告を 1 行にまとめる（マークを拍ごとに打つ使い方では数百件になり、一覧を押し出す）。
/// 3 件までは場所ごとに、それより多ければ件数と先頭の 3 件だけを出す
pub fn summarize(warnings: &mut Vec<String>, places: &[String], reason: &str) {
    const SHOWN: usize = 3;
    if places.len() <= SHOWN {
        for p in places {
            warnings.push(format!("{p}: {reason}"));
        }
        return;
    }
    let head = places[..SHOWN].join("、");
    warnings.push(format!("{} 件: {reason}（{head} ほか）", places.len()));
}

fn describe(c: &Chapter) -> String {
    match c.layer {
        Some(l) => format!("レイヤー {} の {} フレーム目", l + 1, c.frame),
        None => format!("{} フレーム目のマーク", c.frame),
    }
}

/// YouTube のチャプターの条件（先頭 0:00 / 3 つ以上 / 各 10 秒以上）
fn check_youtube(out: &mut Output, span: Span, rate: i64, scale: i64) {
    if out.lines.is_empty() {
        return;
    }
    if out.lines[0].secs != 0 {
        out.warnings.push(format!("YouTube: 先頭が 0:00 ではありません（{}）", out.lines[0].time));
    }
    if out.lines.len() < 3 {
        out.warnings.push(format!("YouTube: 3 つ以上必要です（{} つ）", out.lines.len()));
    }
    let end_secs = frame_to_secs(span.end + 1 - span.start, rate, scale);
    let n = out.lines.len();
    let mut short = 0;
    for i in 0..n {
        let next = if i + 1 < n { out.lines[i + 1].secs } else { end_secs };
        let len = next.saturating_sub(out.lines[i].secs);
        if len < 10 {
            out.lines[i].warnings.push(format!("10 秒未満（{len} 秒）"));
            short += 1;
        }
    }
    if short > 0 {
        out.warnings.push(format!("YouTube: 10 秒未満のチャプターが {short} つあります"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(frame: usize, title: &str) -> Chapter {
        Chapter { frame, title: title.into(), layer: None }
    }

    fn yt() -> Options {
        Options { style: Style::YouTube, template: DEFAULT_TEMPLATE.into(), form: TimeForm::MSS }
    }

    #[test]
    fn secs_floor() {
        assert_eq!(frame_to_secs(0, 30, 1), 0);
        assert_eq!(frame_to_secs(29, 30, 1), 0);
        assert_eq!(frame_to_secs(30, 30, 1), 1);
        assert_eq!(frame_to_secs(1797, 30, 1), 59); // 59.9 秒
        assert_eq!(frame_to_secs(1800, 30, 1), 60);
        // 29.97fps（30000/1001）: 1800 フレームは 60.06 秒
        assert_eq!(frame_to_secs(1800, 30000, 1001), 60);
        assert_eq!(frame_to_secs(1798, 30000, 1001), 59);
        assert_eq!(frame_to_secs(10, 0, 1), 0);
    }

    #[test]
    fn time_forms() {
        assert_eq!(format_time(0, TimeForm::MSS), "0:00");
        assert_eq!(format_time(59, TimeForm::MSS), "0:59");
        assert_eq!(format_time(60, TimeForm::MSS), "1:00");
        assert_eq!(format_time(3599, TimeForm::MSS), "59:59");
        assert_eq!(format_time(3600, TimeForm::MSS), "1:00:00");
        assert_eq!(format_time(65, TimeForm::MMSS), "01:05");
        assert_eq!(format_time(3661, TimeForm::MMSS), "1:01:01");
        assert_eq!(format_time(65, TimeForm::HHMMSS), "00:01:05");
    }

    #[test]
    fn youtube_ok() {
        let c = [ch(0, "オープニング"), ch(30 * 15, "本編"), ch(30 * 40, "まとめ")];
        let o = build(&c, Span { start: 0, end: 30 * 60 - 1 }, 30, 1, &yt());
        assert_eq!(o.text(), "0:00 オープニング\n0:15 本編\n0:40 まとめ");
        assert!(o.warnings.is_empty(), "{:?}", o.warnings);
        assert!(o.lines.iter().all(|l| l.warnings.is_empty()));
    }

    #[test]
    fn youtube_warnings() {
        // 先頭が 0:00 でない
        let o = build(&[ch(30, "a"), ch(600, "b"), ch(1200, "c")], Span { start: 0, end: 3000 }, 30, 1, &yt());
        assert!(o.warnings.iter().any(|w| w.contains("先頭")));
        // 2 つ
        let o = build(&[ch(0, "a"), ch(600, "b")], Span { start: 0, end: 3000 }, 30, 1, &yt());
        assert!(o.warnings.iter().any(|w| w.contains("3 つ以上")));
        // 9 秒の区間
        let o = build(&[ch(0, "a"), ch(270, "b"), ch(1200, "c")], Span { start: 0, end: 3000 }, 30, 1, &yt());
        assert!(o.lines[0].warnings.iter().any(|w| w.contains("10 秒未満")));
        assert!(o.warnings.iter().any(|w| w.contains("10 秒未満")));
        // 最後のチャプターが終わりまで 10 秒未満
        let o = build(&[ch(0, "a"), ch(600, "b"), ch(2990, "c")], Span { start: 0, end: 3000 }, 30, 1, &yt());
        assert!(o.lines[2].warnings.iter().any(|w| w.contains("10 秒未満")));
    }

    #[test]
    fn sorted_and_duplicates() {
        // 逆順に並べても時刻の順に出す
        let o = build(&[ch(1200, "c"), ch(0, "a"), ch(600, "b")], Span { start: 0, end: 3000 }, 30, 1, &yt());
        assert_eq!(o.text(), "0:00 a\n0:20 b\n0:40 c");
        // 同じ秒
        let o = build(&[ch(0, "a"), ch(10, "b"), ch(600, "c")], Span { start: 0, end: 3000 }, 30, 1, &yt());
        assert!(o.lines[1].warnings.iter().any(|w| w.contains("同じ時刻")));
    }

    #[test]
    fn empty_title_skipped() {
        let o = build(&[ch(0, "a"), ch(300, "  "), ch(600, "c")], Span { start: 0, end: 3000 }, 30, 1, &yt());
        assert_eq!(o.lines.len(), 2);
        assert!(o.warnings.iter().any(|w| w.contains("300 フレーム目のマーク") && w.contains("タイトルが空")));
    }

    #[test]
    fn range_origin() {
        // 範囲 300〜2100: 範囲外は落ち、先頭が 0:00
        let c = [ch(0, "前"), ch(300, "a"), ch(900, "b"), ch(1500, "c"), ch(2400, "後")];
        let o = build(&c, Span { start: 300, end: 2100 }, 30, 1, &yt());
        assert_eq!(o.text(), "0:00 a\n0:20 b\n0:40 c");
        assert!(o.warnings.iter().any(|w| w.contains("範囲の外の 2 件")));
    }

    #[test]
    fn many_empty_titles_are_one_warning() {
        let mut v: Vec<Chapter> = (0..300).map(|i| ch(i * 30, "")).collect();
        v.push(ch(0, "本編"));
        let o = build(&v, Span { start: 0, end: 9000 }, 30, 1, &Options { style: Style::Niconico, template: String::new(), form: TimeForm::MSS });
        assert_eq!(o.warnings.len(), 1, "{:?}", o.warnings);
        assert!(o.warnings[0].starts_with("300 件: タイトルが空"), "{}", o.warnings[0]);
        assert!(o.warnings[0].contains("0 フレーム目のマーク"));
    }

    #[test]
    fn niconico_and_custom() {
        let c = [ch(0, "a"), ch(30 * 65, "b")];
        let nico = Options { style: Style::Niconico, template: String::new(), form: TimeForm::MSS };
        let o = build(&c, Span { start: 0, end: 30 * 100 }, 30, 1, &nico);
        assert_eq!(o.text(), "00:00 a\n01:05 b");
        assert!(o.warnings.is_empty(), "ニコニコ動画は検査しない: {:?}", o.warnings);
        let custom = Options { style: Style::Custom, template: "{index}. [{time}] {title}".into(), form: TimeForm::HHMMSS };
        let o = build(&c, Span { start: 0, end: 30 * 100 }, 30, 1, &custom);
        assert_eq!(o.text(), "1. [00:00:00] a\n2. [00:01:05] b");
    }
}
