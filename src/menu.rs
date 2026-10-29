use std::collections::BTreeMap;
use std::fmt::Display;

use chrono::TimeZone;

use crate::dismiss;
use crate::display;
use crate::fetch::Report;
use crate::state::{Cache, Dismissed, Users};

const MESSAGE_CHARS: usize = 50;
const FAILURE_CHARS: usize = 60;

#[derive(Debug, PartialEq)]
pub enum Entry {
    Message { label: String, permalink: String },
    Info(String),
    Separator,
    DismissAll,
    FetchNow,
    Quit,
}

#[derive(Debug, PartialEq)]
pub struct Menu {
    pub title: String,
    pub entries: Vec<Entry>,
}

pub fn build<Tz: TimeZone>(
    cache: Option<&Cache>,
    users: &Users,
    tz: &Tz,
    failures: &[String],
    dismissed: &Dismissed,
) -> Menu
where
    Tz::Offset: Display,
{
    let mut entries = Vec::new();
    let title = match cache {
        None => {
            entries.push(Entry::Info("キャッシュがありません".into()));
            entries.push(Entry::Separator);
            "@ ?".into()
        }
        Some(cache) => {
            let empty = BTreeMap::new();
            let items = dismiss::visible(cache, dismissed);
            entries.extend(items.iter().map(|item| {
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
            if items.is_empty() {
                entries.push(Entry::Info("メンションと DM はありません".into()));
            }
            entries.push(Entry::Separator);
            if !items.is_empty() {
                entries.push(Entry::DismissAll);
            }
            entries.push(Entry::Info(format!(
                "取得 {}",
                display::format_time(tz, cache.fetched_at, "%m/%d %H:%M")
            )));
            if items.is_empty() {
                "@".into()
            } else {
                format!("@ {}", items.len())
            }
        }
    };
    entries.extend(failures.iter().cloned().map(Entry::Info));
    entries.push(Entry::FetchNow);
    entries.push(Entry::Separator);
    entries.push(Entry::Quit);
    Menu { title, entries }
}

pub fn failure_lines(result: &anyhow::Result<Vec<Report>>) -> Vec<String> {
    match result {
        Err(error) => vec![format!("取得に失敗：{}", shorten(&format!("{error:#}")))],
        Ok(reports) => reports
            .iter()
            .filter_map(|report| {
                let error = report.result.as_ref().err()?;
                Some(format!(
                    "取得に失敗：{}（{}）",
                    report.workspace,
                    shorten(error)
                ))
            })
            .collect(),
    }
}

fn shorten(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > FAILURE_CHARS {
        text.chars().take(FAILURE_CHARS).chain(['…']).collect()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::{cache, jst, users};

    use crate::fetch::Counts;

    #[test]
    fn lists_messages_with_count() {
        let menu = build(Some(&cache()), &users(), &jst(), &[], &Dismissed::new());

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
                Entry::DismissAll,
                Entry::Info("取得 10/28 19:05".into()),
                Entry::FetchNow,
                Entry::Separator,
                Entry::Quit,
            ]
        );
    }

    #[test]
    fn hides_dismissed_from_list_and_count() {
        let cache = cache();
        let dismissed: Dismissed = [cache.items[0].key()].into();

        let menu = build(Some(&cache), &users(), &jst(), &[], &dismissed);

        assert_eq!(menu.title, "@ 1");
        assert!(matches!(
            &menu.entries[0],
            Entry::Message { label, .. } if label == "home / #general: 本文"
        ));
        assert_eq!(menu.entries[1], Entry::Separator);
        assert_eq!(menu.entries[2], Entry::DismissAll);
    }

    #[test]
    fn all_dismissed_looks_empty() {
        let cache = cache();
        let dismissed: Dismissed = dismiss::all_keys(&cache).into_iter().collect();

        let menu = build(Some(&cache), &users(), &jst(), &[], &dismissed);

        assert_eq!(menu.title, "@");
        assert_eq!(
            menu.entries[..3],
            [
                Entry::Info("メンションと DM はありません".into()),
                Entry::Separator,
                Entry::Info("取得 10/28 19:05".into()),
            ]
        );
    }

    #[test]
    fn shows_bare_mark_when_empty() {
        let mut empty = cache();
        empty.items.clear();

        let menu = build(Some(&empty), &users(), &jst(), &[], &Dismissed::new());

        assert_eq!(menu.title, "@");
        assert_eq!(
            menu.entries,
            [
                Entry::Info("メンションと DM はありません".into()),
                Entry::Separator,
                Entry::Info("取得 10/28 19:05".into()),
                Entry::FetchNow,
                Entry::Separator,
                Entry::Quit,
            ]
        );
    }

    #[test]
    fn shows_question_without_cache() {
        let menu = build(None, &users(), &jst(), &[], &Dismissed::new());

        assert_eq!(menu.title, "@ ?");
        assert_eq!(
            menu.entries,
            [
                Entry::Info("キャッシュがありません".into()),
                Entry::Separator,
                Entry::FetchNow,
                Entry::Separator,
                Entry::Quit,
            ]
        );
    }

    #[test]
    fn shows_failures_before_fetch_now() {
        let failures = ["取得に失敗：home（x）".to_owned()];

        let menu = build(None, &users(), &jst(), &failures, &Dismissed::new());

        assert_eq!(
            menu.entries[2..],
            [
                Entry::Info("取得に失敗：home（x）".into()),
                Entry::FetchNow,
                Entry::Separator,
                Entry::Quit,
            ]
        );
    }

    #[test]
    fn failure_lines_from_reports() {
        let reports = Ok(vec![
            Report {
                workspace: "home".into(),
                result: Ok(Counts { total: 1, new: 0 }),
            },
            Report {
                workspace: "work".into(),
                result: Err("auth.test を呼び出せませんでした:\n  timed out".into()),
            },
        ]);

        assert_eq!(
            failure_lines(&reports),
            ["取得に失敗：work（auth.test を呼び出せませんでした: timed out）"]
        );
    }

    #[test]
    fn failure_line_for_whole_fetch_is_shortened() {
        let result = Err(anyhow::anyhow!("{}", "あ".repeat(FAILURE_CHARS + 1)));

        assert_eq!(
            failure_lines(&result),
            [format!("取得に失敗：{}…", "あ".repeat(FAILURE_CHARS))]
        );
    }
}
