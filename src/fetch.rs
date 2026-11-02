use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{bail, Result};
use chrono::{NaiveDate, TimeDelta};

use crate::config::{Config, Workspace};
use crate::display;
use crate::env::Env;
use crate::l10n;
use crate::read;
use crate::slack::{AuthTest, Client, Message};
use crate::state::{ts_seconds, Cache, Item, Seen};

const SECONDS_PER_DAY: i64 = 86_400;

#[derive(Debug, PartialEq)]
pub struct Report {
    /// 表示に使うワークスペース名（Slack 上の名前。まだ取得できていなければ設定の name）
    pub workspace: String,
    pub result: std::result::Result<Counts, String>,
}

#[derive(Debug, PartialEq)]
pub struct Counts {
    pub total: usize,
    pub new: usize,
}

pub fn run() -> Result<()> {
    let mut failed = false;
    for report in fetch_once()? {
        match report.result {
            Ok(counts) => println!(
                "{}",
                l10n::format(
                    l10n::FETCH_COUNTS,
                    &[
                        &report.workspace,
                        &counts.total.to_string(),
                        &counts.new.to_string(),
                    ],
                )
            ),
            Err(error) => {
                eprintln!("{}: {error}", report.workspace);
                failed = true;
            }
        }
    }
    if failed {
        bail!(l10n::text(l10n::SOME_WORKSPACES_FAILED));
    }
    Ok(())
}

pub fn fetch_once() -> Result<Vec<Report>> {
    fetch_with(&Env::system()?)
}

/// env の外の世界で、すべてのワークスペースを取得して、通知と保存まで行う
pub fn fetch_with(env: &Env) -> Result<Vec<Report>> {
    let config = Config::load_from(&env.config)?;
    let store = &env.store;
    let previous_seen = store.load_seen()?;
    let previous_cache = store.load_cache()?.unwrap_or_default();
    let mut users = store.load_users()?.unwrap_or_default();
    let now = env.now;
    let after = after_date(now.date_naive(), config.days);

    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut teams = previous_cache.teams.clone();
    let mut team_ids = previous_cache.team_ids.clone();
    for workspace in &config.workspace {
        let names = users.entry(workspace.name.clone()).or_default();
        let previous: Vec<Item> = previous_cache
            .items
            .iter()
            .filter(|item| item.workspace == workspace.name)
            .cloned()
            .collect();
        match fetch_workspace(env, workspace, &after, names, previous) {
            Ok((auth, workspace_items)) => {
                errors.push(None);
                teams.insert(workspace.name.clone(), auth.team);
                if !auth.team_id.is_empty() {
                    team_ids.insert(workspace.name.clone(), auth.team_id);
                }
                items.extend(workspace_items);
            }
            Err(error) => {
                errors.push(Some(format!("{error:#}")));
                items.extend(
                    previous_cache
                        .items
                        .iter()
                        .filter(|item| item.workspace == workspace.name)
                        .cloned(),
                );
            }
        }
    }
    // 外したワークスペースの名前の対応と表示名は残さない
    let names: Vec<&str> = config.workspace.iter().map(|workspace| workspace.name.as_str()).collect();
    keep_workspaces(&mut teams, &names);
    keep_workspaces(&mut team_ids, &names);
    keep_workspaces(&mut users, &names);
    items.sort_by_key(|item| std::cmp::Reverse((ts_seconds(&item.ts), item.ts.clone())));

    let cutoff = now.timestamp() - i64::from(config.days) * SECONDS_PER_DAY;
    let (new_items, seen) = diff(previous_seen.as_ref(), &items, cutoff);
    let reports: Vec<Report> = config
        .workspace
        .iter()
        .zip(errors)
        .map(|(workspace, error)| Report {
            workspace: display::workspace_name(&teams, &workspace.name).to_owned(),
            result: match error {
                Some(error) => Err(error),
                None => Ok(Counts {
                    total: items
                        .iter()
                        .filter(|item| item.workspace == workspace.name)
                        .count(),
                    new: new_items
                        .iter()
                        .filter(|item| item.workspace == workspace.name)
                        .count(),
                }),
            },
        })
        .collect();
    (env.notify)(&new_items, &users, &teams);

    store.save_seen(&seen)?;
    store.save_cache(&Cache {
        fetched_at: fetched_at(
            previous_cache.fetched_at,
            now.timestamp(),
            reports.iter().any(|report| report.result.is_ok()),
        ),
        items,
        teams,
        team_ids,
    })?;
    store.save_users(&users)?;
    Ok(reports)
}

fn fetch_workspace(
    env: &Env,
    workspace: &Workspace,
    after: &str,
    names: &mut BTreeMap<String, String>,
    previous: Vec<Item>,
) -> Result<(AuthTest, Vec<Item>)> {
    let client = Client::with_base((env.token)(workspace)?, env.api.clone())?;
    let auth = client.auth_test()?;
    let me = auth.user_id.clone();
    let mut messages = client.search_messages(&format!("<@{me}> after:{after}"))?;
    messages.extend(client.search_messages(&format!("to:me after:{after}"))?);
    let messages = select(messages, &me);

    for id in user_ids(&messages) {
        if names.contains_key(&id) {
            continue;
        }
        match client.users_info(&id) {
            Ok(user) => {
                names.insert(id, user.label().to_owned());
            }
            Err(error) => {
                eprintln!(
                    "{}",
                    l10n::format(
                        l10n::DISPLAY_NAME_FAILED,
                        &[&workspace.name, &id, &format!("{error:#}")],
                    )
                )
            }
        }
    }
    let fresh = messages
        .into_iter()
        .map(|message| to_item(&workspace.name, message, names))
        .collect();
    Ok((
        auth,
        read::unread_only(merge(fresh, previous), &client),
    ))
}

/// ワークスペースの名前をキーにした対応から、names に無いものを外す
fn keep_workspaces<V>(map: &mut BTreeMap<String, V>, names: &[&str]) {
    map.retain(|name, _| names.contains(&name.as_str()));
}

/// キャッシュに残す取得時刻。すべてのワークスペースで失敗したときは、前回の時刻のままにする
fn fetched_at(previous: i64, now: i64, any_succeeded: bool) -> i64 {
    if any_succeeded {
        now
    } else {
        previous
    }
}

fn after_date(today: NaiveDate, days: u32) -> String {
    (today - TimeDelta::days(i64::from(days)))
        .format("%Y-%m-%d")
        .to_string()
}

fn select(messages: Vec<Message>, me: &str) -> Vec<Message> {
    let mut keys = HashSet::new();
    messages
        .into_iter()
        .filter(|message| message.user.as_deref() != Some(me))
        .filter(|message| keys.insert((message.channel.id.clone(), message.ts.clone())))
        .collect()
}

fn user_ids(messages: &[Message]) -> BTreeSet<String> {
    messages
        .iter()
        .flat_map(|message| {
            let peer = message.channel.is_im.then(|| message.channel.name.clone());
            message
                .user
                .clone()
                .into_iter()
                .chain(peer)
                .chain(mentioned_ids(&message.text))
        })
        .collect()
}

fn mentioned_ids(text: &str) -> Vec<String> {
    text.split("<@")
        .skip(1)
        .filter_map(|rest| Some(rest[..rest.find(['>', '|'])?].to_owned()))
        .collect()
}

fn to_item(workspace: &str, message: Message, names: &BTreeMap<String, String>) -> Item {
    let peer_name = if message.channel.is_im {
        names.get(&message.channel.name).cloned()
    } else {
        None
    };
    let user_name = message
        .user
        .as_ref()
        .and_then(|user| names.get(user).cloned())
        .or(message.username);
    Item {
        workspace: workspace.to_owned(),
        channel_id: message.channel.id,
        channel_name: message.channel.name,
        is_im: message.channel.is_im,
        is_mpim: message.channel.is_mpim,
        peer_name,
        user: message.user,
        user_name,
        ts: message.ts,
        text: message.text,
        thread_ts: thread_ts(&message.permalink),
        permalink: message.permalink,
    }
}

/// スレッドの中のメッセージの permalink には `?thread_ts=…` が付く
fn thread_ts(permalink: &str) -> Option<String> {
    let (_, query) = permalink.split_once('?')?;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("thread_ts="))
        .map(str::to_owned)
}

/// 今回の検索結果に、前回の未読のうち今回の結果に無いもの（取得の範囲から外れたものなど）を足す
fn merge(fresh: Vec<Item>, previous: Vec<Item>) -> Vec<Item> {
    let keys: BTreeSet<_> = fresh.iter().map(Item::key).collect();
    let mut items = fresh;
    items.extend(
        previous
            .into_iter()
            .filter(|item| !keys.contains(&item.key())),
    );
    items
}

fn diff(previous: Option<&Seen>, items: &[Item], cutoff: i64) -> (Vec<Item>, Seen) {
    let new = match previous {
        Some(seen) => items
            .iter()
            .filter(|item| !seen.contains(&item.key()))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    // 一覧に残っている（未読の）項目は、日数に関係なく残す。消すとまた新着として通知してしまう
    let current: BTreeSet<_> = items.iter().map(Item::key).collect();
    let seen = previous
        .into_iter()
        .flatten()
        .cloned()
        .chain(current.iter().cloned())
        .filter(|key| ts_seconds(&key.ts) >= cutoff || current.contains(key))
        .collect();
    (new, seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::Channel;
    use crate::state::SeenKey;

    fn message(channel: &str, user: Option<&str>, ts: &str) -> Message {
        Message {
            channel: Channel {
                id: channel.into(),
                name: format!("name-{channel}"),
                is_im: false,
                is_mpim: false,
            },
            user: user.map(Into::into),
            username: user.map(|user| format!("handle-{user}")),
            ts: ts.into(),
            text: String::new(),
            permalink: String::new(),
        }
    }

    fn item(channel: &str, ts: &str) -> Item {
        to_item("home", message(channel, Some("U2"), ts), &BTreeMap::new())
    }

    fn key(channel: &str, ts: &str) -> SeenKey {
        SeenKey {
            workspace: "home".into(),
            channel: channel.into(),
            ts: ts.into(),
        }
    }

    #[test]
    fn keeps_only_configured_workspaces() {
        let mut teams = BTreeMap::from([
            ("home".to_owned(), "Home".to_owned()),
            ("old".to_owned(), "Old".to_owned()),
        ]);

        keep_workspaces(&mut teams, &["home", "work"]);

        assert_eq!(teams, BTreeMap::from([("home".to_owned(), "Home".to_owned())]));
    }

    #[test]
    fn keeps_fetched_at_when_every_workspace_failed() {
        assert_eq!(fetched_at(100, 200, true), 200);
        assert_eq!(fetched_at(100, 200, false), 100);
    }

    #[test]
    fn after_date_goes_back_days() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        assert_eq!(after_date(today, 7), "2026-10-21");
        let today = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        assert_eq!(after_date(today, 7), "2026-10-27");
    }

    #[test]
    fn user_ids_include_senders_peers_and_mentions() {
        let mut dm = message("D1", Some("U2"), "1.0");
        dm.channel.is_im = true;
        dm.channel.name = "U3".into();
        dm.text = "<@U1> と <@U4|someone> へ。閉じない <@U5".into();
        let bot = message("C1", None, "2.0");

        let ids = user_ids(&[dm, bot]);

        assert_eq!(ids, ["U1", "U2", "U3", "U4"].map(String::from).into());
    }

    #[test]
    fn select_removes_duplicates_and_own_messages() {
        let messages = vec![
            message("C1", Some("U2"), "100.000001"),
            message("C1", Some("U2"), "100.000001"),
            message("C1", Some("U1"), "100.000002"),
            message("C2", Some("U2"), "100.000001"),
            message("C3", None, "100.000003"),
        ];

        let selected = select(messages, "U1");

        let keys: Vec<_> = selected
            .iter()
            .map(|message| (message.channel.id.as_str(), message.ts.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                ("C1", "100.000001"),
                ("C2", "100.000001"),
                ("C3", "100.000003")
            ]
        );
    }

    #[test]
    fn to_item_resolves_names() {
        let mut dm = message("D1", Some("U2"), "100.000001");
        dm.channel.is_im = true;
        dm.channel.name = "U2".into();
        let names = BTreeMap::from([("U2".to_owned(), "相手".to_owned())]);

        let resolved = to_item("home", dm, &names);
        assert_eq!(resolved.peer_name.as_deref(), Some("相手"));
        assert_eq!(resolved.user_name.as_deref(), Some("相手"));

        let unresolved = to_item("home", message("C1", Some("U3"), "100.000002"), &names);
        assert_eq!(unresolved.peer_name, None);
        assert_eq!(unresolved.user_name.as_deref(), Some("handle-U3"));
    }

    #[test]
    fn to_item_keeps_the_thread_parent_from_the_permalink() {
        let mut reply = message("C1", Some("U3"), "200.000002");
        reply.permalink =
            "https://home.slack.com/archives/C1/p200000002?thread_ts=100.000001&cid=C1".into();

        let item = to_item("home", reply, &BTreeMap::new());

        assert_eq!(item.thread_ts.as_deref(), Some("100.000001"));
        assert_eq!(item.reply_parent(), Some("100.000001"));
        assert_eq!(
            to_item("home", message("C1", Some("U3"), "300.000003"), &BTreeMap::new()).thread_ts,
            None
        );
    }

    #[test]
    fn first_run_reports_nothing_and_records_all() {
        let items = [item("C1", "1000.000001"), item("C2", "1000.000002")];

        let (new, seen) = diff(None, &items, 0);

        assert!(new.is_empty());
        assert_eq!(
            seen,
            [key("C1", "1000.000001"), key("C2", "1000.000002")].into()
        );
    }

    #[test]
    fn reports_only_unseen_items() {
        let previous: Seen = [key("C1", "1000.000001")].into();
        let items = [item("C1", "1000.000001"), item("C2", "1000.000002")];

        let (new, seen) = diff(Some(&previous), &items, 0);

        assert_eq!(new, [item("C2", "1000.000002")]);
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn drops_seen_keys_older_than_cutoff() {
        let previous: Seen = [key("C1", "999.000001"), key("C1", "1000.000001")].into();

        let (_, seen) = diff(Some(&previous), &[], 1000);

        assert_eq!(seen, [key("C1", "1000.000001")].into());
    }

    #[test]
    fn keeps_seen_keys_of_items_still_listed() {
        let previous: Seen = [key("C1", "999.000001")].into();
        let items = [item("C1", "999.000001")];

        let (new, seen) = diff(Some(&previous), &items, 1000);

        assert!(new.is_empty());
        assert_eq!(seen, [key("C1", "999.000001")].into());
    }

    #[test]
    fn reads_thread_ts_from_permalink() {
        assert_eq!(
            thread_ts("https://x.slack.com/archives/C1/p1000000002?thread_ts=1000.000001&cid=C1")
                .as_deref(),
            Some("1000.000001")
        );
        assert_eq!(
            thread_ts("https://x.slack.com/archives/C1/p1000000002"),
            None
        );
        assert_eq!(thread_ts("https://x.slack.com/archives/C1/p1?cid=C1"), None);
    }

    #[test]
    fn merge_prefers_fresh_and_keeps_previous_unread() {
        let mut fresh_version = item("C1", "1000.000001");
        fresh_version.text = "新しい本文".into();
        let fresh = vec![fresh_version];
        let previous = vec![item("C1", "1000.000001"), item("C1", "900.000001")];

        let merged = merge(fresh, previous);

        let keys: Vec<_> = merged
            .iter()
            .map(|item| (item.ts.as_str(), item.text.as_str()))
            .collect();
        assert_eq!(keys, [("1000.000001", "新しい本文"), ("900.000001", "")]);
    }
}

/// 取得の流れ全体のテスト。架空の Slack（モックサーバー）と一時的なフォルダで、取得から通知と保存までを通す
#[cfg(test)]
mod flow_tests {
    use mockito::{Matcher, Server, ServerGuard};

    use super::*;
    use crate::env::fake;

    /// メンションのメッセージ（#general、佐藤さんから）と、DM（田中さんから）の ts
    const MENTION: &str = "1793502000.000100";
    const DM: &str = "1793505600.000200";

    struct World {
        world: fake::World,
        server: ServerGuard,
    }

    impl std::ops::Deref for World {
        type Target = fake::World;
        fn deref(&self) -> &fake::World {
            &self.world
        }
    }

    /// config を書いた設定ファイルと、空の状態のフォルダ。token は home にだけある
    fn world(config: &str) -> World {
        let server = Server::new();
        let world = fake::world(config, server.url());
        world
            .tokens
            .borrow_mut()
            .insert("home".into(), "xoxp-home".into());
        World { world, server }
    }

    fn respond(server: &mut ServerGuard, method: &str, params: &[(&str, &str)], body: &str) {
        let body_matchers = params
            .iter()
            .map(|(key, value)| Matcher::UrlEncoded((*key).into(), (*value).into()))
            .collect();
        server
            .mock("POST", format!("/{method}").as_str())
            .match_header("authorization", "Bearer xoxp-home")
            .match_body(Matcher::AllOf(body_matchers))
            .with_body(body)
            .create();
    }

    /// home の Slack：自分は U0ME。#general でのメンション（佐藤さん）と、田中さんからの DM が未読
    fn slack_for_home(server: &mut ServerGuard, general_last_read: &str) {
        respond(
            server,
            "auth.test",
            &[],
            r#"{"ok":true,"user_id":"U0ME","team":"Home","team_id":"T1","url":"https://home.slack.com/"}"#,
        );
        respond(
            server,
            "search.messages",
            &[("query", "<@U0ME> after:2026-10-25")],
            &format!(
                r#"{{"ok":true,"messages":{{"paging":{{"pages":1}},"matches":[
                    {{"channel":{{"id":"C1","name":"general"}},"user":"U1","ts":"{MENTION}","text":"<@U0ME> 確認をお願いします","permalink":"https://home.slack.com/archives/C1/p1"}}
                ]}}}}"#
            ),
        );
        respond(
            server,
            "search.messages",
            &[("query", "to:me after:2026-10-25")],
            &format!(
                r#"{{"ok":true,"messages":{{"paging":{{"pages":1}},"matches":[
                    {{"channel":{{"id":"D1","name":"U2","is_im":true}},"user":"U2","ts":"{DM}","text":"明日の件です","permalink":"https://home.slack.com/archives/D1/p2"}}
                ]}}}}"#
            ),
        );
        for (id, name) in [("U0ME", "鈴木"), ("U1", "佐藤"), ("U2", "田中")] {
            respond(
                server,
                "users.info",
                &[("user", id)],
                &format!(r#"{{"ok":true,"user":{{"name":"{id}","profile":{{"display_name":"{name}"}}}}}}"#),
            );
        }
        respond(
            server,
            "conversations.info",
            &[("channel", "C1")],
            &format!(r#"{{"ok":true,"channel":{{"last_read":"{general_last_read}"}}}}"#),
        );
        respond(
            server,
            "conversations.info",
            &[("channel", "D1")],
            r#"{"ok":true,"channel":{"last_read":"0.000000"}}"#,
        );
    }

    const HOME: &str = "[[workspace]]\nname = \"home\"\n";

    fn cached_ts(world: &World) -> Vec<String> {
        let cache = world.env.store.load_cache().unwrap().unwrap();
        cache.items.into_iter().map(|item| item.ts).collect()
    }

    #[test]
    fn notifies_new_unread_messages_and_saves_them() {
        let mut world = world(HOME);
        slack_for_home(&mut world.server, "0.000000");
        // 前に取得したことがある（通知済みの記録はあるが空）
        world.env.store.save_seen(&Seen::new()).unwrap();

        let reports = fetch_with(&world.env).unwrap();

        assert_eq!(
            reports,
            [Report {
                workspace: "Home".into(),
                result: Ok(Counts { total: 2, new: 2 }),
            }]
        );
        assert_eq!(*world.notified.borrow(), [DM, MENTION]);
        assert_eq!(cached_ts(&world), [DM, MENTION]);
        let cache = world.env.store.load_cache().unwrap().unwrap();
        assert_eq!(cache.fetched_at, world.env.now.timestamp());
        assert_eq!(cache.teams["home"], "Home");
        assert_eq!(cache.team_ids["home"], "T1");
        let dm = &cache.items[0];
        assert_eq!((dm.peer_name.as_deref(), dm.user_name.as_deref()), (Some("田中"), Some("田中")));
        assert_eq!(world.env.store.load_users().unwrap().unwrap()["home"]["U1"], "佐藤");
        assert_eq!(world.env.store.load_seen().unwrap().unwrap().len(), 2);
    }

    #[test]
    fn notifications_use_the_slack_name_of_a_workspace_fetched_for_the_first_time() {
        // home はまだ 1 度も取得していないので、前のキャッシュに Slack 上の名前（Home）は無い
        let mut world = world(HOME);
        slack_for_home(&mut world.server, "0.000000");
        world.env.store.save_seen(&Seen::new()).unwrap();

        fetch_with(&world.env).unwrap();

        assert_eq!(*world.titles.borrow(), ["Home / DM 田中", "Home / #general"]);
    }

    #[test]
    fn first_fetch_saves_without_notifying() {
        // 初めての取得では、今ある未読をまとめて通知しない
        let mut world = world(HOME);
        slack_for_home(&mut world.server, "0.000000");

        fetch_with(&world.env).unwrap();

        assert!(world.notified.borrow().is_empty());
        assert_eq!(cached_ts(&world), [DM, MENTION]);
        assert_eq!(world.env.store.load_seen().unwrap().unwrap().len(), 2);
    }

    #[test]
    fn read_messages_are_left_out() {
        // #general は、メンションのところまで Slack で読んである
        let mut world = world(HOME);
        slack_for_home(&mut world.server, MENTION);
        world.env.store.save_seen(&Seen::new()).unwrap();

        fetch_with(&world.env).unwrap();

        assert_eq!(*world.notified.borrow(), [DM]);
        assert_eq!(cached_ts(&world), [DM]);
    }

    fn previous_item(workspace: &str, ts: &str) -> Item {
        Item {
            workspace: workspace.into(),
            channel_id: "C9".into(),
            channel_name: "random".into(),
            is_im: false,
            is_mpim: false,
            peer_name: None,
            user: None,
            user_name: None,
            ts: ts.into(),
            text: String::new(),
            permalink: format!("https://{workspace}.slack.com/archives/C9/p9"),
            thread_ts: None,
        }
    }

    /// #general のスレッド（親は PARENT）の返信でのメンション
    const PARENT: &str = "1793490000.000100";
    const REPLY: &str = "1793508000.000300";

    /// home の Slack：スレッドの返信でのメンションだけがある。#general は general_last_read まで、スレッドは thread_last_read まで読んである
    fn slack_with_reply(
        server: &mut ServerGuard,
        general_last_read: &str,
        subscribed: bool,
        thread_last_read: &str,
    ) {
        respond(
            server,
            "auth.test",
            &[],
            r#"{"ok":true,"user_id":"U0ME","team":"Home","team_id":"T1","url":"https://home.slack.com/"}"#,
        );
        respond(
            server,
            "search.messages",
            &[("query", "<@U0ME> after:2026-10-25")],
            &format!(
                r#"{{"ok":true,"messages":{{"paging":{{"pages":1}},"matches":[
                    {{"channel":{{"id":"C1","name":"general"}},"user":"U1","ts":"{REPLY}","text":"<@U0ME> スレッドで質問です","permalink":"https://home.slack.com/archives/C1/p1793508000000300?thread_ts={PARENT}&cid=C1"}}
                ]}}}}"#
            ),
        );
        respond(
            server,
            "search.messages",
            &[("query", "to:me after:2026-10-25")],
            r#"{"ok":true,"messages":{"paging":{"pages":1},"matches":[]}}"#,
        );
        for (id, name) in [("U0ME", "鈴木"), ("U1", "佐藤")] {
            respond(
                server,
                "users.info",
                &[("user", id)],
                &format!(r#"{{"ok":true,"user":{{"name":"{id}","profile":{{"display_name":"{name}"}}}}}}"#),
            );
        }
        respond(
            server,
            "conversations.info",
            &[("channel", "C1")],
            &format!(r#"{{"ok":true,"channel":{{"last_read":"{general_last_read}"}}}}"#),
        );
        respond(
            server,
            "conversations.replies",
            &[("channel", "C1"), ("ts", PARENT)],
            &format!(
                r#"{{"ok":true,"messages":[{{"subscribed":{subscribed},"last_read":"{thread_last_read}"}}]}}"#
            ),
        );
    }

    #[test]
    fn reply_is_judged_by_its_thread_not_the_channel() {
        // #general は返信より先まで読んであるが、スレッドはまだ読んでいない
        let mut world = world(HOME);
        slack_with_reply(&mut world.server, "1793509000.000000", true, PARENT);
        world.env.store.save_seen(&Seen::new()).unwrap();

        fetch_with(&world.env).unwrap();

        assert_eq!(cached_ts(&world), [REPLY]);
        assert_eq!(*world.notified.borrow(), [REPLY]);
    }

    #[test]
    fn reply_disappears_once_the_thread_is_read() {
        let mut world = world(HOME);
        slack_with_reply(&mut world.server, "0.000000", true, REPLY);

        fetch_with(&world.env).unwrap();

        assert!(cached_ts(&world).is_empty());
    }

    #[test]
    fn reply_in_a_thread_not_followed_is_left_out() {
        let mut world = world(HOME);
        slack_with_reply(&mut world.server, "0.000000", false, PARENT);

        fetch_with(&world.env).unwrap();

        assert!(cached_ts(&world).is_empty());
    }

    /// 検索の範囲から外れた、前回からの home の未読（#random）。通知済みの記録にも入っている
    const CARRIED: &str = "1793400000.000300";

    fn carry_over(world: &mut World, random_last_read: &str) {
        let carried = previous_item("home", CARRIED);
        world
            .env
            .store
            .save_seen(&[carried.key()].into())
            .unwrap();
        world
            .env
            .store
            .save_cache(&Cache {
                fetched_at: 100,
                items: vec![carried],
                teams: BTreeMap::new(),
                team_ids: BTreeMap::new(),
            })
            .unwrap();
        respond(
            &mut world.server,
            "conversations.info",
            &[("channel", "C9")],
            &format!(r#"{{"ok":true,"channel":{{"last_read":"{random_last_read}"}}}}"#),
        );
    }

    #[test]
    fn carried_over_unread_disappears_once_read_in_slack() {
        let mut world = world(HOME);
        slack_for_home(&mut world.server, "0.000000");
        carry_over(&mut world, CARRIED);

        fetch_with(&world.env).unwrap();

        assert_eq!(cached_ts(&world), [DM, MENTION]);
    }

    #[test]
    fn carried_over_unread_stays_until_read() {
        let mut world = world(HOME);
        slack_for_home(&mut world.server, "0.000000");
        carry_over(&mut world, "1793300000.000100");

        fetch_with(&world.env).unwrap();

        assert_eq!(cached_ts(&world), [DM, MENTION, CARRIED]);
        // 前回から残っているだけなので、もう一度は通知しない
        assert_eq!(*world.notified.borrow(), [DM, MENTION]);
    }

    #[test]
    fn a_failing_workspace_keeps_its_previous_items() {
        // work は token が読めずに失敗する。前回の work の未読（前回の取得で通知済み）は残す
        let mut world = world("[[workspace]]\nname = \"home\"\n\n[[workspace]]\nname = \"work\"\n");
        slack_for_home(&mut world.server, "0.000000");
        let carried = previous_item("work", "1793400000.000300");
        world.env.store.save_seen(&[carried.key()].into()).unwrap();
        world
            .env
            .store
            .save_cache(&Cache {
                fetched_at: 100,
                items: vec![carried],
                teams: BTreeMap::from([("work".to_owned(), "Work".to_owned())]),
                team_ids: BTreeMap::new(),
            })
            .unwrap();

        let reports = fetch_with(&world.env).unwrap();

        assert_eq!(reports[0].result, Ok(Counts { total: 2, new: 2 }));
        assert_eq!(
            reports[1],
            Report {
                workspace: "Work".into(),
                result: Err("no token for work".into()),
            }
        );
        assert_eq!(cached_ts(&world), [DM, MENTION, "1793400000.000300"]);
        // 失敗したワークスペースの持ち越しは、もう一度は通知しない
        assert_eq!(*world.notified.borrow(), [DM, MENTION]);
        let cache = world.env.store.load_cache().unwrap().unwrap();
        assert_eq!(cache.fetched_at, world.env.now.timestamp());
    }

    #[test]
    fn every_workspace_failing_keeps_the_previous_fetched_at() {
        let world = world("[[workspace]]\nname = \"work\"\n");
        world
            .env
            .store
            .save_cache(&Cache {
                fetched_at: 100,
                ..Cache::default()
            })
            .unwrap();

        let reports = fetch_with(&world.env).unwrap();

        assert!(reports[0].result.is_err());
        assert_eq!(world.env.store.load_cache().unwrap().unwrap().fetched_at, 100);
    }

    #[test]
    fn removed_workspace_is_forgotten() {
        let mut world = world(HOME);
        slack_for_home(&mut world.server, "0.000000");
        world
            .env
            .store
            .save_cache(&Cache {
                fetched_at: 100,
                items: vec![previous_item("old", "1793400000.000300")],
                teams: BTreeMap::from([("old".to_owned(), "Old".to_owned())]),
                team_ids: BTreeMap::from([("old".to_owned(), "T9".to_owned())]),
            })
            .unwrap();
        world
            .env
            .store
            .save_users(&BTreeMap::from([("old".to_owned(), BTreeMap::new())]))
            .unwrap();

        fetch_with(&world.env).unwrap();

        let cache = world.env.store.load_cache().unwrap().unwrap();
        assert_eq!(cached_ts(&world), [DM, MENTION]);
        assert!(!cache.teams.contains_key("old"));
        assert!(!cache.team_ids.contains_key("old"));
        assert!(!world.env.store.load_users().unwrap().unwrap().contains_key("old"));
    }

    #[test]
    fn broken_config_stops_before_touching_anything() {
        let world = world("days = ");

        let error = fetch_with(&world.env).unwrap_err();

        assert!(error.to_string().starts_with("The contents of "));
        assert!(world.env.store.load_cache().unwrap().is_none());
    }
}
