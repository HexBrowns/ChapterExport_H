//! ChapterExport_H（チャプター書き出し）
//!
//! 本体のマーク（マークリストのメモ）か、指定したレイヤーに置いたオブジェクト（名前・テキストの本文の 1 行目）から、
//! YouTube・ニコニコ動画の概要欄に貼るチャプター（`0:00 タイトル` の並び）を作ってコピーする。
//! 仕様は `AI/specifications/20261009_ChapterExport_H_spec.md`（YMM4 のタイムスタンププラグインに当たる）。
//!
//! 読むのは `call_read_section` だけで、プロジェクトは書き換えない。例外は行の「移動」ボタン（今のフレームを動かす）で、
//! そのときだけ `call_edit_section` を呼ぶ。設定はプロジェクトごとに `.aup2` へ残す。
//!
//! 自分で起こすスレッドは無い。

pub mod format;
mod host_fonts;
mod gui;
mod read;
pub mod settings;
pub mod text;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{LazyLock, OnceLock};

use aviutl2::generic::{EventType, GlobalEditHandle};
use aviutl2::raw_window_handle::HasWindowHandle;
use aviutl2::AnyResult;
use aviutl2_eframe::egui;
use parking_lot::{Mutex, RwLock};

use settings::{Settings, PARAM_KEY};

pub const WINDOW_NAME: &str = "チャプター書き出し";

pub(crate) static EDIT_HANDLE: GlobalEditHandle = GlobalEditHandle::new();

/// ウィンドウの HWND（見えているかを調べるため）
static WINDOW_HWND: AtomicIsize = AtomicIsize::new(0);

/// 本体のイベントで立てる印。イベント用スレッドで立て、UI のスレッドで下ろす（ここから本体を読まない）
pub(crate) static OBJECT_DIRTY: AtomicBool = AtomicBool::new(true);

/// UI のスレッドを起こすための egui の Context（ウィンドウができたら入る）
pub(crate) static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();

/// 今のプロジェクトの設定（プロジェクトの読み込み・保存とウィンドウで共有する）
pub(crate) static SETTINGS: LazyLock<Mutex<Settings>> = LazyLock::new(|| Mutex::new(Settings::default()));
/// 設定を差し替えた回数（ウィンドウが読み込み直すかを知るため）
pub(crate) static SETTINGS_GEN: AtomicU64 = AtomicU64::new(0);

/// 開いているプロジェクトの `.aup2`（テキストを隣に書き込むため）。未保存なら None
pub(crate) static PROJECT: RwLock<Option<PathBuf>> = RwLock::new(None);

fn init_logging() {
    use aviutl2::tracing::Level;
    let level = if cfg!(debug_assertions) { Level::DEBUG } else { Level::INFO };
    let _ = aviutl2::tracing_subscriber::fmt()
        .with_max_level(level)
        .event_format(aviutl2::logger::AviUtl2Formatter)
        .with_writer(aviutl2::logger::AviUtl2LogWriter)
        .try_init();
}

/// 本体のイベント: 印を立てて UI を起こす。立っている間は起こし直さない
fn on_host_event() {
    if !OBJECT_DIRTY.swap(true, Ordering::AcqRel) {
        if let Some(ctx) = EGUI_CTX.get() {
            ctx.request_repaint();
        }
    }
}

/// ウィンドウが見えているか（本体のパネルに出ていなければ偽）。ハンドルが取れていなければ見えているとみなす
pub(crate) fn window_visible() -> bool {
    let hwnd = WINDOW_HWND.load(Ordering::Acquire);
    if hwnd == 0 {
        return true;
    }
    unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(windows::Win32::Foundation::HWND(hwnd as _)).as_bool() }
}

fn replace_settings(s: Settings) {
    *SETTINGS.lock() = s;
    SETTINGS_GEN.fetch_add(1, Ordering::AcqRel);
    on_host_event();
}

/// 初期化にかかった時間をログに 1 行出す（強化デバッグ。仕様 `AI/specifications/20261012_CurveEditor_H_キーフレーム速度_spec.md` の 7.0）。
/// `register` の始めに作り、`register` を抜けるとき（途中の `return` も含む）に new と register の時間を出す
struct InitTimeLog {
    new_time: std::time::Duration,
    register_start: std::time::Instant,
}

impl InitTimeLog {
    fn start(new_time: std::time::Duration) -> Self {
        Self { new_time, register_start: std::time::Instant::now() }
    }
}

impl Drop for InitTimeLog {
    fn drop(&mut self) {
        let register = self.register_start.elapsed();
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        aviutl2::tracing::info!(
            "ChapterExport_H: 初期化 {:.1} ms（new {:.1} ms・register {:.1} ms）",
            ms(self.new_time + register),
            ms(self.new_time),
            ms(register)
        );
    }
}

#[aviutl2::plugin(GenericPlugin)]
pub struct ChapterExportPlugin {
    window: Mutex<Option<aviutl2_eframe::EframeWindow>>,
    /// `new` にかかった時間（初期化の時間をログに出すため。`InitTimeLog`）
    init_new_time: std::time::Duration,
}

impl aviutl2::generic::GenericPlugin for ChapterExportPlugin {
    fn new(_info: aviutl2::AviUtl2Info) -> AnyResult<Self> {
        let init_t0 = std::time::Instant::now();
        init_logging();
        tracing::info!("ChapterExport_H v{} 初期化", env!("CARGO_PKG_VERSION"));
        Ok(Self { window: Mutex::new(None), init_new_time: init_t0.elapsed() })
    }

    fn plugin_info(&self) -> aviutl2::generic::GenericPluginTable {
        aviutl2::generic::GenericPluginTable {
            name: "ChapterExport_H".to_string(),
            information: format!("ChapterExport_H v{} - チャプター書き出し / by HexBrowns", env!("CARGO_PKG_VERSION")),
        }
    }

    fn register(&mut self, registry: &mut aviutl2::generic::HostAppHandle) {
        // 抜けるときに初期化の時間をログに出す（`InitTimeLog`）
        let _init_time = InitTimeLog::start(self.init_new_time);
        EDIT_HANDLE.init(registry.create_edit_handle());
        let window = match aviutl2_eframe::EframeWindow::new(WINDOW_NAME, move |cc, handle| Ok(Box::new(gui::ChapterApp::new(cc, handle)))) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("チャプター書き出しのウィンドウを作れませんでした: {e:#}");
                return;
            }
        };
        match window.handle() {
            Ok(handle) => {
                if let Err(e) = registry.register_window_client(WINDOW_NAME, &handle) {
                    tracing::error!("register_window_client 失敗: {e}");
                }
                if let Ok(raw) = handle.window_handle() {
                    if let aviutl2::raw_window_handle::RawWindowHandle::Win32(h) = raw.as_raw() {
                        WINDOW_HWND.store(h.hwnd.get(), Ordering::Release);
                    }
                }
            }
            Err(e) => tracing::error!("チャプター書き出しのウィンドウのハンドルを取れませんでした: {e:#}"),
        }
        *self.window.lock() = Some(window);

        // ウィンドウは表示メニューから開く（メニューから ShowWindow しても開かない。2026-10-10 にユーザーの指摘で外した）

        // 読み直す合図（フレームの移動は使わない。再生中に毎フレーム来る）
        for ev in [EventType::UpdateObject, EventType::ChangeEditScene] {
            registry.register_event_listener(ev, on_host_event);
        }
    }

    fn on_project_load(&mut self, project: &mut aviutl2::generic::ProjectFile) {
        *PROJECT.write() = project.get_path();
        let s = project.get_param_string(PARAM_KEY).map(|v| Settings::from_param(&v)).unwrap_or_default();
        replace_settings(s);
    }

    fn on_project_save(&mut self, project: &mut aviutl2::generic::ProjectFile) {
        if let Some(p) = project.get_path() {
            *PROJECT.write() = Some(p);
        }
        let v = SETTINGS.lock().to_param();
        if let Err(e) = project.set_param_string(PARAM_KEY, &v) {
            tracing::warn!("[ChapterExport_H] 設定をプロジェクトに残せませんでした: {e}");
        }
    }
}

// 本体が呼ぶ関数（GetCommonPluginTable など）を書き出す（ルール au2-rs-plugin「ビルド」）
aviutl2::register_generic_plugin!(ChapterExportPlugin);
