use std::collections::BTreeMap;
use std::fmt::Display;

use anyhow::{bail, Result};
use chrono::{Local, TimeZone};

use crate::display;
use crate::state::{ts_seconds, Cache, Store, Users};

const TEXT_CHARS: usize = 100;

pub fn run() -> Result<()> {
    let store = Store::open()?;
    let Some(cache) = store.load_cache()? else {
        bail!("キャッシュがありません。atme fetch を実行してください");
    };
    let users = store.load_users()?.unwrap_or_default();
    print!("{}", text(&cache, &users, &Local));
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
            display::format_time(tz, ts_seconds(&item.ts), "%Y-%m-%d %H:%M"),
            display::location(item, &cache.teams),
            item.user_name.as_deref().unwrap_or(""),
            display::summary(&item.text, names, TEXT_CHARS),
            item.permalink,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::{cache, jst, users};

    #[test]
    fn formats_text() {
        assert_eq!(
            text(&cache(), &users(), &jst()),
            "2026-10-28 19:00  Home 株式会社 / DM 相手  相手\n  @自分 a|b 次の行\n  https://example.slack.com/archives/D1/p1793181600.000100\n\
             2026-10-28 18:00  Home 株式会社 / #general  \n  本文\n  https://example.slack.com/archives/C1/p1793178000.000200\n"
        );
    }
}
