//! atme 用の Slack App を作るための App Manifest（リポジトリ直下の slack-app-manifest.json）
use url::Url;

/// ビルドのときに埋め込む。リポジトリの manifest と、ボタンが開く manifest をずらさないため
const MANIFEST: &str = include_str!("../slack-app-manifest.json");

/// atme が使う user token のスコープと、それを要る API
#[cfg_attr(not(test), allow(dead_code))]
pub const USER_SCOPES: &[&str] = &[
    // search.messages（自分宛のメンションと DM を探す）
    "search:read",
    // users.info（送り主と DM の相手の表示名）
    "users:read",
    // conversations.info（チャンネルの既読の位置）
    "channels:read",
    "groups:read",
    "im:read",
    "mpim:read",
    // conversations.replies（スレッドの既読の位置）
    "channels:history",
    "groups:history",
    "im:history",
    "mpim:history",
];

/// manifest を入れた状態で、Slack App を作る画面を開くリンク
pub fn create_app_url() -> Url {
    // 改行や字下げを除いて、リンクを短くする
    let compact = serde_json::from_str::<serde_json::Value>(MANIFEST)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| MANIFEST.to_owned());
    Url::parse_with_params(
        "https://api.slack.com/apps",
        &[("new_app", "1"), ("manifest_json", compact.as_str())],
    )
    .expect("固定の URL は正しい")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn manifest() -> Value {
        serde_json::from_str(MANIFEST).expect("manifest の JSON が読めない")
    }

    #[test]
    fn requests_exactly_the_scopes_atme_uses() {
        let mut scopes: Vec<String> = manifest()["oauth_config"]["scopes"]["user"]
            .as_array()
            .expect("user のスコープが無い")
            .iter()
            .map(|scope| scope.as_str().unwrap().to_owned())
            .collect();
        scopes.sort();
        let mut expected: Vec<&str> = USER_SCOPES.to_vec();
        expected.sort();

        assert_eq!(scopes, expected);
    }

    #[test]
    fn has_no_bot_and_keeps_tokens_from_expiring() {
        let manifest = manifest();

        assert!(manifest["oauth_config"]["scopes"]["bot"].is_null());
        // ローテーションが有効だと token が 12 時間で切れ、atme が読めなくなる
        assert_eq!(manifest["settings"]["token_rotation_enabled"], false);
        assert_eq!(manifest["display_information"]["name"], "atme");
    }

    #[test]
    fn link_carries_the_whole_manifest() {
        let url = create_app_url();
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();

        assert_eq!(url.host_str(), Some("api.slack.com"));
        assert_eq!(url.path(), "/apps");
        assert_eq!(pairs[0], ("new_app".into(), "1".into()));
        assert_eq!(pairs[1].0, "manifest_json");
        assert_eq!(
            serde_json::from_str::<Value>(&pairs[1].1).unwrap(),
            manifest()
        );
    }
}
