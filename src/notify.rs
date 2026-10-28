use std::collections::BTreeMap;
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::{NSBundle, NSDictionary, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationStatus, UNMutableNotificationContent, UNNotificationRequest,
    UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
};

use crate::display;
use crate::state::{Item, Users};

pub const PERMALINK_KEY: &str = "permalink";
const MAX_INDIVIDUAL: usize = 5;
const MESSAGE_CHARS: usize = 100;
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(10);

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
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
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
    for notification in notifications {
        if let Err(error) = send(&center, &notification) {
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
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&notification.title));
    if let Some(subtitle) = &notification.subtitle {
        content.setSubtitle(&NSString::from_str(subtitle));
    }
    content.setBody(&NSString::from_str(&notification.message));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    if let Some(open) = &notification.open {
        let key = NSString::from_str(PERMALINK_KEY);
        let value = NSString::from_str(open);
        let info = NSDictionary::<NSString, NSString>::from_slices(&[&*key], &[&*value]);
        // userInfo は型を指定しない NSDictionary を受け取る。NSString 同士の辞書もそのまま渡せる
        let info: Retained<NSDictionary> = unsafe { Retained::cast_unchecked(info) };
        unsafe { content.setUserInfo(&info) };
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&notification.identifier),
        &content,
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
