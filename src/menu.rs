use std::collections::BTreeMap;
use std::fmt::Display;
use std::time::Duration;

use chrono::TimeZone;

use crate::dismiss;
use crate::display;
use crate::fetch::Report;
use crate::login::LoginState;
use crate::scheduler::Phase;
use crate::state::{Cache, Dismissed, Users};

const MESSAGE_CHARS: usize = 50;
const FAILURE_CHARS: usize = 60;

/// メニューの「今すぐ取得」の行の状態
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FetchButton {
    /// 押せる。押せばすぐ取得する
    Ready,
    /// 前回の取得から間があいていないので押せない。あと何秒で押せるか
    Cooldown(u64),
    /// 自動の取得（スリープからの復帰など）が、間があくのを待っている
    Waiting,
    Fetching,
}

/// 取得の状態と、前回の取得が終わってからの時間から、行の状態を決める
pub fn fetch_button(phase: Phase, since_last: Option<Duration>, gap: Duration) -> FetchButton {
    match phase {
        Phase::Fetching => FetchButton::Fetching,
        Phase::Waiting => FetchButton::Waiting,
        Phase::Idle => match since_last {
            Some(elapsed) if elapsed < gap => {
                FetchButton::Cooldown((gap - elapsed).as_secs_f64().ceil() as u64)
            }
            _ => FetchButton::Ready,
        },
    }
}

#[derive(Debug, PartialEq)]
pub enum Entry {
    /// ワークスペース名の見出し
    Header(String),
    Message {
        label: String,
        permalink: String,
    },
    /// 見出しの下に置く「なし」
    NoItems,
    Info(String),
    Separator,
    DismissAll,
    Fetch(FetchButton),
    LaunchAtLogin(LoginState),
    /// Sparkle の「アップデートを確認…」
    CheckForUpdates,
    Quit,
}

#[derive(Debug, PartialEq)]
pub struct Menu {
    /// メニューバーの絵の右に出す文字（件数。0 件なら空、キャッシュが無ければ ?）
    pub badge: String,
    pub entries: Vec<Entry>,
}

pub struct Inputs<'a> {
    pub cache: Option<&'a Cache>,
    pub users: &'a Users,
    pub failures: &'a [String],
    pub dismissed: &'a Dismissed,
    pub login: LoginState,
    /// Sparkle を読み込めたか。読み込めたときだけ「アップデートを確認…」を出す
    pub updates: bool,
    pub fetch: FetchButton,
    /// 設定ファイルに書かれた順のワークスペース（設定の name）
    pub workspaces: &'a [String],
}

pub fn build<Tz: TimeZone>(inputs: &Inputs, tz: &Tz) -> Menu
where
    Tz::Offset: Display,
{
    let mut entries = Vec::new();
    let badge = match inputs.cache {
        None => {
            entries.push(Entry::Info("キャッシュがありません".into()));
            entries.push(Entry::Separator);
            "?".into()
        }
        Some(cache) => {
            let items = dismiss::visible(cache, inputs.dismissed);
            // 設定に無くなったワークスペースの項目も、見出しを付けて後ろに出す
            let mut workspaces: Vec<&str> = inputs.workspaces.iter().map(String::as_str).collect();
            for item in &items {
                if !workspaces.contains(&item.workspace.as_str()) {
                    workspaces.push(&item.workspace);
                }
            }
            let empty = BTreeMap::new();
            for workspace in &workspaces {
                entries.push(Entry::Header(
                    display::workspace_name(&cache.teams, workspace).to_owned(),
                ));
                let names = inputs.users.get(*workspace).unwrap_or(&empty);
                let before = entries.len();
                entries.extend(
                    items
                        .iter()
                        .filter(|item| item.workspace == *workspace)
                        .map(|item| {
                            let sender = match &item.user_name {
                                Some(name) => format!("  {name}"),
                                None => String::new(),
                            };
                            Entry::Message {
                                label: format!(
                                    "{}{sender}: {}",
                                    display::channel_label(item),
                                    display::summary(&item.text, names, MESSAGE_CHARS)
                                ),
                                permalink: item.permalink.clone(),
                            }
                        }),
                );
                if entries.len() == before {
                    entries.push(Entry::NoItems);
                }
            }
            if workspaces.is_empty() {
                entries.push(Entry::Info("メンションと DM はありません".into()));
            }
            entries.push(Entry::Separator);
            if !items.is_empty() {
                entries.push(Entry::DismissAll);
            }
            // 取得時刻は、1 度も成功していなければ 0 のまま
            entries.push(Entry::Info(if cache.fetched_at == 0 {
                "まだ取得できていません".into()
            } else {
                format!(
                    "取得 {}",
                    display::format_time(tz, cache.fetched_at, "%m/%d %H:%M")
                )
            }));
            if items.is_empty() {
                String::new()
            } else {
                items.len().to_string()
            }
        }
    };
    entries.extend(inputs.failures.iter().cloned().map(Entry::Info));
    entries.push(Entry::Fetch(inputs.fetch));
    entries.push(Entry::LaunchAtLogin(inputs.login));
    if inputs.updates {
        entries.push(Entry::CheckForUpdates);
    }
    entries.push(Entry::Separator);
    entries.push(Entry::Quit);
    Menu { badge, entries }
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

    const DM: &str = "https://example.slack.com/archives/D1/p1793181600.000100";
    const GENERAL: &str = "https://example.slack.com/archives/C1/p1793178000.000200";

    fn build_with(
        cache: Option<&Cache>,
        dismissed: &Dismissed,
        failures: &[String],
        login: LoginState,
        workspaces: &[String],
    ) -> Menu {
        let users = users();
        build(
            &Inputs {
                cache,
                users: &users,
                failures,
                dismissed,
                login,
                updates: false,
                fetch: FetchButton::Ready,
                workspaces,
            },
            &jst(),
        )
    }

    fn home() -> Vec<String> {
        vec!["home".to_owned()]
    }

    fn footer() -> [Entry; 4] {
        [
            Entry::Fetch(FetchButton::Ready),
            Entry::LaunchAtLogin(LoginState::Off),
            Entry::Separator,
            Entry::Quit,
        ]
    }

    #[test]
    fn groups_messages_under_workspace_header() {
        let menu = build_with(
            Some(&cache()),
            &Dismissed::new(),
            &[],
            LoginState::Off,
            &home(),
        );

        assert_eq!(menu.badge, "2");
        assert_eq!(
            menu.entries[..6],
            [
                Entry::Header("Home 株式会社".into()),
                Entry::Message {
                    label: "DM 相手  相手: @自分 a|b 次の行".into(),
                    permalink: DM.into(),
                },
                Entry::Message {
                    label: "#general: 本文".into(),
                    permalink: GENERAL.into(),
                },
                Entry::Separator,
                Entry::DismissAll,
                Entry::Info("取得 10/28 19:05".into()),
            ]
        );
        assert_eq!(menu.entries[6..], footer());
    }

    #[test]
    fn shows_every_configured_workspace_even_without_items() {
        let workspaces = vec!["work".to_owned(), "home".to_owned()];

        let menu = build_with(
            Some(&cache()),
            &Dismissed::new(),
            &[],
            LoginState::Off,
            &workspaces,
        );

        assert_eq!(
            menu.entries[..3],
            [
                Entry::Header("work".into()),
                Entry::NoItems,
                Entry::Header("Home 株式会社".into()),
            ]
        );
    }

    #[test]
    fn keeps_items_of_workspaces_removed_from_config() {
        let menu = build_with(Some(&cache()), &Dismissed::new(), &[], LoginState::Off, &[]);

        assert_eq!(menu.entries[0], Entry::Header("Home 株式会社".into()));
        assert_eq!(menu.badge, "2");
    }

    #[test]
    fn hides_dismissed_from_list_and_count() {
        let cache = cache();
        let dismissed: Dismissed = [cache.items[0].key()].into();

        let menu = build_with(Some(&cache), &dismissed, &[], LoginState::Off, &home());

        assert_eq!(menu.badge, "1");
        assert_eq!(
            menu.entries[..4],
            [
                Entry::Header("Home 株式会社".into()),
                Entry::Message {
                    label: "#general: 本文".into(),
                    permalink: GENERAL.into(),
                },
                Entry::Separator,
                Entry::DismissAll,
            ]
        );
    }

    #[test]
    fn all_dismissed_shows_no_items() {
        let cache = cache();
        let dismissed: Dismissed = dismiss::all_keys(&cache).into_iter().collect();

        let menu = build_with(Some(&cache), &dismissed, &[], LoginState::Off, &home());

        assert_eq!(menu.badge, "");
        assert_eq!(
            menu.entries[..4],
            [
                Entry::Header("Home 株式会社".into()),
                Entry::NoItems,
                Entry::Separator,
                Entry::Info("取得 10/28 19:05".into()),
            ]
        );
    }

    #[test]
    fn says_nothing_when_no_workspace_is_known() {
        let mut empty = cache();
        empty.items.clear();

        let menu = build_with(Some(&empty), &Dismissed::new(), &[], LoginState::Off, &[]);

        assert_eq!(menu.badge, "");
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
    fn says_not_fetched_yet_when_never_succeeded() {
        let mut never = cache();
        never.items.clear();
        never.fetched_at = 0;

        let menu = build_with(Some(&never), &Dismissed::new(), &[], LoginState::Off, &[]);

        assert_eq!(
            menu.entries[2],
            Entry::Info("まだ取得できていません".into())
        );
    }

    #[test]
    fn shows_question_without_cache() {
        let menu = build_with(None, &Dismissed::new(), &[], LoginState::Off, &home());

        assert_eq!(menu.badge, "?");
        assert_eq!(
            menu.entries[..2],
            [
                Entry::Info("キャッシュがありません".into()),
                Entry::Separator,
            ]
        );
        assert_eq!(menu.entries[2..], footer());
    }

    #[test]
    fn shows_failures_before_fetch_now() {
        let failures = ["取得に失敗：home（x）".to_owned()];

        let menu = build_with(None, &Dismissed::new(), &failures, LoginState::Off, &home());

        assert_eq!(menu.entries[2], Entry::Info("取得に失敗：home（x）".into()));
        assert_eq!(menu.entries[3..], footer());
    }

    #[test]
    fn shows_fetch_phase() {
        let users = users();
        for fetch in [
            FetchButton::Fetching,
            FetchButton::Waiting,
            FetchButton::Cooldown(20),
        ] {
            let menu = build(
                &Inputs {
                    cache: None,
                    users: &users,
                    failures: &[],
                    dismissed: &Dismissed::new(),
                    login: LoginState::Off,
                    updates: false,
                    fetch,
                    workspaces: &home(),
                },
                &jst(),
            );

            assert!(menu.entries.contains(&Entry::Fetch(fetch)));
        }
    }

    #[test]
    fn fetch_button_follows_phase_and_gap() {
        let gap = Duration::from_secs(30);

        assert_eq!(
            fetch_button(Phase::Fetching, None, gap),
            FetchButton::Fetching
        );
        assert_eq!(
            fetch_button(Phase::Waiting, Some(gap), gap),
            FetchButton::Waiting
        );
        assert_eq!(fetch_button(Phase::Idle, None, gap), FetchButton::Ready);
        assert_eq!(
            fetch_button(Phase::Idle, Some(Duration::from_secs(30)), gap),
            FetchButton::Ready
        );
        assert_eq!(
            fetch_button(Phase::Idle, Some(Duration::from_millis(9_500)), gap),
            FetchButton::Cooldown(21)
        );
        assert_eq!(
            fetch_button(Phase::Idle, Some(Duration::from_millis(29_999)), gap),
            FetchButton::Cooldown(1)
        );
    }

    #[test]
    fn shows_check_for_updates_only_with_sparkle() {
        let users = users();
        let menu = build(
            &Inputs {
                cache: None,
                users: &users,
                failures: &[],
                dismissed: &Dismissed::new(),
                login: LoginState::Off,
                updates: true,
                fetch: FetchButton::Ready,
                workspaces: &home(),
            },
            &jst(),
        );

        assert_eq!(
            menu.entries[menu.entries.len() - 4..],
            [
                Entry::LaunchAtLogin(LoginState::Off),
                Entry::CheckForUpdates,
                Entry::Separator,
                Entry::Quit,
            ]
        );
        assert!(
            !build_with(None, &Dismissed::new(), &[], LoginState::Off, &home())
                .entries
                .contains(&Entry::CheckForUpdates)
        );
    }

    #[test]
    fn shows_login_state() {
        for login in [LoginState::On, LoginState::NeedsApproval] {
            let menu = build_with(None, &Dismissed::new(), &[], login, &home());

            assert!(menu.entries.contains(&Entry::LaunchAtLogin(login)));
        }
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
