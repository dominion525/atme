use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::NSError;
use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};

const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(120);

pub fn run() -> Result<()> {
    let center = UNUserNotificationCenter::currentNotificationCenter();
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
