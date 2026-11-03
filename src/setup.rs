//! ワークスペースの追加と削除（画面から呼ぶ。Keychain と設定ファイルを書き換える）
use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use crate::config;
use crate::credentials::Token;
use crate::env::Env;
use crate::l10n;
use crate::slack::Client;

/// token を auth.test で確かめ、Keychain と設定ファイルに足す。足したワークスペースの Slack 上の名前を返す
pub fn add(token: &str) -> Result<String> {
    add_with(&Env::system()?, token)
}

/// env の外の世界で add を行う
pub fn add_with(env: &Env, token: &str) -> Result<String> {
    let token = token.trim();
    if !token.starts_with("xoxp-") {
        bail!(l10n::text(l10n::NOT_USER_TOKEN));
    }
    let auth = Client::with_base(Token::new(token.to_owned()), env.api.clone())?.auth_test()?;
    let name = name_from_url(&auth.url).with_context(|| l10n::text(l10n::NO_WORKSPACE_URL))?;
    let names: Vec<String> = config::Config::load_from(&env.config)?
        .workspace
        .into_iter()
        .map(|workspace| workspace.name)
        .collect();
    // 取得のたびに残している team_id と比べ、別の name で同じワークスペースを二重に足さない
    let team_ids = env
        .store
        .load_cache()?
        .map(|cache| cache.team_ids)
        .unwrap_or_default();
    if already_added(&names, &team_ids, &name, &auth.team_id) {
        bail!(l10n::format(l10n::ALREADY_ADDED, &[&auth.team]));
    }
    (env.save_token)(&Token::new(token.to_owned()), &name)?;
    config::add_workspace_at(&env.config, &name)?;
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
    remove_with(&Env::system()?, name)
}

/// env の外の世界で remove を行う
pub fn remove_with(env: &Env, name: &str) -> Result<()> {
    config::remove_workspace_at(&env.config, name)?;
    (env.delete_token)(name)
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
        assert!(!already_added(
            &["home".into()],
            &ids(&[("home", "")]),
            "work",
            ""
        ));
    }

    #[test]
    fn name_is_the_subdomain() {
        assert_eq!(
            name_from_url("https://home.slack.com/").as_deref(),
            Some("home")
        );
        assert_eq!(
            name_from_url("https://acme.enterprise.slack.com/").as_deref(),
            Some("acme")
        );
        assert_eq!(name_from_url(""), None);
    }
}

/// ワークスペースの追加と削除を、架空の Slack（モックサーバー）と偽の外の世界で通すテスト
#[cfg(test)]
mod flow_tests {
    use mockito::{Server, ServerGuard};

    use super::*;
    use crate::env::fake;
    use crate::state::Cache;

    const TOKEN: &str = "xoxp-acme";

    /// TOKEN で auth.test を呼ばれたら body を返す
    fn slack(body: &str) -> ServerGuard {
        let mut server = Server::new();
        server
            .mock("POST", "/auth.test")
            .match_header("authorization", format!("Bearer {TOKEN}").as_str())
            .with_body(body)
            .create();
        server
    }

    const ACME: &str = r#"{"ok":true,"user_id":"U1","team":"Acme","team_id":"T2","url":"https://acme.slack.com/"}"#;

    fn names(world: &fake::World) -> Vec<String> {
        config::Config::load_from(&world.env.config)
            .unwrap()
            .workspace
            .into_iter()
            .map(|workspace| workspace.name)
            .collect()
    }

    #[test]
    fn adds_the_workspace_and_saves_the_token() {
        let server = slack(ACME);
        let world = fake::world("", server.url());

        // 前後の空白や改行は、貼り付けたときに付いてくることがある
        let team = add_with(&world.env, &format!("  {TOKEN}\n")).unwrap();

        assert_eq!(team, "Acme");
        assert_eq!(names(&world), ["acme"]);
        assert_eq!(world.tokens.borrow()["acme"], TOKEN);
    }

    #[test]
    fn refuses_what_is_not_a_user_token() {
        let world = fake::world("", "http://127.0.0.1:9".into());

        let error = add_with(&world.env, "xoxb-bot-token").unwrap_err();

        assert_eq!(
            error.to_string(),
            "Paste a User OAuth Token that starts with xoxp-"
        );
        assert!(world.tokens.borrow().is_empty());
    }

    #[test]
    fn refuses_the_same_name_twice() {
        let server = slack(ACME);
        let world = fake::world("[[workspace]]\nname = \"acme\"\n", server.url());

        let error = add_with(&world.env, TOKEN).unwrap_err();

        assert_eq!(error.to_string(), "Acme has already been added");
        assert!(world.tokens.borrow().is_empty());
    }

    #[test]
    fn refuses_the_same_team_under_another_name() {
        // 手で書いた home が、取得のたびに残る team_id で Acme（T2）だと分かっている
        let server = slack(ACME);
        let world = fake::world("[[workspace]]\nname = \"home\"\n", server.url());
        world
            .env
            .store
            .save_cache(&Cache {
                team_ids: BTreeMap::from([("home".to_owned(), "T2".to_owned())]),
                ..Cache::default()
            })
            .unwrap();

        let error = add_with(&world.env, TOKEN).unwrap_err();

        assert_eq!(error.to_string(), "Acme has already been added");
        assert_eq!(names(&world), ["home"]);
    }

    #[test]
    fn token_rejected_by_slack_is_not_saved() {
        let server = slack(r#"{"ok":false,"error":"invalid_auth"}"#);
        let world = fake::world("", server.url());

        let error = add_with(&world.env, TOKEN).unwrap_err();

        assert_eq!(error.to_string(), "auth.test failed: invalid_auth");
        assert!(world.tokens.borrow().is_empty());
        assert!(names(&world).is_empty());
    }

    #[test]
    fn workspace_without_url_is_an_error() {
        let server = slack(r#"{"ok":true,"user_id":"U1","team":"Acme","team_id":"T2"}"#);
        let world = fake::world("", server.url());

        let error = add_with(&world.env, TOKEN).unwrap_err();

        assert_eq!(
            error.to_string(),
            "auth.test didn't return the workspace URL"
        );
    }

    #[test]
    fn removes_the_workspace_and_its_token() {
        let world = fake::world(
            "[[workspace]]\nname = \"acme\"\n\n[[workspace]]\nname = \"home\"\n",
            "http://127.0.0.1:9".into(),
        );
        world.tokens.borrow_mut().extend([
            ("acme".to_owned(), "xoxp-acme".to_owned()),
            ("home".to_owned(), "xoxp-home".to_owned()),
        ]);

        remove_with(&world.env, "acme").unwrap();

        assert_eq!(names(&world), ["home"]);
        assert_eq!(world.tokens.borrow().keys().collect::<Vec<_>>(), ["home"]);
    }
}
