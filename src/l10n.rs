//! 画面に出す文言の多言語化。macOS 標準の Localizable.strings を NSBundle から引く。
//! キーは英語の文言そのもの。訳が無い言語（英語）や、.app の外で動かしたときは、キーがそのまま出る
use objc2_foundation::{NSBundle, NSString};

pub const QUIT: &str = "Quit";
pub const CHECK_FOR_UPDATES: &str = "Check for Updates…";
pub const NO_CACHE: &str = "No cache";
pub const NOTHING: &str = "No mentions or DMs";
pub const NOT_FETCHED_YET: &str = "Not fetched yet";
pub const FETCHED_AT: &str = "Fetched %@";
pub const FETCH_FAILED: &str = "Fetch failed: %@";
pub const WORKSPACE_FETCH_FAILED: &str = "Fetch failed: %1$@ (%2$@)";
pub const LOGIN_TOGGLE_FAILED: &str = "Couldn't change Open at Login (%@)";
pub const NONE: &str = "None";
pub const MARK_ALL_AS_READ: &str = "Mark All as Read";
pub const FETCH_NOW: &str = "Fetch Now";
pub const FETCH_NOW_IN: &str = "Fetch Now (in %@s)";
pub const WAITING_TO_FETCH: &str = "Waiting to Fetch";
pub const FETCHING: &str = "Fetching…";
pub const OPEN_AT_LOGIN: &str = "Open at Login";
pub const OPEN_AT_LOGIN_NEEDS_APPROVAL: &str = "Open at Login (needs approval in System Settings)";
pub const AND_MORE: &str = "And %@ more";
pub const GROUP_DM: &str = "Group DM";
pub const NO_TEXT: &str = "(no text)";

// エラー・案内の文
pub const READ_FAILED: &str = "Couldn't read %@";
pub const INVALID_CONTENTS: &str = "The contents of %@ are invalid";
pub const NO_WORKSPACE: &str = "No [[workspace]] is configured";
pub const DUPLICATE_WORKSPACE: &str = "Workspace name %@ is duplicated";
pub const HOME_NOT_SET: &str = "HOME is not set";
pub const SECURITY_FAILED: &str = "Couldn't run the security command";
pub const KEYCHAIN_READ_FAILED: &str = "Couldn't read service=%1$@ account=%2$@ from the Keychain: %3$@";
pub const KEYCHAIN_NOT_UTF8: &str = "The value of account=%@ in the Keychain isn't UTF-8";
pub const CHANNEL_READ_STATE_FAILED: &str = "Couldn't read the read state of %1$@: %2$@";
pub const THREAD_READ_STATE_FAILED: &str = "Couldn't read the read state of thread %2$@ in %1$@: %3$@";
pub const HTTP_CLIENT_FAILED: &str = "Couldn't create the HTTP client";
pub const NO_THREAD_PARENT: &str = "conversations.replies didn't return the thread parent";
pub const CALL_FAILED: &str = "Couldn't call %@";
pub const HTTP_STATUS: &str = "%1$@ returned HTTP %2$@";
pub const RESPONSE_READ_FAILED: &str = "Couldn't read the response of %@";
pub const RESPONSE_PARSE_FAILED: &str = "Couldn't parse the response of %@";
pub const API_FAILED: &str = "%1$@ failed: %2$@";
pub const NO_SAVE_DIR: &str = "No directory to save to";
pub const CREATE_FAILED: &str = "Couldn't create %@";
pub const PERMISSION_FAILED: &str = "Couldn't set the permissions of %@";
pub const OPEN_FAILED: &str = "Couldn't open %@";
pub const WRITE_FAILED: &str = "Couldn't write to %@";
pub const REPLACE_FAILED: &str = "Couldn't replace %@";
pub const SOME_WORKSPACES_FAILED: &str = "Some workspaces failed to fetch";
pub const DISPLAY_NAME_FAILED: &str = "%1$@: couldn't look up the display name of %2$@: %3$@";
pub const SPARKLE_LOAD_FAILED: &str = "Couldn't load Sparkle.framework";
pub const OUTSIDE_APP_RESIDENT: &str = "Can't stay in the menu bar because atme was run outside atme.app";
pub const NOT_MAIN_THREAD: &str = "Not running on the main thread";
pub const NOTIFICATION_REQUEST_FAILED: &str = "Couldn't ask for notification permission: %@";
pub const NOTIFICATIONS_NOT_ALLOWED_SETTINGS: &str = "Notifications aren't allowed. Allow atme under Notifications in System Settings";
pub const OUTSIDE_APP_NOTIFY: &str = "Not sending notifications because atme was run outside atme.app";
pub const NOTIFICATIONS_NOT_ALLOWED_OPEN: &str = "Notifications aren't allowed. Open atme.app to allow them";
pub const NOTIFICATION_SETTINGS_UNREADABLE: &str = "Not sending notifications because the notification settings couldn't be read";
pub const NOTIFY_FAILED: &str = "Couldn't send a notification: %@";
pub const NOTIFICATION_NO_RESPONSE: &str = "No response to the notification request";
pub const NO_CACHE_RUN_FETCH: &str = "No cache. Run atme fetch";

/// コードで使うキーの一覧。すべて ja.lproj に訳があることをテストで確かめる
#[cfg_attr(not(test), allow(dead_code))]
pub const KEYS: &[&str] = &[
    QUIT,
    CHECK_FOR_UPDATES,
    NO_CACHE,
    NOTHING,
    NOT_FETCHED_YET,
    FETCHED_AT,
    FETCH_FAILED,
    WORKSPACE_FETCH_FAILED,
    LOGIN_TOGGLE_FAILED,
    NONE,
    MARK_ALL_AS_READ,
    FETCH_NOW,
    FETCH_NOW_IN,
    WAITING_TO_FETCH,
    FETCHING,
    OPEN_AT_LOGIN,
    OPEN_AT_LOGIN_NEEDS_APPROVAL,
    AND_MORE,
    GROUP_DM,
    NO_TEXT,
    READ_FAILED,
    INVALID_CONTENTS,
    NO_WORKSPACE,
    DUPLICATE_WORKSPACE,
    HOME_NOT_SET,
    SECURITY_FAILED,
    KEYCHAIN_READ_FAILED,
    KEYCHAIN_NOT_UTF8,
    CHANNEL_READ_STATE_FAILED,
    THREAD_READ_STATE_FAILED,
    HTTP_CLIENT_FAILED,
    NO_THREAD_PARENT,
    CALL_FAILED,
    HTTP_STATUS,
    RESPONSE_READ_FAILED,
    RESPONSE_PARSE_FAILED,
    API_FAILED,
    NO_SAVE_DIR,
    CREATE_FAILED,
    PERMISSION_FAILED,
    OPEN_FAILED,
    WRITE_FAILED,
    REPLACE_FAILED,
    SOME_WORKSPACES_FAILED,
    DISPLAY_NAME_FAILED,
    SPARKLE_LOAD_FAILED,
    OUTSIDE_APP_RESIDENT,
    NOT_MAIN_THREAD,
    NOTIFICATION_REQUEST_FAILED,
    NOTIFICATIONS_NOT_ALLOWED_SETTINGS,
    OUTSIDE_APP_NOTIFY,
    NOTIFICATIONS_NOT_ALLOWED_OPEN,
    NOTIFICATION_SETTINGS_UNREADABLE,
    NOTIFY_FAILED,
    NOTIFICATION_NO_RESPONSE,
    NO_CACHE_RUN_FETCH,
];

/// key の訳を、macOS が選んだ言語で引く
pub fn text(key: &str) -> String {
    lookup(&NSBundle::mainBundle(), key, None).unwrap_or_else(|| key.to_owned())
}

/// key の訳を引き、%@（順番を書くときは %1$@ など）に args を差し込む
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
    fn japanese_keeps_placeholders() {
        fn placeholders(text: &str) -> Vec<String> {
            let mut found: Vec<String> = text
                .match_indices('%')
                .map(|(at, _)| {
                    let rest = &text[at..];
                    let end = rest.find('@').map_or(1, |end| end + 1);
                    rest[..end].to_owned()
                })
                .collect();
            found.sort();
            found
        }
        let bundle = japanese();
        let differ: Vec<&str> = KEYS
            .iter()
            .copied()
            .filter(|key| {
                lookup(&bundle, key, None).is_some_and(|ja| placeholders(&ja) != placeholders(key))
            })
            .collect();
        assert_eq!(differ, Vec::<&str>::new());
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
