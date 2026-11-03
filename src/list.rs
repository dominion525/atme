use std::collections::BTreeMap;
use std::fmt::Display;

use anyhow::{Result, bail};
use chrono::{Local, TimeZone};

use crate::display;
use crate::l10n;
use crate::state::{Cache, Store, Users, ts_seconds};

const TEXT_CHARS: usize = 100;

pub fn run() -> Result<()> {
    print!("{}", output(&Store::open()?, &Local)?);
    Ok(())
}

/// store のキャッシュから、atme list が出す文を作る
fn output<Tz: TimeZone>(store: &Store, tz: &Tz) -> Result<String>
where
    Tz::Offset: Display,
{
    let Some(cache) = store.load_cache()? else {
        bail!(l10n::text(l10n::NO_CACHE_RUN_FETCH));
    };
    let users = store.load_users()?.unwrap_or_default();
    Ok(text(&cache, &users, tz))
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
    fn output_needs_a_cache() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());

        let error = output(&store, &jst()).unwrap_err();

        assert_eq!(error.to_string(), "No cache. Run atme fetch");
    }

    #[test]
    fn output_reads_the_cache_and_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.save_cache(&cache()).unwrap();
        store.save_users(&users()).unwrap();

        assert_eq!(
            output(&store, &jst()).unwrap(),
            text(&cache(), &users(), &jst())
        );
    }

    #[test]
    fn formats_text() {
        assert_eq!(
            text(&cache(), &users(), &jst()),
            "2026-10-28 19:00  Home 株式会社 / DM 相手  相手\n  @自分 a|b 次の行\n  https://example.slack.com/archives/D1/p1793181600.000100\n\
             2026-10-28 18:00  Home 株式会社 / #general  \n  本文\n  https://example.slack.com/archives/C1/p1793178000.000200\n"
        );
    }
}
