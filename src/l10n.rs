//! 画面に出す文言の多言語化。macOS 標準の Localizable.strings を NSBundle から引く。
//! キーは英語の文言そのもの。訳が無い言語（英語）や、.app の外で動かしたときは、キーがそのまま出る
use objc2_foundation::{NSBundle, NSString};

pub const QUIT: &str = "Quit";
pub const CHECK_FOR_UPDATES: &str = "Check for Updates…";

/// コードで使うキーの一覧。すべて ja.lproj に訳があることをテストで確かめる
#[cfg_attr(not(test), allow(dead_code))]
pub const KEYS: &[&str] = &[QUIT, CHECK_FOR_UPDATES];

/// key の訳を、macOS が選んだ言語で引く
pub fn text(key: &str) -> String {
    lookup(&NSBundle::mainBundle(), key, None).unwrap_or_else(|| key.to_owned())
}

/// key の訳を引き、%@（順番を書くときは %1$@ など）に args を差し込む
#[allow(dead_code)]
pub fn format(key: &str, args: &[&str]) -> String {
    fill(&text(key), args)
}

/// bundle から key の訳を引く。訳が無ければ None
fn lookup(bundle: &NSBundle, key: &str, table: Option<&str>) -> Option<String> {
    // 訳が無いときに返させる値。訳の文言にはまず出てこない文字列にしておく
    let missing = "\u{0}atme-missing\u{0}";
    let table = table.map(NSString::from_str);
    let value = bundle
        .localizedStringForKey_value_table(
            &NSString::from_str(key),
            Some(&NSString::from_str(missing)),
            table.as_deref(),
        )
        .to_string();
    (value != missing).then_some(value)
}

fn fill(template: &str, args: &[&str]) -> String {
    let mut text = template.to_owned();
    for (index, arg) in args.iter().enumerate() {
        text = text.replace(&format!("%{}$@", index + 1), arg);
    }
    for arg in args {
        match text.find("%@") {
            Some(at) => text.replace_range(at..at + 2, arg),
            None => break,
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// リポジトリの ja.lproj を、1 つのバンドルとして開く
    fn japanese() -> objc2::rc::Retained<NSBundle> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/ja.lproj");
        NSBundle::bundleWithPath(&NSString::from_str(path)).expect("ja.lproj を開けません")
    }

    #[test]
    fn every_key_has_japanese() {
        let bundle = japanese();
        let missing: Vec<&str> = KEYS
            .iter()
            .copied()
            .filter(|key| lookup(&bundle, key, None).is_none())
            .collect();
        assert_eq!(missing, Vec::<&str>::new());
    }

    #[test]
    fn looks_up_japanese() {
        assert_eq!(lookup(&japanese(), QUIT, None).as_deref(), Some("終了"));
    }

    #[test]
    fn missing_key_is_none() {
        assert_eq!(lookup(&japanese(), "No such key", None), None);
    }

    #[test]
    fn fills_arguments_in_order() {
        assert_eq!(fill("%@ and %@ more", &["A", "3"]), "A and 3 more");
    }

    #[test]
    fn fills_numbered_arguments() {
        assert_eq!(fill("%2$@ の %1$@", &["A", "B"]), "B の A");
    }
}
