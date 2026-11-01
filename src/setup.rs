//! ワークスペースの追加と削除（画面から呼ぶ。Keychain と設定ファイルを書き換える）
use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};

use crate::config;
use crate::credentials::Token;
use crate::l10n;
use crate::slack::Client;
use crate::state::Store;

/// token を auth.test で確かめ、Keychain と設定ファイルに足す。足したワークスペースの Slack 上の名前を返す
pub fn add(token: &str) -> Result<String> {
    let token = token.trim();
    if !token.starts_with("xoxp-") {
        bail!(l10n::text(l10n::NOT_USER_TOKEN));
    }
    let auth = Client::new(Token::new(token.to_owned()))?.auth_test()?;
    let name = name_from_url(&auth.url).with_context(|| l10n::text(l10n::NO_WORKSPACE_URL))?;
    let names: Vec<String> = config::Config::load()?
        .workspace
        .into_iter()
        .map(|workspace| workspace.name)
        .collect();
    // 取得のたびに残している team_id と比べ、別の name で同じワークスペースを二重に足さない
    let team_ids = Store::open()?
        .load_cache()?
        .map(|cache| cache.team_ids)
        .unwrap_or_default();
    if already_added(&names, &team_ids, &name, &auth.team_id) {
        bail!(l10n::format(l10n::ALREADY_ADDED, &[&auth.team]));
    }
    Token::new(token.to_owned()).save(&name)?;
    config::add_workspace(&name)?;
    Ok(auth.team)
}

/// 足そうとしているワークスペース（name・team_id）が、すでにある項目と同じか。
/// name が同じか、すでにある項目の team_id（取得できたものだけ分かる）が同じなら、同じワークスペース
fn already_added(
    names: &[String],
    team_ids: &BTreeMap<String, String>,
    name: &str,
    team_id: &str,
) -> bool {
    names.iter().any(|existing| {
        existing == name
            || (!team_id.is_empty() && team_ids.get(existing).is_some_and(|id| id == team_id))
    })
}

/// 設定ファイルと Keychain から name のワークスペースを外す
pub fn remove(name: &str) -> Result<()> {
    config::remove_workspace(name)?;
    Token::delete(name)
}

/// https://xxx.slack.com/ の xxx。設定の name と Keychain の account に使う
fn name_from_url(url: &str) -> Option<String> {
    let host = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()?;
    let name = host.split('.').next()?;
    (!name.is_empty() && host.contains('.')).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, id)| ((*name).to_owned(), (*id).to_owned()))
            .collect()
    }

    #[test]
    fn same_name_is_already_added() {
        assert!(already_added(&["home".into()], &ids(&[]), "home", "T1"));
    }

    #[test]
    fn same_team_under_another_name_is_already_added() {
        // 手で書いた home と、URL から決めた dominion525 が同じワークスペース
        assert!(already_added(
            &["home".into()],
            &ids(&[("home", "T1")]),
            "dominion525",
            "T1"
        ));
    }

    #[test]
    fn another_team_can_be_added() {
        assert!(!already_added(
            &["home".into()],
            &ids(&[("home", "T1")]),
            "work",
            "T2"
        ));
    }

    #[test]
    fn unknown_team_ids_do_not_match() {
        // まだ取得できていない項目の team_id は分からない。空の team_id 同士も同じとみなさない
        assert!(!already_added(&["home".into()], &ids(&[]), "work", "T2"));
        assert!(!already_added(&["home".into()], &ids(&[("home", "")]), "work", ""));
    }

    #[test]
    fn name_is_the_subdomain() {
        assert_eq!(name_from_url("https://home.slack.com/").as_deref(), Some("home"));
        assert_eq!(
            name_from_url("https://acme.enterprise.slack.com/").as_deref(),
            Some("acme")
        );
        assert_eq!(name_from_url(""), None);
    }
}
