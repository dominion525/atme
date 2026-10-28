use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::process::Command;

use crate::display;
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
            title: format!("{} / {}", item.workspace, display::channel_label(item)),
            subtitle: item.user_name.clone(),
            message: message(&item.text, names),
            open: Some(item.permalink.clone()),
        }
    }));
    notifications
}

fn message(text: &str, names: &BTreeMap<String, String>) -> String {
    let mut text = display::summary(text, names, MESSAGE_CHARS);
    if text.starts_with('[') {
        text.insert(0, '\\');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::item;

    #[test]
    fn message_escapes_leading_bracket() {
        let names = BTreeMap::new();
        assert_eq!(message("[重要]", &names), "\\[重要]");
        assert_eq!(message("重要 [a]", &names), "重要 [a]");
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
