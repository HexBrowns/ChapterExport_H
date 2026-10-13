//! チャプター書き出しのウィンドウ（egui）
//!
//! 見えている間は、本体のイベント（オブジェクトの更新・シーンの変更）で読み直す。続けて来ても 0.4 秒に 1 回まで。
//! マークが元のときだけ、1 秒ごとにも読み直す（マークの変更は本体のイベントで来ないことがあるため）。
//! 再生中・ファイル出力中は間隔では読まない（出力中は本体が読み取りを断る）。
//! 読むのは `call_read_section` だけ。書き込みは行の「移動」ボタン（今のフレームを動かす）だけで、
//! そのときだけ `call_edit_section` を呼ぶ（ルール au2-rs-plugin「EditHandle」「ウィンドウの重さ」）。

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use aviutl2::generic::EditState;
use aviutl2_eframe::{eframe, egui, AviUtl2EframeHandle};

use crate::format::{self, Options, Output, Span, Style, TimeForm};
use crate::read::{self, Snapshot};
use crate::settings::{LayerBy, Settings, Source};
use crate::{EDIT_HANDLE, OBJECT_DIRTY, PROJECT, SETTINGS, SETTINGS_GEN};

const YELLOW: egui::Color32 = egui::Color32::from_rgb(235, 200, 90);
const RED: egui::Color32 = egui::Color32::from_rgb(235, 110, 110);

/// マークが元のときの読み直しの間隔（マークの変更は本体のイベントで来ないことがあるため）
const POLL: Duration = Duration::from_secs(1);
/// 本体のイベントが続けて来ても、読むのはこの間隔に 1 回まで（ドラッグ中はイベントが来続ける）
const MIN_READ_GAP: Duration = Duration::from_millis(400);
/// 1 回の読み取りがこれより長ければ、1 回だけログに出す
const SLOW_READ: Duration = Duration::from_millis(50);
/// 見えていない間に、見えたかどうかを調べる間隔
const HIDDEN_CHECK: Duration = Duration::from_millis(500);
/// 下に出す全体の警告の欄の高さの上限（超えたらその中でスクロールする）
const WARNINGS_MAX_HEIGHT: f32 = 72.0;

pub struct ChapterApp {
    _handle: AviUtl2EframeHandle,
    /// 本体のフォント（窓が見えてから読む。`host_fonts`）
    host_fonts: crate::host_fonts::LazyFonts,
    settings: Settings,
    settings_gen: u64,
    snap: Option<Snapshot>,
    out: Output,
    /// 読めなかった理由。前に読めた結果があれば、それを出したまま添える
    read_error: Option<String>,
    /// 最後にログへ出した読めなかった理由（同じ理由を繰り返し出さない）
    logged_error: Option<String>,
    slow_read_logged: bool,
    last_read: Option<Instant>,
    status: String,
    status_is_error: bool,
}

impl ChapterApp {
    pub fn new(cc: &eframe::CreationContext<'_>, handle: AviUtl2EframeHandle) -> Self {
        cc.egui_ctx.all_styles_mut(|style| {
            style.visuals = aviutl2_eframe::aviutl2_visuals();
        });
        // 本体のフォントは窓が見えてから ui で読む（ここで読むと、本体の起動がこの窓の初期化を待つ間ずっと止まる。`host_fonts`）
        let host_fonts = crate::host_fonts::LazyFonts::new(&handle).log_time("ChapterExport_H");
        let _ = crate::EGUI_CTX.set(cc.egui_ctx.clone());
        ChapterApp {
            _handle: handle,
            host_fonts,
            settings: SETTINGS.lock().clone(),
            settings_gen: SETTINGS_GEN.load(Ordering::Acquire),
            snap: None,
            out: Output::default(),
            read_error: None,
            logged_error: None,
            slow_read_logged: false,
            last_read: None,
            status: String::new(),
            status_is_error: false,
        }
    }

    fn options(&self) -> Options {
        Options { style: self.settings.style, template: self.settings.template.clone(), form: self.settings.form }
    }

    /// 設定が変わったら共有の設定へ写す（プロジェクトの保存で `.aup2` に残る）
    fn store_settings(&mut self) {
        *SETTINGS.lock() = self.settings.clone();
        self.settings_gen = SETTINGS_GEN.fetch_add(1, Ordering::AcqRel) + 1;
    }

    fn reload(&mut self) {
        let started = Instant::now();
        match read::read(&self.settings) {
            None => return,
            Some(Ok(snap)) => {
                self.read_error = None;
                self.logged_error = None;
                self.snap = Some(snap);
            }
            Some(Err(e)) => {
                // 出力中などで読めないときは、前に読めた結果を出したままにする（コピーできるように）。ログは理由ごとに 1 回
                if self.logged_error.as_deref() != Some(e.as_str()) {
                    tracing::warn!("[ChapterExport_H] {e}");
                    self.logged_error = Some(e.clone());
                }
                self.read_error = Some(e);
            }
        }
        let took = started.elapsed();
        if took > SLOW_READ && !self.slow_read_logged {
            self.slow_read_logged = true;
            let n = self.snap.as_ref().map(|s| s.chapters.len()).unwrap_or(0);
            tracing::info!("[ChapterExport_H] 本体からの読み取りに {} ms（チャプター候補 {n} 件）", took.as_millis());
        }
        self.last_read = Some(Instant::now());
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let Some(snap) = &self.snap else {
            self.out = Output::default();
            return;
        };
        let before = std::mem::take(&mut self.out);
        let span = match (self.settings.use_range, snap.range) {
            (true, Some((a, b))) => Span { start: a, end: b },
            _ => Span { start: 0, end: snap.frame_max },
        };
        let mut out = format::build(&snap.chapters, span, snap.rate, snap.scale, &self.options());
        let mut w = snap.warnings.clone();
        w.append(&mut out.warnings);
        out.warnings = w;
        // 一覧が変わったら「N 件をコピーしました」などの古い知らせを消す（失敗の知らせは残す）
        if out != before && !self.status_is_error {
            self.status.clear();
        }
        self.out = out;
    }

    fn poll(&mut self, ctx: &egui::Context) {
        // 本体の登録が終わる前は読まない（ルール au2-rs-plugin「EditHandle」）
        if !EDIT_HANDLE.is_ready() {
            ctx.request_repaint_after(HIDDEN_CHECK);
            return;
        }
        // プロジェクトを開き直したら、そのプロジェクトの設定に切り替える
        let gen = SETTINGS_GEN.load(Ordering::Acquire);
        if gen != self.settings_gen {
            self.settings = SETTINGS.lock().clone();
            self.settings_gen = gen;
            OBJECT_DIRTY.store(true, Ordering::Release);
        }
        let since = self.last_read.map(|t| t.elapsed());
        // 前の読み取りから 0.4 秒経つまでは、印を残したまま待つ
        if let Some(since) = since.filter(|s| *s < MIN_READ_GAP) {
            if OBJECT_DIRTY.load(Ordering::Acquire) {
                ctx.request_repaint_after(MIN_READ_GAP - since);
                return;
            }
        }
        // 間隔で読むのはマークが元で、編集中のときだけ（再生中・出力中は本体を待たせない）
        let editing = matches!(EDIT_HANDLE.get_edit_state(), Ok(EditState::Edit));
        let periodic = self.settings.source == Source::Mark && editing;
        // 出力中に読めなかったときは、編集に戻ってから読み直す
        let retry = self.read_error.is_some() && editing;
        let due = since.is_none() || ((periodic || retry) && since.is_some_and(|s| s >= POLL));
        if OBJECT_DIRTY.swap(false, Ordering::AcqRel) || due {
            self.reload();
        }
        if periodic || retry {
            ctx.request_repaint_after(POLL);
        } else if !editing {
            // 再生・出力が終わったら間隔の読み取りに戻るため、ときどき起きる
            ctx.request_repaint_after(POLL);
        }
    }

    fn render_top(&mut self, ui: &mut egui::Ui) {
        let before = self.settings.clone();
        ui.horizontal(|ui| {
            ui.label("元:");
            ui.radio_value(&mut self.settings.source, Source::Mark, "マーク")
                .on_hover_text("本体のマーク（マークリスト）。メモの 1 行目をタイトルにする");
            ui.radio_value(&mut self.settings.source, Source::Layer, "レイヤー")
                .on_hover_text("指定したレイヤーに置いたオブジェクト。オブジェクト名（無ければテキストの本文の 1 行目）をタイトルにする");
            if ui.button("読み直す").clicked() {
                OBJECT_DIRTY.store(true, Ordering::Release);
            }
            if let Some(s) = &self.snap {
                ui.label(format!("シーン: {}", s.scene_name)).on_hover_text("いま開いているシーンから読みます");
            }
        });
        if self.settings.source == Source::Layer {
            ui.horizontal(|ui| {
                ui.label("レイヤー:");
                ui.radio_value(&mut self.settings.layer_by, LayerBy::Name, "名前");
                ui.radio_value(&mut self.settings.layer_by, LayerBy::Number, "番号");
                match self.settings.layer_by {
                    LayerBy::Name => {
                        let names: Vec<String> =
                            self.snap.as_ref().map(|s| s.named_layers.iter().map(|(_, n)| n.clone()).collect()).unwrap_or_default();
                        egui::ComboBox::from_id_salt("layer_name")
                            .selected_text(self.settings.layer_name.clone())
                            .show_ui(ui, |ui| {
                                if names.is_empty() {
                                    ui.label("名前の付いたレイヤーがありません");
                                }
                                for n in names {
                                    ui.selectable_value(&mut self.settings.layer_name, n.clone(), n);
                                }
                            });
                        // 打っている途中の名前で読み直さない（Enter・ほかをクリックで確定、Esc で元に戻す）
                        line_edit(ui, "layer_name_edit", &mut self.settings.layer_name, 120.0)
                            .0
                            .on_hover_text("レイヤー名（完全一致）。Enter かほかをクリックで確定、Esc で元に戻す");
                    }
                    LayerBy::Number => {
                        drag_value(ui, &mut self.settings.layer_number, |d| d.range(1..=1000));
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("書式:");
            for s in [Style::YouTube, Style::Niconico, Style::Custom] {
                ui.radio_value(&mut self.settings.style, s, s.label());
            }
        });
        if self.settings.style == Style::Custom {
            ui.horizontal(|ui| {
                ui.label("テンプレート:");
                line_edit(ui, "template_edit", &mut self.settings.template, 200.0)
                    .0
                    .on_hover_text("{time} 時刻 / {title} タイトル / {index} 番号（1 から）。Enter かほかをクリックで確定、Esc で元に戻す");
                egui::ComboBox::from_id_salt("form").selected_text(self.settings.form.label()).show_ui(ui, |ui| {
                    for f in [TimeForm::MSS, TimeForm::MMSS, TimeForm::HHMMSS] {
                        ui.selectable_value(&mut self.settings.form, f, f.label());
                    }
                });
            });
        }
        let range_text = match self.snap.as_ref().and_then(|s| s.range) {
            Some((a, b)) => format!("範囲選択（{a}〜{b} フレーム）の中だけを出し、先頭を 0:00 にする"),
            None => "範囲選択があれば、その中だけを出し、先頭を 0:00 にする".to_string(),
        };
        ui.checkbox(&mut self.settings.use_range, range_text);

        if self.settings != before {
            let source_changed =
                self.settings.source != before.source || self.settings.layer_by != before.layer_by || self.settings.layer_name != before.layer_name || self.settings.layer_number != before.layer_number;
            self.store_settings();
            if source_changed {
                OBJECT_DIRTY.store(true, Ordering::Release);
            } else {
                self.rebuild();
            }
        }
    }

    fn render_rows(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = &self.read_error {
            ui.colored_label(RED, e);
            if self.snap.is_none() {
                return;
            }
            ui.label("前に読めた結果を出しています（ファイル出力の最中は本体から読めません）");
        }
        if self.out.lines.is_empty() {
            ui.label("チャプターがありません");
            return;
        }
        let cursor_layer = self.snap.as_ref().map(|s| s.cursor_layer).unwrap_or(0);
        let mut jump = None;
        // 行の高さはそろっているので、見えている行だけを組み立てる（チャプターが数百でもマウスを動かすたびに全行を作らない）
        let row_height = ui.spacing().interact_size.y;
        let lines = &self.out.lines;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_height, lines.len(), |ui, range| {
            for line in &lines[range] {
                ui.horizontal(|ui| {
                    if ui.small_button("移動").on_hover_text(format!("{} フレーム目へ", line.frame)).clicked() {
                        jump = Some((line.layer.unwrap_or(cursor_layer), line.frame));
                    }
                    ui.monospace(&line.time);
                    // 警告はタイトルより前に出す（長いタイトルで右へ押し出されて見えなくならないように）
                    for w in &line.warnings {
                        ui.colored_label(YELLOW, format!("※ {w}"));
                    }
                    ui.add(egui::Label::new(&line.title).truncate());
                });
            }
        });
        if let Some((layer, frame)) = jump {
            self.jump(layer, frame);
        }
    }

    /// 今のフレームを動かす（ボタンを押したときだけ。本体の編集セクションが要る）。
    /// タイムラインに見えていないフレームなら、表示もそこへ送る
    fn jump(&mut self, layer: usize, frame: usize) {
        if !EDIT_HANDLE.is_ready() {
            return;
        }
        let info = EDIT_HANDLE.get_edit_info();
        let shown = info.display_frame.contains(&frame);
        let width = info.display_frame.len();
        let display_layer = info.display_layer.start;
        let res = EDIT_HANDLE.call_edit_section(move |e| {
            e.set_cursor_layer_frame(layer, frame)?;
            if !shown {
                // 移った先が左から 1/4 の位置に来るように
                e.set_display_layer_frame(display_layer, frame.saturating_sub(width / 4))?;
            }
            Ok::<(), aviutl2::generic::EditSectionError>(())
        });
        match res {
            Ok(Ok(())) => {
                if self.status_is_error {
                    self.set_status(String::new());
                }
            }
            Ok(Err(e)) => self.set_error(format!("移動できませんでした: {e}")),
            Err(e) => self.set_error(format!("移動できませんでした: {e}")),
        }
    }

    fn render_bottom(&mut self, ui: &mut egui::Ui) {
        // 警告が多くても一覧とボタンを押し出さないよう、決まった高さの中でスクロールする
        if !self.out.warnings.is_empty() {
            egui::ScrollArea::vertical().id_salt("warnings").max_height(WARNINGS_MAX_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
                for w in &self.out.warnings {
                    ui.colored_label(YELLOW, format!("※ {w}"));
                }
            });
        }
        ui.horizontal(|ui| {
            let has = !self.out.lines.is_empty();
            if ui.add_enabled(has, egui::Button::new("コピー")).clicked() {
                ui.ctx().copy_text(self.out.text());
                self.set_status(format!("{} 件をコピーしました", self.out.lines.len()));
            }
            let target = save_path();
            let tip = match &target {
                Some(p) => format!("{} に書き込む（UTF-8）", p.display()),
                None => "プロジェクトを保存すると、その隣に書き込めます".to_string(),
            };
            if ui.add_enabled(has && target.is_some(), egui::Button::new("テキストに書き込む")).on_hover_text(tip).on_disabled_hover_text("プロジェクトを保存すると、その隣に書き込めます").clicked() {
                if let Some(p) = target {
                    match std::fs::write(&p, self.out.text() + "\n") {
                        Ok(()) => self.set_status(format!("{} に書き込みました", p.display())),
                        Err(e) => self.set_error(format!("{} に書き込めませんでした: {e}", p.display())),
                    }
                }
            }
        });
        let text = egui::RichText::new(if self.status.is_empty() { " " } else { &self.status }).small();
        if self.status_is_error {
            ui.colored_label(RED, text);
        } else {
            ui.label(text);
        }
    }

    fn set_status(&mut self, s: String) {
        self.status = s;
        self.status_is_error = false;
    }

    fn set_error(&mut self, s: String) {
        tracing::warn!("[ChapterExport_H] {s}");
        self.status = s;
        self.status_is_error = true;
    }
}

/// 書き出し先: プロジェクトの隣の `<プロジェクト名>_チャプター.txt`。未保存なら None
fn save_path() -> Option<PathBuf> {
    let p = PROJECT.read().clone()?;
    let stem = p.file_stem()?.to_string_lossy().to_string();
    Some(p.with_file_name(format!("{stem}_チャプター.txt")))
}

/// 数値の欄（ルール au2-rs-plugin「入力の確定と取り消し」）。`update_while_editing(false)` を付けて出す。
/// 打っている途中の値は使わず、Enter かほかをクリックしてフォーカスが外れたときに確定する。入力中の Esc は入力前の値に戻す。
/// ドラッグで変えた値はその場で使う。
///
/// egui 0.36 の `DragValue` は `update_while_editing(false)` でも、Esc の次のフレームで打った文字を値にしてしまう
/// （フォーカスを失ったとみなす期間が 2 フレームあり、2 フレーム目には Esc が押されていないため）。
/// Esc のフレームの値を覚えておき、次のフレームで戻す（参照実装 LayerSilenceCut_H の `setting_value`）
fn drag_value<T>(ui: &mut egui::Ui, value: &mut T, build: impl FnOnce(egui::DragValue<'_>) -> egui::DragValue<'_>) -> egui::Response
where
    T: egui::emath::Numeric + Default + Send + Sync,
{
    let resp = ui.add(build(egui::DragValue::new(value)).update_while_editing(false));
    let key = resp.id.with("escaped");
    let pass = ui.ctx().cumulative_pass_nr();
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.ctx().data_mut(|d| d.insert_temp(key, (pass, *value)));
    } else if let Some((at, before)) = ui.ctx().data_mut(|d| d.remove_temp::<(u64, T)>(key)) {
        if pass == at + 1 {
            *value = before;
        }
    }
    resp
}

/// 確定してから使う 1 行の入力欄（ルール au2-rs-plugin「入力の確定と取り消し」）。
///
/// 打っている間の文字は欄の中（egui の一時領域）だけに持ち、`value` を書き換えるのは
/// Enter・ほかをクリック・Tab でフォーカスが外れたときだけ。入力中の Esc は、入力を始める前の文字に戻す。
/// 戻り値の bool は、確定して `value` が変わったときだけ true
fn line_edit(ui: &mut egui::Ui, id_salt: &str, value: &mut String, width: f32) -> (egui::Response, bool) {
    // ウィンドウの中で一意な名前から作る（親の Ui の自動の ID は、前に出す部品の数で変わる）
    let id = egui::Id::new(("line_edit", id_salt));
    let key = id.with("pending");
    // 入力中の Esc は取り消し。egui はこのフレームの始めに Esc でフォーカスを外しているので、
    // 「フォーカスが無い + このフレームの Esc」なら、入力中だった文字を描く前に捨てる
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().memory(|m| m.has_focus(id)) {
        ui.ctx().data_mut(|d| d.remove_temp::<String>(key));
    }
    // 前のフレームまで入力中だった文字。無ければ今の値を出す
    let mut buf = ui.ctx().data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| value.clone());
    let resp = ui.add(egui::TextEdit::singleline(&mut buf).id(id).desired_width(width));
    let committed = finish_line_edit(ui.ctx(), key, resp.has_focus(), buf, value);
    (resp, committed)
}

/// `line_edit` の後半。フォーカスがある間は入力中の文字を覚え、外れたら確定するか取り消す
fn finish_line_edit(ctx: &egui::Context, key: egui::Id, focused: bool, buf: String, value: &mut String) -> bool {
    if focused {
        ctx.data_mut(|d| d.insert_temp(key, buf));
        return false;
    }
    // 入力中だった文字が残っていれば確定する（Esc で取り消したときは、描く前に捨ててある）
    let was_editing = ctx.data_mut(|d| d.remove_temp::<String>(key)).is_some();
    if !was_editing || buf == *value {
        return false;
    }
    *value = buf;
    true
}

impl eframe::App for ChapterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 本体のフォントは窓が見えてから読む。読んだフレームは描かない（新しいフォントは次のフレームから効く。次のフレームは頼んである）
        if self.host_fonts.load_if_visible(ui.ctx()) {
            return;
        }
        // 本体のパネルに出ていない間は読まない（イベントの印は残り、出たときに読む）
        if !crate::window_visible() {
            ui.ctx().request_repaint_after(HIDDEN_CHECK);
            return;
        }
        self.poll(ui.ctx());
        egui::Panel::top("top").show(ui, |ui| self.render_top(ui));
        egui::Panel::bottom("bottom").show(ui, |ui| self.render_bottom(ui));
        // 中央パネルは最後に追加する（ルール au2-rs-plugin「egui レイアウト」）
        egui::CentralPanel::default().show(ui, |ui| self.render_rows(ui));
    }
}

#[cfg(test)]
mod tests {
    /// テストの 1 フレームの出力を捨てる。テクスチャの差分を片付けずに捨てると、デバッグビルドで epaint の debug_assert
    /// （Dropped TexturesDelta with N unapplied deltas）が落ちる（au2 release の prebuild の cargo test はデバッグビルド）
    fn discard_frame(mut out: egui::FullOutput) {
        out.textures_delta.clear();
    }

    use super::*;

    fn key(k: egui::Key) -> egui::Event {
        egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    /// 1 フレーム回す。戻り値は（確定したか, 欄に出ている文字）
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, value: &mut String, focus: bool) -> (bool, String) {
        let mut out = (false, String::new());
        let raw = egui::RawInput { events, ..Default::default() };
        discard_frame(ctx.run_ui(raw, |ui| {
            let (resp, committed) = line_edit(ui, "t", value, 100.0);
            if focus {
                resp.request_focus();
            }
            let shown = ui.ctx().data(|d| d.get_temp::<String>(resp.id.with("pending"))).unwrap_or_else(|| value.clone());
            out = (committed, shown);
        }));
        out
    }

    /// フォーカスを当てて "X" を打つところまで
    fn start(ctx: &egui::Context, value: &mut String) {
        frame(ctx, vec![], value, true);
        let (committed, shown) = frame(ctx, vec![egui::Event::Text("X".into())], value, false);
        assert!(!committed);
        assert_eq!(shown, "oldX");
        assert_eq!(value, "old", "打っている途中では値を変えない");
    }

    #[test]
    fn enter_commits() {
        let ctx = egui::Context::default();
        let mut value = "old".to_string();
        start(&ctx, &mut value);
        let (committed, _) = frame(&ctx, vec![key(egui::Key::Enter)], &mut value, false);
        assert!(committed);
        assert_eq!(value, "oldX");
        // 次のフレームで二度確定しない
        assert!(!frame(&ctx, vec![], &mut value, false).0);
    }

    #[test]
    fn escape_reverts() {
        let ctx = egui::Context::default();
        let mut value = "old".to_string();
        start(&ctx, &mut value);
        let (committed, shown) = frame(&ctx, vec![key(egui::Key::Escape)], &mut value, false);
        assert!(!committed);
        assert_eq!(value, "old");
        assert_eq!(shown, "old", "欄の文字も入力前に戻る");
        assert!(!frame(&ctx, vec![], &mut value, false).0);
        assert_eq!(value, "old");
    }

    #[test]
    fn losing_focus_commits() {
        let ctx = egui::Context::default();
        let mut value = "old".to_string();
        start(&ctx, &mut value);
        // ほかをクリックしたのと同じく、フォーカスを外す
        ctx.memory_mut(|m| m.stop_text_input());
        let (committed, _) = frame(&ctx, vec![], &mut value, false);
        assert!(committed);
        assert_eq!(value, "oldX");
    }

    /// 数値の欄を 1 フレーム回す
    fn drag_frame(ctx: &egui::Context, events: Vec<egui::Event>, value: &mut i32, focus: bool) {
        let raw = egui::RawInput { events, ..Default::default() };
        discard_frame(ctx.run_ui(raw, |ui| {
            let resp = drag_value(ui, value, |d| d.range(0..=100));
            if focus {
                resp.request_focus();
            }
        }));
    }

    /// フォーカスを当てて 5 を消し、42 を打つところまで
    fn drag_start(ctx: &egui::Context, value: &mut i32) {
        drag_frame(ctx, vec![], value, true);
        let mut events = vec![key(egui::Key::Backspace); 5];
        events.push(egui::Event::Text("42".into()));
        drag_frame(ctx, events, value, false);
        assert_eq!(*value, 5, "打っている途中では値を変えない");
    }

    #[test]
    fn drag_value_escape_reverts_for_following_frames() {
        let ctx = egui::Context::default();
        let mut value = 5;
        drag_start(&ctx, &mut value);
        drag_frame(&ctx, vec![key(egui::Key::Escape)], &mut value, false);
        assert_eq!(value, 5);
        // egui は Esc の次のフレームで打った文字を値にする。3 フレーム先まで元のままか
        for n in 1..=3 {
            drag_frame(&ctx, vec![], &mut value, false);
            assert_eq!(value, 5, "Esc の {n} フレーム後");
        }
    }

    #[test]
    fn drag_value_enter_commits() {
        let ctx = egui::Context::default();
        let mut value = 5;
        drag_start(&ctx, &mut value);
        drag_frame(&ctx, vec![key(egui::Key::Enter)], &mut value, false);
        assert_eq!(value, 42);
        for _ in 0..3 {
            drag_frame(&ctx, vec![], &mut value, false);
            assert_eq!(value, 42);
        }
    }
}
