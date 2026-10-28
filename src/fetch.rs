use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{bail, Result};
use chrono::{Local, NaiveDate, TimeDelta};

use crate::config::{Config, Workspace};
use crate::credentials::Credentials;
use crate::slack::{Client, Message};
use crate::state::{Cache, Item, Seen, Store};

const SECONDS_PER_DAY: i64 = 86_400;

pub fn run() -> Result<()> {
    let config = Config::load()?;
    let store = Store::open()?;
    let previous_seen = store.load_seen()?;
    let previous_cache = store.load_cache()?.unwrap_or_default();
    let mut users = store.load_users()?.unwrap_or_default();
    let now = Local::now();
    let after = after_date(now.date_naive(), config.days);

    let mut items = Vec::new();
    let mut fetched = Vec::new();
    let mut failed = false;
    for workspace in &config.workspace {
        let names = users.entry(workspace.name.clone()).or_default();
        match fetch_workspace(workspace, &after, names) {
            Ok(workspace_items) => {
                fetched.push(workspace.name.as_str());
                items.extend(workspace_items);
            }
            Err(error) => {
                eprintln!("{}: {error:#}", workspace.name);
                failed = true;
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
    items.sort_by_key(|item| std::cmp::Reverse((ts_seconds(&item.ts), item.ts.clone())));

    let cutoff = now.timestamp() - i64::from(config.days) * SECONDS_PER_DAY;
    let (new_items, seen) = diff(previous_seen.as_ref(), &items, cutoff);
    for name in fetched {
        let total = items.iter().filter(|item| item.workspace == name).count();
        let new = new_items
            .iter()
            .filter(|item| item.workspace == name)
            .count();
        println!("{name}: {total} 件、新着 {new} 件");
    }

    store.save_seen(&seen)?;
    store.save_cache(&Cache {
        fetched_at: now.timestamp(),
        items,
    })?;
    store.save_users(&users)?;
    if failed {
        bail!("取得に失敗したワークスペースがあります");
    }
    Ok(())
}

fn fetch_workspace(
    workspace: &Workspace,
    after: &str,
    names: &mut BTreeMap<String, String>,
) -> Result<Vec<Item>> {
    let client = Client::new(workspace, Credentials::load(workspace)?)?;
    let me = client.auth_test()?.user_id;
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
                    "{}: {id} の表示名を引けませんでした: {error:#}",
                    workspace.name
                )
            }
        }
    }
    Ok(messages
        .into_iter()
        .map(|message| to_item(&workspace.name, message, names))
        .collect())
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
            message.user.clone().into_iter().chain(peer)
        })
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
        permalink: message.permalink,
    }
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
    let seen = previous
        .into_iter()
        .flatten()
        .cloned()
        .chain(items.iter().map(Item::key))
        .filter(|key| ts_seconds(&key.ts) >= cutoff)
        .collect();
    (new, seen)
}

fn ts_seconds(ts: &str) -> i64 {
    ts.split('.')
        .next()
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(0)
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
    fn after_date_goes_back_days() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        assert_eq!(after_date(today, 7), "2026-10-21");
        let today = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        assert_eq!(after_date(today, 7), "2026-10-27");
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
}
