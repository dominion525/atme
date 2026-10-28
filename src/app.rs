use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread};
use objc2_app_kit::{NSApplication, NSWorkspace};
use objc2_foundation::{
    MainThreadMarker, NSBundle, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString, NSURL,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNNotificationResponse, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

use crate::notify::PERMALINK_KEY;

const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(120);
const CLICK_TIMEOUT: Duration = Duration::from_secs(10);

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
            std::process::exit(0);
        }
    }
);

impl NotificationDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

pub fn in_app_bundle() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

pub fn run() -> Result<()> {
    if !in_app_bundle() {
        bail!("atme.app の外から実行したため、通知の許可を求められません");
    }
    let mtm = MainThreadMarker::new().context("メインスレッドで動いていません")?;
    let app = NSApplication::sharedApplication(mtm);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = NotificationDelegate::new();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    request_authorization(&center)?;

    thread::spawn(|| {
        thread::sleep(CLICK_TIMEOUT);
        std::process::exit(0);
    });
    app.run();
    Ok(())
}

pub fn permalink(info: &NSDictionary) -> Option<Retained<NSURL>> {
    let value = info.objectForKey(&NSString::from_str(PERMALINK_KEY))?;
    let value = value.downcast::<NSString>().ok()?;
    if value.length() == 0 {
        return None;
    }
    NSURL::URLWithString(&value)
}

fn request_authorization(center: &UNUserNotificationCenter) -> Result<()> {
    let (sender, receiver) = mpsc::channel();
    let handler = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        let error = unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
        let _ = sender.send((granted.as_bool(), error));
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &handler,
    );
    let (granted, error) = receiver
        .recv_timeout(AUTHORIZATION_TIMEOUT)
        .context("通知の許可の応答がありませんでした")?;
    if let Some(error) = error {
        bail!("通知の許可を求められませんでした: {error}");
    }
    if !granted {
        bail!("通知が許可されていません。システム設定の「通知」で atme を許可してください");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::user_info;

    #[test]
    fn reads_permalink_written_by_notify() {
        let info = user_info("https://example.slack.com/archives/C1/p1");

        let url = permalink(&info)
            .and_then(|url| url.absoluteString())
            .map(|url| url.to_string());

        assert_eq!(
            url.as_deref(),
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
    fn tests_run_outside_app_bundle() {
        assert!(!in_app_bundle());
    }
}
