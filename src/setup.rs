//! ワークスペースの追加と削除（画面から呼ぶ。Keychain と設定ファイルを書き換える）
use anyhow::{bail, Context, Result};

use crate::config;
use crate::credentials::Token;
use crate::l10n;
use crate::slack::Client;

/// token を auth.test で確かめ、Keychain と設定ファイルに足す。足したワークスペースの Slack 上の名前を返す
pub fn add(token: &str) -> Result<String> {
    let token = token.trim();
    if !token.starts_with("xoxp-") {
        bail!(l10n::text(l10n::NOT_USER_TOKEN));
    }
    let auth = Client::new(Token::new(token.to_owned()))?.auth_test()?;
    let name = name_from_url(&auth.url).with_context(|| l10n::text(l10n::NO_WORKSPACE_URL))?;
    let existing = config::Config::load()
        .map(|config| config.workspace)
        .unwrap_or_default();
    if existing.iter().any(|workspace| workspace.name == name) {
        bail!(l10n::format(l10n::ALREADY_ADDED, &[&auth.team]));
    }
    Token::new(token.to_owned()).save(&name)?;
    config::add_workspace(&name)?;
    Ok(auth.team)
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
