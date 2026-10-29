use std::collections::BTreeMap;
use std::ptr::NonNull;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::{NSDictionary, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationStatus, UNMutableNotificationContent, UNNotificationRequest,
    UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
};

use crate::app;
use crate::display;
use crate::state::{Item, Users};

pub const PERMALINK_KEY: &str = "permalink";
const MAX_INDIVIDUAL: usize = 5;
const MESSAGE_CHARS: usize = 100;
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(10);
const NOTIFY_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq)]
struct Notification {
    identifier: String,
    title: String,
    subtitle: Option<String>,
    message: String,
    open: Option<String>,
}

pub fn notify(items: &[Item], users: &Users) {
    let notifications = plan(items, users);
    if notifications.is_empty() {
        return;
    }
    if !app::in_app_bundle() {
        eprintln!("atme.app の外から実行したため、通知を出しません");
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    match authorization_status(&center) {
        Some(status)
            if status == UNAuthorizationStatus::Authorized
                || status == UNAuthorizationStatus::Provisional => {}
        Some(_) => {
            eprintln!("通知が許可されていません。atme.app を open して許可してください");
            return;
        }
        None => {
            eprintln!("通知の設定を読めなかったため、通知を出しません");
            return;
        }
    }
    for (index, notification) in notifications.iter().enumerate() {
        // 同時に出すとバナーが 1 つにまとめられるので、間隔をあけて 1 件ずつ出す
        if index > 0 {
            thread::sleep(NOTIFY_INTERVAL);
        }
        if let Err(error) = send(&center, notification) {
            eprintln!("通知を出せませんでした: {error}");
        }
    }
}

fn authorization_status(center: &UNUserNotificationCenter) -> Option<UNAuthorizationStatus> {
    let (sender, receiver) = mpsc::channel();
    let handler = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        let _ = sender.send(unsafe { settings.as_ref() }.authorizationStatus());
    });
    center.getNotificationSettingsWithCompletionHandler(&handler);
    receiver.recv_timeout(CALLBACK_TIMEOUT).ok()
}

fn send(center: &UNUserNotificationCenter, notification: &Notification) -> Result<(), String> {
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&notification.identifier),
        &content(notification),
        None,
    );

    let (sender, receiver) = mpsc::channel();
    let handler = RcBlock::new(move |error: *mut NSError| {
        let error = unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
        let _ = sender.send(error);
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&handler));
    match receiver.recv_timeout(CALLBACK_TIMEOUT) {
        Ok(None) => Ok(()),
        Ok(Some(error)) => Err(error),
        Err(_) => Err("通知の登録の応答がありませんでした".into()),
    }
}

fn content(notification: &Notification) -> Retained<UNMutableNotificationContent> {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&notification.title));
    if let Some(subtitle) = &notification.subtitle {
        content.setSubtitle(&NSString::from_str(subtitle));
    }
    content.setBody(&NSString::from_str(&notification.message));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    if let Some(open) = &notification.open {
        unsafe { content.setUserInfo(&user_info(open)) };
    }
    content
}

pub fn user_info(permalink: &str) -> Retained<NSDictionary> {
    let key = NSString::from_str(PERMALINK_KEY);
    let value = NSString::from_str(permalink);
    let info = NSDictionary::<NSString, NSString>::from_slices(&[&*key], &[&*value]);
    // userInfo は型を指定しない NSDictionary を受け取る。NSString 同士の辞書もそのまま渡せる
    unsafe { Retained::cast_unchecked(info) }
}

fn plan(items: &[Item], users: &Users) -> Vec<Notification> {
    let mut notifications = Vec::new();
    if items.len() > MAX_INDIVIDUAL {
        notifications.push(Notification {
            identifier: format!("summary/{}", items[0].ts),
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
            identifier: format!("{}/{}/{}", item.workspace, item.channel_id, item.ts),
            title: format!("{} / {}", item.workspace, display::channel_label(item)),
            subtitle: item.user_name.clone(),
            message: display::summary(&item.text, names, MESSAGE_CHARS),
            open: Some(item.permalink.clone()),
        }
    }));
    notifications
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::item;

    #[test]
    fn plans_individual_notifications_oldest_first() {
        let items = [item("C1", "3.0"), item("C1", "2.0")];

        let planned = plan(&items, &Users::new());

        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].identifier, "home/C1/2.0");
        assert_eq!(planned[0].title, "home / #general");
        assert_eq!(planned[0].subtitle.as_deref(), Some("相手"));
        assert_eq!(planned[0].open, Some(items[1].permalink.clone()));
        assert_eq!(planned[1].open, Some(items[0].permalink.clone()));
    }

    #[test]
    fn builds_content_with_permalink() {
        let notification = Notification {
            identifier: "home/C1/1.0".into(),
            title: "home / #general".into(),
            subtitle: Some("相手".into()),
            message: "本文".into(),
            open: Some("https://example.slack.com/archives/C1/p1".into()),
        };

        let content = content(&notification);

        assert_eq!(content.title().to_string(), "home / #general");
        assert_eq!(content.subtitle().to_string(), "相手");
        assert_eq!(content.body().to_string(), "本文");
        assert!(content.sound().is_some());
        assert_eq!(
            crate::app::permalink(&content.userInfo()),
            Some("https://example.slack.com/archives/C1/p1".into())
        );
    }

    #[test]
    fn builds_content_without_permalink() {
        let notification = Notification {
            identifier: "summary/1.0".into(),
            title: "atme".into(),
            subtitle: None,
            message: "ほか 2 件の新着があります".into(),
            open: None,
        };

        let content = content(&notification);

        assert_eq!(content.title().to_string(), "atme");
        assert_eq!(content.userInfo().count(), 0);
        assert!(crate::app::permalink(&content.userInfo()).is_none());
    }

    #[test]
    fn summarizes_beyond_the_limit() {
        let items: Vec<_> = (0..MAX_INDIVIDUAL + 2)
            .map(|i| item("C1", &format!("{}.0", 100 - i)))
            .collect();

        let planned = plan(&items, &Users::new());

        assert_eq!(planned.len(), MAX_INDIVIDUAL + 1);
        assert_eq!(planned[0].identifier, "summary/100.0");
        assert_eq!(planned[0].message, "ほか 2 件の新着があります");
        assert_eq!(planned[0].open, None);
        assert_eq!(
            planned[MAX_INDIVIDUAL].open,
            Some(items[0].permalink.clone())
        );
    }
}
