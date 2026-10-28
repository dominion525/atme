use std::collections::BTreeMap;
use std::fmt::Display;

use anyhow::{bail, Result};
use chrono::{Local, TimeZone};
use clap::ValueEnum;

use crate::display;
use crate::state::{ts_seconds, Cache, Item, Store, Users};

const TEXT_CHARS: usize = 100;
const SWIFTBAR_CHARS: usize = 50;
const NO_CACHE: &str = "キャッシュがありません。atme fetch を実行してください";

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Text,
    Swiftbar,
}

pub fn run(format: Format) -> Result<()> {
    let store = Store::open()?;
    let cache = store.load_cache()?;
    let users = store.load_users()?.unwrap_or_default();
    match (format, cache) {
        (Format::Text, Some(cache)) => print!("{}", text(&cache, &users, &Local)),
        (Format::Text, None) => bail!(NO_CACHE),
        (Format::Swiftbar, cache) => print!("{}", swiftbar(cache.as_ref(), &users, &Local)),
    }
    Ok(())
}

fn text<Tz: TimeZone>(cache: &Cache, users: &Users, tz: &Tz) -> String
where
    Tz::Offset: Display,
{
    let empty = BTreeMap::new();
    let mut out = String::new();
    for item in &cache.items {
        let names = users.get(&item.workspace).unwrap_or(&empty);
        out.push_str(&format!(
            "{}  {}  {}\n  {}\n  {}\n",
            format_time(tz, ts_seconds(&item.ts), "%Y-%m-%d %H:%M"),
            location(item),
            item.user_name.as_deref().unwrap_or(""),
            display::summary(&item.text, names, TEXT_CHARS),
            item.permalink,
        ));
    }
    out
}

fn swiftbar<Tz: TimeZone>(cache: Option<&Cache>, users: &Users, tz: &Tz) -> String
where
    Tz::Offset: Display,
{
    let Some(cache) = cache else {
        return format!("@ ?\n---\n{NO_CACHE}\n");
    };
    let empty = BTreeMap::new();
    let mut out = format!("@ {}\n---\n", cache.items.len());
    for item in &cache.items {
        let names = users.get(&item.workspace).unwrap_or(&empty);
        let sender = match &item.user_name {
            Some(name) => format!("  {name}"),
            None => String::new(),
        };
        let line = format!(
            "{}{sender}: {}",
            location(item),
            display::summary(&item.text, names, SWIFTBAR_CHARS)
        );
        out.push_str(&format!(
            "{} | href={}\n",
            line.replace('|', "｜"),
            item.permalink
        ));
    }
    out.push_str(&format!(
        "---\n取得 {}\n",
        format_time(tz, cache.fetched_at, "%m/%d %H:%M")
    ));
    out
}

fn location(item: &Item) -> String {
    format!("{} / {}", item.workspace, display::channel_label(item))
}

fn format_time<Tz: TimeZone>(tz: &Tz, seconds: i64, pattern: &str) -> String
where
    Tz::Offset: Display,
{
    tz.timestamp_opt(seconds, 0)
        .single()
        .map_or_else(String::new, |time| time.format(pattern).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::item;
    use chrono::FixedOffset;

    fn jst() -> FixedOffset {
        FixedOffset::east_opt(9 * 3600).unwrap()
    }

    fn cache() -> Cache {
        let mut dm = item("D1", "1793181600.000100");
        dm.is_im = true;
        dm.channel_name = "U2".into();
        dm.peer_name = Some("相手".into());
        dm.text = "<@U1> a|b\n次の行".into();
        let mut bot = item("C1", "1793178000.000200");
        bot.user_name = None;
        Cache {
            fetched_at: 1793181900,
            items: vec![dm, bot],
        }
    }

    fn users() -> Users {
        BTreeMap::from([(
            "home".to_owned(),
            BTreeMap::from([("U1".to_owned(), "自分".to_owned())]),
        )])
    }

    #[test]
    fn formats_text() {
        assert_eq!(
            text(&cache(), &users(), &jst()),
            "2026-10-28 19:00  home / DM 相手  相手\n  @自分 a|b 次の行\n  https://example.slack.com/archives/D1/p1793181600.000100\n\
             2026-10-28 18:00  home / #general  \n  本文\n  https://example.slack.com/archives/C1/p1793178000.000200\n"
        );
    }

    #[test]
    fn formats_swiftbar() {
        assert_eq!(
            swiftbar(Some(&cache()), &users(), &jst()),
            "@ 2\n---\n\
             home / DM 相手  相手: @自分 a｜b 次の行 | href=https://example.slack.com/archives/D1/p1793181600.000100\n\
             home / #general: 本文 | href=https://example.slack.com/archives/C1/p1793178000.000200\n\
             ---\n取得 10/28 19:05\n"
        );
    }

    #[test]
    fn swiftbar_without_cache() {
        assert_eq!(
            swiftbar(None, &users(), &jst()),
            format!("@ ?\n---\n{NO_CACHE}\n")
        );
    }
}
