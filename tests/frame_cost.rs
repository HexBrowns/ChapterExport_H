//! 実用の規模での時間の目安（ルール au2-plugin-checklist「実用の規模で確かめる」）。
//! チャプター候補 2000 件・タイトルが空のマーク 500 件・長いテキストの本文で、読み直し 1 回の組み立てと、一覧の 1 フレームを測る。
//! 時間は機械で変わるので、既定では走らせない:
//!   cargo test --release --test frame_cost -- --ignored --nocapture

use std::time::Instant;

use aviutl2_eframe::egui;
use chapter_export_h::format::{self, Chapter, Options, Span, Style, TimeForm};
use chapter_export_h::text;

fn chapters(n: usize, empty: usize) -> Vec<Chapter> {
    let mut v: Vec<Chapter> = (0..n)
        .map(|i| Chapter { frame: i * 45, title: format!("第 {i} 章 とても長いタイトルの例として百字近くまで伸ばした行 {}", "あ".repeat(60)), layer: None })
        .collect();
    v.extend((0..empty).map(|i| Chapter { frame: i * 45 + 7, title: String::new(), layer: None }));
    v
}

#[test]
#[ignore]
fn frame_cost_at_scale() {
    let opts = Options { style: Style::YouTube, template: String::new(), form: TimeForm::MSS };
    let v = chapters(2000, 500);
    let started = Instant::now();
    let out = format::build(&v, Span { start: 0, end: 107_999 }, 30, 1, &opts);
    println!("[INFO] 組み立て（候補 2500 件）: {:.2} ms / 全体の警告 {} 行", started.elapsed().as_secs_f64() * 1000.0, out.warnings.len());
    assert!(out.warnings.len() <= 4, "警告が件数でまとまっていない: {}", out.warnings.len());

    // 閉じない `<` を 5000 個含む 5 万字の本文（数式など）
    let body = "a<b ".repeat(5000) + &"本文".repeat(20000);
    let started = Instant::now();
    let t = text::title_from_text(&body);
    println!("[INFO] 本文 {} 字からタイトル: {:.2} ms（{} 字）", body.chars().count(), started.elapsed().as_secs_f64() * 1000.0, t.chars().count());

    // 一覧の 1 フレーム（gui.rs の render_rows と同じ組み立て。見えている行だけ）
    let ctx = egui::Context::default();
    let mut per = Vec::new();
    for i in 0..12 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))),
            events: vec![egui::Event::PointerMoved(egui::pos2(100.0 + i as f32, 300.0))],
            ..Default::default()
        };
        let started = Instant::now();
        let mut full = ctx.run_ui(raw, |ui| {
            let row_height = ui.spacing().interact_size.y;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_height, out.lines.len(), |ui, range| {
                for line in &out.lines[range] {
                    ui.horizontal(|ui| {
                        let _ = ui.small_button("移動").on_hover_text(format!("{} フレーム目へ", line.frame));
                        ui.monospace(&line.time);
                        for w in &line.warnings {
                            ui.colored_label(egui::Color32::YELLOW, format!("※ {w}"));
                        }
                        ui.add(egui::Label::new(&line.title).truncate());
                    });
                }
            });
        });
        full.textures_delta.clear();
        if i >= 2 {
            per.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let avg = per.iter().sum::<f64>() / per.len() as f64;
    let max = per.iter().cloned().fold(0.0, f64::max);
    println!("[INFO] 一覧の 1 フレーム（{} 行）: 平均 {avg:.2} ms / 最大 {max:.2} ms", out.lines.len());

    // 比較: v0.2.0 の組み立て（Grid で全行を毎フレーム作る）
    let ctx = egui::Context::default();
    let mut per = Vec::new();
    for i in 0..12 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))),
            events: vec![egui::Event::PointerMoved(egui::pos2(100.0 + i as f32, 300.0))],
            ..Default::default()
        };
        let started = Instant::now();
        let mut full = ctx.run_ui(raw, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("rows").striped(true).num_columns(3).show(ui, |ui| {
                    for line in &out.lines {
                        let _ = ui.small_button("移動").on_hover_text(format!("{} フレーム目へ", line.frame));
                        ui.monospace(&line.time);
                        ui.horizontal(|ui| {
                            ui.label(&line.title);
                            for w in &line.warnings {
                                ui.colored_label(egui::Color32::YELLOW, format!("※ {w}"));
                            }
                        });
                        ui.end_row();
                    }
                });
            });
        });
        full.textures_delta.clear();
        if i >= 2 {
            per.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let avg = per.iter().sum::<f64>() / per.len() as f64;
    let max = per.iter().cloned().fold(0.0, f64::max);
    println!("[INFO] 比較 v0.2.0 の一覧の 1 フレーム（{} 行・全行）: 平均 {avg:.2} ms / 最大 {max:.2} ms", out.lines.len());
}
