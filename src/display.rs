use std::collections::BTreeMap;
use std::fmt::Display;

use chrono::TimeZone;

use crate::l10n;
use crate::state::Item;

/// メニューの行に出す名前の長さの上限（文字の数）
const NAME_CHARS: usize = 12;

/// Slack 上のワークスペース名。まだ取得できていなければ設定の name を使う
pub fn workspace_name<'a>(teams: &'a BTreeMap<String, String>, workspace: &'a str) -> &'a str {
    teams.get(workspace).map_or(workspace, String::as_str)
}

pub fn location(item: &Item, teams: &BTreeMap<String, String>) -> String {
    format!(
        "{} / {}",
        workspace_name(teams, &item.workspace),
        channel_label(item)
    )
}

/// メニューの行に出す名前。Slack の表示名には所属や状況まで入れる人がいるので、先頭の NAME_CHARS 文字で切る
pub fn short_name(name: &str) -> String {
    if name.chars().count() > NAME_CHARS {
        // 切った末尾に空白が残ると「開発部 …」のようになるので、空白を落としてから付ける
        let head: String = name.chars().take(NAME_CHARS).collect();
        format!("{}…", head.trim_end())
    } else {
        name.to_owned()
    }
}

/// 表示する送り主。1 対 1 の DM で、送り主が場所に出る相手と同じなら、重なるので出さない
pub fn sender(item: &Item) -> Option<&str> {
    let name = item.user_name.as_deref()?;
    (!(item.is_im && item.peer_name.as_deref() == Some(name))).then_some(name)
}

pub fn format_time<Tz: TimeZone>(tz: &Tz, seconds: i64, pattern: &str) -> String
where
    Tz::Offset: Display,
{
    tz.timestamp_opt(seconds, 0)
        .single()
        .map_or_else(String::new, |time| time.format(pattern).to_string())
}

pub fn channel_label(item: &Item) -> String {
    channel_label_with(item, |name| name.to_owned())
}

/// メニューの行に出す場所。DM の相手の名前は short_name で切る
pub fn menu_channel_label(item: &Item) -> String {
    channel_label_with(item, short_name)
}

fn channel_label_with(item: &Item, name: impl Fn(&str) -> String) -> String {
    if item.is_im {
        format!(
            "DM {}",
            name(item.peer_name.as_deref().unwrap_or(&item.channel_name))
        )
    } else if item.is_mpim {
        l10n::text(l10n::GROUP_DM)
    } else {
        format!("#{}", item.channel_name)
    }
}

pub fn summary(text: &str, names: &BTreeMap<String, String>, max_chars: usize) -> String {
    let text = render(text, names)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        l10n::text(l10n::NO_TEXT)
    } else if text.chars().count() > max_chars {
        text.chars().take(max_chars).chain(['…']).collect()
    } else {
        text
    }
}

fn render(text: &str, names: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        out.push_str(&render_token(&after[..end], names));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn render_token(token: &str, names: &BTreeMap<String, String>) -> String {
    let (target, label) = match token.split_once('|') {
        Some((target, label)) => (target, Some(label)),
        None => (token, None),
    };
    if let Some(id) = target.strip_prefix('@') {
        let name = names.get(id).map(String::as_str).or(label).unwrap_or(id);
        return format!("@{name}");
    }
    if let Some(id) = target.strip_prefix('#') {
        return format!("#{}", label.unwrap_or(id));
    }
    if let Some(special) = target.strip_prefix('!') {
        return match (special, label) {
            ("here" | "channel" | "everyone", _) => format!("@{special}"),
            (_, Some(label)) => label.to_owned(),
            (_, None) => format!("@{}", special.split('^').next().unwrap_or(special)),
        };
    }
    label.unwrap_or(target).to_owned()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::state::{Cache, Users};
    use chrono::FixedOffset;

    pub fn jst() -> FixedOffset {
        FixedOffset::east_opt(9 * 3600).unwrap()
    }

    pub fn cache() -> Cache {
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
            teams: BTreeMap::from([("home".to_owned(), "Home 株式会社".to_owned())]),
            team_ids: BTreeMap::from([("home".to_owned(), "T1".to_owned())]),
        }
    }

    pub fn users() -> Users {
        BTreeMap::from([(
            "home".to_owned(),
            BTreeMap::from([("U1".to_owned(), "自分".to_owned())]),
        )])
    }

    pub fn item(channel: &str, ts: &str) -> Item {
        Item {
            workspace: "home".into(),
            channel_id: channel.into(),
            channel_name: "general".into(),
            is_im: false,
            is_mpim: false,
            peer_name: None,
            user: Some("U2".into()),
            user_name: Some("相手".into()),
            ts: ts.into(),
            text: "本文".into(),
            permalink: format!("https://example.slack.com/archives/{channel}/p{ts}"),
            thread_ts: None,
        }
    }

    #[test]
    fn renders_slack_markup() {
        let names = BTreeMap::from([("U1".to_owned(), "自分".to_owned())]);
        assert_eq!(
            render(
                "<@U1> <@U9> <!here> <!channel> <!subteam^S1|@dev> <#C1|general> <https://a.example|リンク> <https://b.example> a &lt;b&gt; &amp; c",
                &names
            ),
            "@自分 @U9 @here @channel @dev #general リンク https://b.example a <b> & c"
        );
        assert_eq!(render("閉じない <@U1", &names), "閉じない <@U1");
    }

    #[test]
    fn summary_is_one_line_and_truncated() {
        let names = BTreeMap::new();
        assert_eq!(
            summary("一行目\n\n二行目  三", &names, 10),
            "一行目 二行目 三"
        );
        assert_eq!(summary("あいうえお", &names, 3), "あいう…");
        assert_eq!(summary("あいう", &names, 3), "あいう");
        assert_eq!(summary("", &names, 3), "(no text)");
    }

    #[test]
    fn workspace_name_falls_back_to_config_name() {
        let teams = cache().teams;

        assert_eq!(workspace_name(&teams, "home"), "Home 株式会社");
        assert_eq!(workspace_name(&teams, "work"), "work");
    }

    #[test]
    fn shortens_long_names_for_the_menu() {
        assert_eq!(short_name("佐藤"), "佐藤");
        assert_eq!(short_name("123456789012"), "123456789012");
        assert_eq!(
            short_name("佐藤 太郎 / 開発部 / 在宅勤務中"),
            "佐藤 太郎 / 開発部…"
        );

        let mut dm = item("D1", "1793181600.000100");
        dm.is_im = true;
        dm.peer_name = Some("佐藤 太郎 / 開発部 / 在宅勤務中".into());
        // メニューでは切り、通知の題などに使う場所の名前は切らない
        assert_eq!(menu_channel_label(&dm), "DM 佐藤 太郎 / 開発部…");
        assert_eq!(channel_label(&dm), "DM 佐藤 太郎 / 開発部 / 在宅勤務中");
    }

    #[test]
    fn omits_the_sender_only_when_a_dm_names_them_already() {
        let mut dm = item("D1", "1793181600.000100");
        dm.is_im = true;
        dm.peer_name = Some("佐藤".into());
        dm.user_name = Some("佐藤".into());
        assert_eq!(sender(&dm), None);

        // DM でも、相手以外（アプリなど）からなら出す
        dm.user_name = Some("Reminder".into());
        assert_eq!(sender(&dm), Some("Reminder"));

        let mut group = item("G1", "1793181600.000100");
        group.is_mpim = true;
        group.user_name = Some("伊藤".into());
        assert_eq!(sender(&group), Some("伊藤"));
    }

    #[test]
    fn labels_channels() {
        let mut dm = item("D1", "1.0");
        dm.is_im = true;
        dm.channel_name = "U2".into();
        assert_eq!(channel_label(&dm), "DM U2");
        dm.peer_name = Some("相手".into());
        assert_eq!(channel_label(&dm), "DM 相手");

        let mut group = item("G1", "1.0");
        group.is_mpim = true;
        assert_eq!(channel_label(&group), "Group DM");

        assert_eq!(channel_label(&item("C1", "1.0")), "#general");
    }
}
