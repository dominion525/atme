use anyhow::{bail, Context, Result};
use block2::{DynBlock, RcBlock};
use chrono::Local;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    NSWorkspace,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSBundle, NSDictionary, NSError, NSObject, NSObjectProtocol,
    NSString, NSURL,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNNotificationResponse, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

use crate::menu::{self, Entry, Menu};
use crate::notify::PERMALINK_KEY;
use crate::state::Store;

define_class!(
    // SAFETY: NSObject にサブクラスの条件は無く、この型は Drop を実装しない
    #[unsafe(super = NSObject)]
    #[name = "AtmeNotificationDelegate"]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion_handler: &DynBlock<dyn Fn()>,
        ) {
            let info = response.notification().request().content().userInfo();
            if let Some(url) = permalink(&info) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
            completion_handler.call(());
        }
    }
);

impl NotificationDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    // SAFETY: NSObject にサブクラスの条件は無く、この型は Drop を実装しない
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "AtmeMenuTarget"]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(openPermalink:))]
        fn open_permalink(&self, sender: &NSMenuItem) {
            if let Some(url) = sender.representedObject().and_then(url_from) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &NSMenuItem) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

pub fn in_app_bundle() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

pub fn run() -> Result<()> {
    if !in_app_bundle() {
        bail!("atme.app の外から実行したため、常駐できません");
    }
    let mtm = MainThreadMarker::new().context("メインスレッドで動いていません")?;
    let app = NSApplication::sharedApplication(mtm);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = NotificationDelegate::new();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    request_authorization(&center);

    let store = Store::open()?;
    let cache = store.load_cache().unwrap_or_else(|error| {
        eprintln!("{error:#}");
        None
    });
    let users = store.load_users().ok().flatten().unwrap_or_default();
    let target = MenuTarget::new(mtm);
    let status_item =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    show(
        &status_item,
        &menu::build(cache.as_ref(), &users, &Local),
        &target,
        mtm,
    );

    app.run();
    Ok(())
}

fn show(status_item: &NSStatusItem, model: &Menu, target: &MenuTarget, mtm: MainThreadMarker) {
    if let Some(button) = status_item.button(mtm) {
        button.setTitle(&NSString::from_str(&model.title));
    }
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for entry in &model.entries {
        menu.addItem(&menu_item(entry, target, mtm));
    }
    status_item.setMenu(Some(&menu));
}

fn menu_item(entry: &Entry, target: &MenuTarget, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let target: &AnyObject = target;
    match entry {
        Entry::Separator => NSMenuItem::separatorItem(mtm),
        Entry::Info(text) => {
            let item = new_item(text, None, mtm);
            item.setEnabled(false);
            item
        }
        Entry::Message { label, permalink } => {
            let item = new_item(label, Some(sel!(openPermalink:)), mtm);
            let permalink = NSString::from_str(permalink);
            let permalink: &AnyObject = &permalink;
            unsafe {
                item.setTarget(Some(target));
                item.setRepresentedObject(Some(permalink));
            }
            item
        }
        Entry::Quit => {
            let item = new_item("終了", Some(sel!(quit:)), mtm);
            unsafe { item.setTarget(Some(target)) };
            item
        }
    }
}

fn new_item(title: &str, action: Option<Sel>, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            ns_string!(""),
        )
    }
}

pub fn permalink(info: &NSDictionary) -> Option<Retained<NSURL>> {
    url_from(info.objectForKey(&NSString::from_str(PERMALINK_KEY))?)
}

fn url_from(value: Retained<AnyObject>) -> Option<Retained<NSURL>> {
    let value = value.downcast::<NSString>().ok()?;
    if value.length() == 0 {
        return None;
    }
    NSURL::URLWithString(&value)
}

fn request_authorization(center: &UNUserNotificationCenter) {
    let handler = RcBlock::new(|granted: Bool, error: *mut NSError| {
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!(
                "通知の許可を求められませんでした: {}",
                error.localizedDescription()
            );
        } else if !granted.as_bool() {
            eprintln!("通知が許可されていません。システム設定の「通知」で atme を許可してください");
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &handler,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::user_info;

    fn absolute(url: Option<Retained<NSURL>>) -> Option<String> {
        url.and_then(|url| url.absoluteString())
            .map(|url| url.to_string())
    }

    #[test]
    fn reads_permalink_written_by_notify() {
        let info = user_info("https://example.slack.com/archives/C1/p1");

        assert_eq!(
            absolute(permalink(&info)).as_deref(),
            Some("https://example.slack.com/archives/C1/p1")
        );
    }

    #[test]
    fn no_permalink_without_key() {
        assert!(permalink(&NSDictionary::new()).is_none());
    }

    #[test]
    fn no_permalink_for_empty_value() {
        assert!(permalink(&user_info("")).is_none());
    }

    #[test]
    fn url_from_menu_item_object() {
        let string: Retained<AnyObject> =
            NSString::from_str("https://example.slack.com/archives/C1/p1")
                .into_super()
                .into_super();
        assert_eq!(
            absolute(url_from(string)).as_deref(),
            Some("https://example.slack.com/archives/C1/p1")
        );

        let object: Retained<AnyObject> = NSObject::new().into_super();
        assert!(url_from(object).is_none());
    }

    #[test]
    fn tests_run_outside_app_bundle() {
        assert!(!in_app_bundle());
    }
}
