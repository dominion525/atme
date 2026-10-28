use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::process::Command;

use crate::state::{Item, Users};

const MAX_INDIVIDUAL: usize = 5;
const MESSAGE_CHARS: usize = 100;

#[derive(Debug, PartialEq)]
struct Notification {
    title: String,
    subtitle: Option<String>,
    message: String,
    open: Option<String>,
}

impl Notification {
    fn args(&self) -> Vec<&str> {
        let mut args = vec!["-title", &self.title, "-message", &self.message];
        if let Some(subtitle) = &self.subtitle {
            args.extend(["-subtitle", subtitle]);
        }
        if let Some(open) = &self.open {
            args.extend(["-open", open]);
        }
        args.extend(["-sound", "default"]);
        args
    }
}

pub fn notify(items: &[Item], users: &Users) {
    for notification in plan(items, users) {
        match Command::new("terminal-notifier")
            .args(notification.args())
            .output()
        {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                eprintln!("terminal-notifier が見つからないため、通知を出さずに続けます");
                return;
            }
            Err(error) => eprintln!("terminal-notifier を実行できませんでした: {error}"),
            Ok(output) if !output.status.success() => eprintln!(
                "terminal-notifier が失敗しました（{}）: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Ok(_) => {}
        }
    }
}

fn plan(items: &[Item], users: &Users) -> Vec<Notification> {
    let mut notifications = Vec::new();
    if items.len() > MAX_INDIVIDUAL {
        notifications.push(Notification {
            title: "atme".into(),
            subtitle: None,
            message: format!("ほか {} 件の新着があります", items.len() - MAX_INDIVIDUAL),
            open: None,
        });
    }
    let empty = BTreeMap::new();
    notifications.extend(items.iter().take(MAX_INDIVIDUAL).rev().map(|item| {
        let names = users.get(&item.workspace).unwrap_or(&empty);
        Notification {
            title: format!("{} / {}", item.workspace, channel_label(item)),
            subtitle: item.user_name.clone(),
            message: message(&item.text, names),
            open: Some(item.permalink.clone()),
        }
    }));
    notifications
}

fn channel_label(item: &Item) -> String {
    if item.is_im {
        format!(
            "DM {}",
            item.peer_name.as_deref().unwrap_or(&item.channel_name)
        )
    } else if item.is_mpim {
        "グループ DM".into()
    } else {
        format!("#{}", item.channel_name)
    }
}

fn message(text: &str, names: &BTreeMap<String, String>) -> String {
    let text = render(text, names)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut text = if text.chars().count() > MESSAGE_CHARS {
        text.chars().take(MESSAGE_CHARS).chain(['…']).collect()
    } else {
        text
    };
    if text.is_empty() {
        text = "（本文なし）".into();
    }
    if text.starts_with('[') {
        text.insert(0, '\\');
    }
    text
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
mod tests {
    use super::*;

    fn item(channel: &str, ts: &str) -> Item {
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
    fn message_is_one_line_and_truncated() {
        let names = BTreeMap::new();
        assert_eq!(message("一行目\n\n二行目  三", &names), "一行目 二行目 三");
        let long = "あ".repeat(MESSAGE_CHARS + 1);
        assert_eq!(
            message(&long, &names),
            format!("{}…", "あ".repeat(MESSAGE_CHARS))
        );
        assert_eq!(message("", &names), "（本文なし）");
        assert_eq!(message("[重要]", &names), "\\[重要]");
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
        assert_eq!(channel_label(&group), "グループ DM");

        assert_eq!(channel_label(&item("C1", "1.0")), "#general");
    }

    #[test]
    fn plans_individual_notifications_oldest_first() {
        let items = [item("C1", "3.0"), item("C1", "2.0")];

        let planned = plan(&items, &Users::new());

        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].title, "home / #general");
        assert_eq!(planned[0].subtitle.as_deref(), Some("相手"));
        assert_eq!(planned[0].open, Some(items[1].permalink.clone()));
        assert_eq!(planned[1].open, Some(items[0].permalink.clone()));
    }

    #[test]
    fn summarizes_beyond_the_limit() {
        let items: Vec<_> = (0..MAX_INDIVIDUAL + 2)
            .map(|i| item("C1", &format!("{}.0", 100 - i)))
            .collect();

        let planned = plan(&items, &Users::new());

        assert_eq!(planned.len(), MAX_INDIVIDUAL + 1);
        assert_eq!(planned[0].message, "ほか 2 件の新着があります");
        assert_eq!(planned[0].open, None);
        assert_eq!(
            planned[MAX_INDIVIDUAL].open,
            Some(items[0].permalink.clone())
        );
    }
}
