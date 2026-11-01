use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{bail, Result};
use chrono::{Local, NaiveDate, TimeDelta};

use crate::config::{Config, Workspace};
use crate::credentials::Token;
use crate::display;
use crate::l10n;
use crate::notify;
use crate::read;
use crate::slack::{Client, Message};
use crate::state::{ts_seconds, Cache, Item, Seen, Store};

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
    let config = Config::load()?;
    let store = Store::open()?;
    let previous_seen = store.load_seen()?;
    let previous_cache = store.load_cache()?.unwrap_or_default();
    let mut users = store.load_users()?.unwrap_or_default();
    let now = Local::now();
    let after = after_date(now.date_naive(), config.days);

    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut teams = previous_cache.teams.clone();
    for workspace in &config.workspace {
        let names = users.entry(workspace.name.clone()).or_default();
        let previous: Vec<Item> = previous_cache
            .items
            .iter()
            .filter(|item| item.workspace == workspace.name)
            .cloned()
            .collect();
        match fetch_workspace(workspace, &after, names, previous) {
            Ok((team, workspace_items)) => {
                errors.push(None);
                teams.insert(workspace.name.clone(), team);
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
    notify::notify(&new_items, &users, &teams);

    store.save_seen(&seen)?;
    store.save_cache(&Cache {
        fetched_at: fetched_at(
            previous_cache.fetched_at,
            now.timestamp(),
            reports.iter().any(|report| report.result.is_ok()),
        ),
        items,
        teams,
    })?;
    store.save_users(&users)?;
    Ok(reports)
}

fn fetch_workspace(
    workspace: &Workspace,
    after: &str,
    names: &mut BTreeMap<String, String>,
    previous: Vec<Item>,
) -> Result<(String, Vec<Item>)> {
    let client = Client::new(Token::load(workspace)?)?;
    let auth = client.auth_test()?;
    let me = auth.user_id;
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
        auth.team,
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
