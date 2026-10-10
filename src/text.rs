//! テキストオブジェクトの本文からタイトルを作る（本体と関係なく、cargo test で確かめる）。
//!
//! `unescape` と `segments` は TextReplace_H の `src/text.rs` を写したもの（直すときは両方を直す）。
//! 本体は改行を `\n` の 2 文字、`\` を `\\` で返す（実機で確認。`AI/docs/20261008_パラメータ編集_SDK実機確認.md`）。

use std::ops::Range;

/// 本体が返す形（`\n` = 改行、`\\` = `\`）を本文に戻す。ほかの `\` はそのまま
pub fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut it = raw.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek() {
                Some('n') => {
                    it.next();
                    out.push('\n');
                }
                Some('\\') => {
                    it.next();
                    out.push('\\');
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 本文を制御文字（true）と地の文（false）の範囲に分ける。
/// 制御文字は `<` から最初の `>` まで。ただし `<//…//>`（コメント）と `<?…?>`（スクリプト）はその終わりまで。閉じない `<` は地の文
pub fn segments(text: &str) -> Vec<(Range<usize>, bool)> {
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with('<') {
            let rest = &text[i..];
            let close = if rest.starts_with("<//") {
                rest[3..].find("//>").map(|p| 3 + p + 3)
            } else if rest.starts_with("<?") {
                rest[2..].find("?>").map(|p| 2 + p + 2)
            } else {
                rest.find('>').map(|p| p + 1)
            };
            if let Some(len) = close {
                if plain_start < i {
                    out.push((plain_start..i, false));
                }
                out.push((i..i + len, true));
                i += len;
                plain_start = i;
                continue;
            }
        }
        i += text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    if plain_start < text.len() {
        out.push((plain_start..text.len(), false));
    }
    out
}

/// 制御文字を除いた地の文
pub fn plain(text: &str) -> String {
    segments(text).into_iter().filter(|(_, ctl)| !ctl).map(|(r, _)| &text[r]).collect()
}

/// 本体から読んだ本文（保存形式）から、タイトルにする 1 行を作る。
/// 制御文字を除いてから、空でない最初の行を前後の空白を落として返す
pub fn title_from_text(raw: &str) -> String {
    let body = plain(&unescape(raw));
    body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_control() {
        assert_eq!(title_from_text("<#ff0000>本編<#>"), "本編");
        assert_eq!(title_from_text("<s48,源ノ角ゴシック>第 1 章<s>\\n2 行目"), "第 1 章");
        assert_eq!(title_from_text("<//メモ//>オープニング"), "オープニング");
        assert_eq!(title_from_text("<?mes(\"x\")?>まとめ"), "まとめ");
    }

    #[test]
    fn first_nonempty_line() {
        assert_eq!(title_from_text("\\n  \\n  本編  \\nあと"), "本編");
        assert_eq!(title_from_text("<#ff0000><#>\\n次の行"), "次の行");
        assert_eq!(title_from_text(""), "");
    }

    #[test]
    fn keeps_unclosed_and_backslash() {
        assert_eq!(title_from_text("a < b"), "a < b");
        assert_eq!(title_from_text("C:\\\\path"), "C:\\path");
    }
}
