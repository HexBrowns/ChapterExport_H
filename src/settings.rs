//! 書き出しの設定。プロジェクトごとに `.aup2` へ残す（`set_param_string` / `get_param_string`）。

use serde::{Deserialize, Serialize};

use crate::format::{Style, TimeForm, DEFAULT_TEMPLATE};

/// プロジェクトに残すときのキー
pub const PARAM_KEY: &str = "settings";

pub const DEFAULT_LAYER_NAME: &str = "チャプター";

/// チャプターの元
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Source {
    /// 本体のマーク（マークリスト）。メモをタイトルにする
    #[default]
    Mark,
    /// 指定したレイヤーに置いたオブジェクト。オブジェクト名（無ければテキストの本文の 1 行目）をタイトルにする
    Layer,
}

/// レイヤーの選び方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayerBy {
    #[default]
    Name,
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub source: Source,
    pub layer_by: LayerBy,
    pub layer_name: String,
    /// 1 始まり（画面の表示と同じ）
    pub layer_number: usize,
    pub style: Style,
    pub template: String,
    pub form: TimeForm,
    /// フレームの範囲選択があれば、その中だけを出し、範囲の先頭を 0:00 にする
    pub use_range: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            source: Source::Mark,
            layer_by: LayerBy::Name,
            layer_name: DEFAULT_LAYER_NAME.to_string(),
            layer_number: 1,
            style: Style::YouTube,
            template: DEFAULT_TEMPLATE.to_string(),
            form: TimeForm::MSS,
            use_range: true,
        }
    }
}

impl Settings {
    pub fn to_param(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// 読めなければ既定（壊れた値でプラグインを止めない）
    pub fn from_param(s: &str) -> Self {
        if s.trim().is_empty() {
            return Settings::default();
        }
        match serde_json::from_str(s) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("[ChapterExport_H] プロジェクトに残した設定を読めないので既定にしました: {e}");
                Settings::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings { source: Source::Layer, layer_number: 7, style: Style::Custom, template: "{index} {title}".into(), ..Default::default() };
        assert_eq!(Settings::from_param(&s.to_param()), s);
    }

    #[test]
    fn broken_or_partial() {
        assert_eq!(Settings::from_param(""), Settings::default());
        assert_eq!(Settings::from_param("{壊れた"), Settings::default());
        // 項目が足りなくても既定で埋める（版を上げて項目が増えたとき）
        let s = Settings::from_param(r#"{"source":"Layer"}"#);
        assert_eq!(s.source, Source::Layer);
        assert_eq!(s.layer_name, DEFAULT_LAYER_NAME);
    }
}
