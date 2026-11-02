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
use crate::l10n;
use crate::state::{Item, SeenKey, Users};

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
    /// この通知で知らせる未読（個別の通知はその 1 件、まとめの通知は個別に出さなかった残り）
    covers: Vec<SeenKey>,
}

/// items を通知し、実際に知らせることができた未読を返す。出せなかった分は返さない（次の取得で、もう一度知らせる）
pub fn notify(items: &[Item], users: &Users, teams: &BTreeMap<String, String>) -> Vec<SeenKey> {
    let notifications = plan(items, users, teams);
    if notifications.is_empty() {
        return Vec::new();
    }
    if !app::in_app_bundle() {
        eprintln!("{}", l10n::text(l10n::OUTSIDE_APP_NOTIFY));
        return Vec::new();
    }
    match deliver(&SystemCenter::new(), &notifications) {
        Ok(sent) => {
            for failure in sent.failures {
                eprintln!("{}", l10n::format(l10n::NOTIFY_FAILED, &[&failure]));
            }
            sent.delivered
        }
        Err(refused) => {
            eprintln!(
                "{}",
                l10n::text(match refused {
                    Refused::NotAllowed => l10n::NOTIFICATIONS_NOT_ALLOWED_OPEN,
                    Refused::SettingsUnreadable => l10n::NOTIFICATION_SETTINGS_UNREADABLE,
                })
            );
            Vec::new()
        }
    }
}

/// 通知を出す先に頼むこと。ふだんは macOS の通知センター（SystemCenter）、テストでは偽物
trait Center {
    /// 通知の許可の状態。読めなかったときは None
    fn status(&self) -> Option<UNAuthorizationStatus>;
    fn send(&self, notification: &Notification) -> Result<(), String>;
    /// 次の通知を出すまで待つ
    fn pause(&self);
}

#[derive(Debug, PartialEq)]
enum Refused {
    NotAllowed,
    SettingsUnreadable,
}

/// deliver の結果
#[derive(Debug, PartialEq)]
struct Sent {
    /// 出せた通知が知らせた未読
    delivered: Vec<SeenKey>,
    /// 出せなかった通知の理由
    failures: Vec<String>,
}

/// 許可があれば、notifications を 1 件ずつ出す（途中で失敗しても残りは出す）
fn deliver(center: &impl Center, notifications: &[Notification]) -> Result<Sent, Refused> {
    match center.status() {
        None => return Err(Refused::SettingsUnreadable),
        Some(status) if !allows(status) => return Err(Refused::NotAllowed),
        Some(_) => {}
    }
    let mut sent = Sent {
        delivered: Vec::new(),
        failures: Vec::new(),
    };
    for (index, notification) in notifications.iter().enumerate() {
        // 同時に出すとバナーが 1 つにまとめられるので、間隔をあけて 1 件ずつ出す
        if index > 0 {
            center.pause();
        }
        match center.send(notification) {
            Ok(()) => sent.delivered.extend(notification.covers.iter().cloned()),
            Err(error) => sent.failures.push(error),
        }
    }
    Ok(sent)
}

/// 通知を出してよい許可の状態（許可済みと、仮の許可）
fn allows(status: UNAuthorizationStatus) -> bool {
    status == UNAuthorizationStatus::Authorized || status == UNAuthorizationStatus::Provisional
}

/// macOS の通知センター
struct SystemCenter(Retained<UNUserNotificationCenter>);

impl SystemCenter {
    fn new() -> Self {
        Self(UNUserNotificationCenter::currentNotificationCenter())
    }
}

impl Center for SystemCenter {
    fn status(&self) -> Option<UNAuthorizationStatus> {
        authorization_status(&self.0)
    }

    fn send(&self, notification: &Notification) -> Result<(), String> {
        send(&self.0, notification)
    }

    fn pause(&self) {
        thread::sleep(NOTIFY_INTERVAL);
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
        Err(_) => Err(l10n::text(l10n::NOTIFICATION_NO_RESPONSE).into()),
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

fn plan(items: &[Item], users: &Users, teams: &BTreeMap<String, String>) -> Vec<Notification> {
    let mut notifications = Vec::new();
    if items.len() > MAX_INDIVIDUAL {
        notifications.push(Notification {
            identifier: format!("summary/{}", items[0].ts),
            title: "atme".into(),
            subtitle: None,
            message: l10n::format(l10n::AND_MORE, &[&(items.len() - MAX_INDIVIDUAL).to_string()]),
            open: None,
            covers: items[MAX_INDIVIDUAL..].iter().map(Item::key).collect(),
        });
    }
    let empty = BTreeMap::new();
    notifications.extend(items.iter().take(MAX_INDIVIDUAL).rev().map(|item| {
        let names = users.get(&item.workspace).unwrap_or(&empty);
        Notification {
            identifier: format!("{}/{}/{}", item.workspace, item.channel_id, item.ts),
            title: display::location(item, teams),
            subtitle: display::sender(item).map(str::to_owned),
            message: display::summary(&item.text, names, MESSAGE_CHARS),
            open: Some(item.permalink.clone()),
            covers: vec![item.key()],
        }
    }));
    notifications
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::item;

    #[test]
    fn each_notification_covers_the_unread_it_tells_about() {
        // 7 件のうち、新しい 5 件は個別に、残りの 2 件はまとめの通知で知らせる
        let items: Vec<Item> = (1..=7).rev().map(|n| item("C1", &format!("{n}.0"))).collect();

        let planned = plan(&items, &Users::new(), &BTreeMap::new());

        let covered: Vec<Vec<String>> = planned
            .iter()
            .map(|notification| notification.covers.iter().map(|key| key.ts.clone()).collect())
            .collect();
        assert_eq!(
            covered,
            [
                vec!["2.0".to_owned(), "1.0".to_owned()],
                vec!["3.0".to_owned()],
                vec!["4.0".to_owned()],
                vec!["5.0".to_owned()],
                vec!["6.0".to_owned()],
                vec!["7.0".to_owned()],
            ]
        );
    }

    #[test]
    fn plans_individual_notifications_oldest_first() {
        let items = [item("C1", "3.0"), item("C1", "2.0")];

        let planned = plan(&items, &Users::new(), &BTreeMap::new());

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
            covers: Vec::new(),
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
            message: "And 2 more".into(),
            open: None,
            covers: Vec::new(),
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

        let planned = plan(&items, &Users::new(), &BTreeMap::new());

        assert_eq!(planned.len(), MAX_INDIVIDUAL + 1);
        assert_eq!(planned[0].identifier, "summary/100.0");
        assert_eq!(planned[0].message, "And 2 more");
        assert_eq!(planned[0].open, None);
        assert_eq!(
            planned[MAX_INDIVIDUAL].open,
            Some(items[0].permalink.clone())
        );
    }
}

/// 通知センターを偽物にして、許可の扱いと、1 件ずつ出す流れを確かめる
#[cfg(test)]
mod delivery_tests {
    use std::cell::RefCell;

    use super::*;

    /// 起きたことを順に記録する偽の通知センター。fail に入れた identifier の通知は失敗させる
    struct FakeCenter {
        status: Option<UNAuthorizationStatus>,
        fail: Vec<&'static str>,
        log: RefCell<Vec<String>>,
    }

    impl FakeCenter {
        fn new(status: Option<UNAuthorizationStatus>) -> Self {
            Self {
                status,
                fail: Vec::new(),
                log: RefCell::new(Vec::new()),
            }
        }
    }

    impl Center for FakeCenter {
        fn status(&self) -> Option<UNAuthorizationStatus> {
            self.log.borrow_mut().push("status".into());
            self.status
        }

        fn send(&self, notification: &Notification) -> Result<(), String> {
            self.log
                .borrow_mut()
                .push(format!("send {}", notification.identifier));
            if self.fail.contains(&notification.identifier.as_str()) {
                Err(format!("rejected {}", notification.identifier))
            } else {
                Ok(())
            }
        }

        fn pause(&self) {
            self.log.borrow_mut().push("pause".into());
        }
    }

    /// identifier の未読 1 件を知らせる通知
    fn notification(identifier: &str) -> Notification {
        Notification {
            identifier: identifier.into(),
            title: "Home / #general".into(),
            subtitle: None,
            message: "本文".into(),
            open: None,
            covers: vec![key(identifier)],
        }
    }

    fn key(ts: &str) -> SeenKey {
        SeenKey {
            workspace: "home".into(),
            channel: "C1".into(),
            ts: ts.into(),
        }
    }

    #[test]
    fn sends_one_by_one_with_a_pause_between() {
        let center = FakeCenter::new(Some(UNAuthorizationStatus::Authorized));

        let sent = deliver(&center, &[notification("a"), notification("b"), notification("c")]);

        assert_eq!(
            sent,
            Ok(Sent {
                delivered: vec![key("a"), key("b"), key("c")],
                failures: Vec::new(),
            })
        );
        assert_eq!(
            *center.log.borrow(),
            ["status", "send a", "pause", "send b", "pause", "send c"]
        );
    }

    #[test]
    fn keeps_sending_after_a_failure() {
        let mut center = FakeCenter::new(Some(UNAuthorizationStatus::Authorized));
        center.fail = vec!["b"];

        let sent = deliver(&center, &[notification("a"), notification("b"), notification("c")]);

        // 出せなかった b は、知らせた未読に入れない（次の取得で、もう一度知らせる）
        assert_eq!(
            sent,
            Ok(Sent {
                delivered: vec![key("a"), key("c")],
                failures: vec!["rejected b".to_owned()],
            })
        );
        assert!(center.log.borrow().contains(&"send c".to_owned()));
    }

    #[test]
    fn sends_nothing_without_permission() {
        let center = FakeCenter::new(Some(UNAuthorizationStatus::Denied));

        let result = deliver(&center, &[notification("a")]);

        assert_eq!(result, Err(Refused::NotAllowed));
        assert_eq!(*center.log.borrow(), ["status"]);
    }

    #[test]
    fn sends_nothing_when_settings_cannot_be_read() {
        let center = FakeCenter::new(None);

        let result = deliver(&center, &[notification("a")]);

        assert_eq!(result, Err(Refused::SettingsUnreadable));
        assert_eq!(*center.log.borrow(), ["status"]);
    }

    #[test]
    fn allows_authorized_and_provisional_only() {
        assert!(allows(UNAuthorizationStatus::Authorized));
        assert!(allows(UNAuthorizationStatus::Provisional));
        assert!(!allows(UNAuthorizationStatus::Denied));
        assert!(!allows(UNAuthorizationStatus::NotDetermined));
    }
}
