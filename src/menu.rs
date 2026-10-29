use std::collections::BTreeMap;
use std::fmt::Display;

use chrono::TimeZone;

use crate::display;
use crate::state::{Cache, Users};

const MESSAGE_CHARS: usize = 50;

#[derive(Debug, PartialEq)]
pub enum Entry {
    Message { label: String, permalink: String },
    Info(String),
    Separator,
    Quit,
}

#[derive(Debug, PartialEq)]
pub struct Menu {
    pub title: String,
    pub entries: Vec<Entry>,
}

pub fn build<Tz: TimeZone>(cache: Option<&Cache>, users: &Users, tz: &Tz) -> Menu
where
    Tz::Offset: Display,
{
    let mut entries = Vec::new();
    let title = match cache {
        None => {
            entries.push(Entry::Info("キャッシュがありません".into()));
            "@ ?".into()
        }
        Some(cache) => {
            let empty = BTreeMap::new();
            entries.extend(cache.items.iter().map(|item| {
                let names = users.get(&item.workspace).unwrap_or(&empty);
                let sender = match &item.user_name {
                    Some(name) => format!("  {name}"),
                    None => String::new(),
                };
                Entry::Message {
                    label: format!(
                        "{}{sender}: {}",
                        display::location(item),
                        display::summary(&item.text, names, MESSAGE_CHARS)
                    ),
                    permalink: item.permalink.clone(),
                }
            }));
            if cache.items.is_empty() {
                entries.push(Entry::Info("メンションと DM はありません".into()));
            }
            entries.push(Entry::Separator);
            entries.push(Entry::Info(format!(
                "取得 {}",
                display::format_time(tz, cache.fetched_at, "%m/%d %H:%M")
            )));
            if cache.items.is_empty() {
                "@".into()
            } else {
                format!("@ {}", cache.items.len())
            }
        }
    };
    entries.push(Entry::Separator);
    entries.push(Entry::Quit);
    Menu { title, entries }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::{cache, jst, users};

    #[test]
    fn lists_messages_with_count() {
        let menu = build(Some(&cache()), &users(), &jst());

        assert_eq!(menu.title, "@ 2");
        assert_eq!(
            menu.entries,
            [
                Entry::Message {
                    label: "home / DM 相手  相手: @自分 a|b 次の行".into(),
                    permalink: "https://example.slack.com/archives/D1/p1793181600.000100".into(),
                },
                Entry::Message {
                    label: "home / #general: 本文".into(),
                    permalink: "https://example.slack.com/archives/C1/p1793178000.000200".into(),
                },
                Entry::Separator,
                Entry::Info("取得 10/28 19:05".into()),
                Entry::Separator,
                Entry::Quit,
            ]
        );
    }

    #[test]
    fn shows_bare_mark_when_empty() {
        let mut empty = cache();
        empty.items.clear();

        let menu = build(Some(&empty), &users(), &jst());

        assert_eq!(menu.title, "@");
        assert_eq!(
            menu.entries,
            [
                Entry::Info("メンションと DM はありません".into()),
                Entry::Separator,
                Entry::Info("取得 10/28 19:05".into()),
                Entry::Separator,
                Entry::Quit,
            ]
        );
    }

    #[test]
    fn shows_question_without_cache() {
        let menu = build(None, &users(), &jst());

        assert_eq!(menu.title, "@ ?");
        assert_eq!(
            menu.entries,
            [
                Entry::Info("キャッシュがありません".into()),
                Entry::Separator,
                Entry::Quit,
            ]
        );
    }
}
