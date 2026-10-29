use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::mpsc::Sender;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use block2::{DynBlock, RcBlock};
use chrono::Local;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSCellImagePosition, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSWorkspace,
    NSWorkspaceDidWakeNotification,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSBundle, NSDictionary, NSError, NSNotification, NSObject,
    NSObjectProtocol, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer, NSURL,
};
use objc2_service_management::SMAppService;
use objc2_user_notifications::{
    UNAuthorizationOptions, UNNotificationResponse, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

use crate::config::Config;
use crate::dismiss;
use crate::fetch;
use crate::login::{self, LoginState};
use crate::menu::{self, Entry, Menu};
use crate::notify::PERMALINK_KEY;
use crate::scheduler::{self, Phase};
use crate::state::{Cache, SeenKey, Store};

const FETCH_INTERVAL: Duration = Duration::from_secs(300);
// 取得中の絵を差し替える間隔（秒）。12 枚で 1 周およそ 0.6 秒
const ANIMATION_INTERVAL: f64 = 0.05;
// search.messages の呼び出し回数の上限に届かないよう、続けて取得するときはこれだけ空ける
const FETCH_MIN_GAP: Duration = Duration::from_secs(30);

struct Ui {
    status_item: Retained<NSStatusItem>,
    target: Retained<MenuTarget>,
    failures: Vec<String>,
    phase: Phase,
    badge: String,
    icons: Icons,
    frame: usize,
    animation: Option<Retained<NSTimer>>,
    /// 取得が終わり、縦に戻る絵まで回してから止めるところ
    stopping: bool,
}

/// メニューバーの絵。atme.app の Resources から読む。読めなければ文字の @ で代わりにする
struct Icons {
    idle: Option<Retained<NSImage>>,
    loading: Vec<Retained<NSImage>>,
}

impl Icons {
    fn load() -> Self {
        Self {
            idle: load_icon("menubar-idle"),
            loading: (0..12)
                .filter_map(|step| load_icon(&format!("menubar-rotate-{step}")))
                .collect(),
        }
    }
}

fn load_icon(name: &str) -> Option<Retained<NSImage>> {
    let image = NSImage::imageNamed(&NSString::from_str(name))?;
    image.setTemplate(true);
    image.setSize(NSSize::new(18.0, 18.0));
    Some(image)
}

thread_local! {
    // メインスレッドだけが触る。取得のスレッドからは DispatchQueue::main 経由で refresh を呼ぶ
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

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
            if let Some(permalink) = permalink(&info) {
                // 記録の書き込みとメニューの作り直しはメインスレッドで行う
                DispatchQueue::main().exec_async(move || open_and_dismiss(&permalink));
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
    #[ivars = Sender<()>]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(fetchNow:))]
        fn fetch_now(&self, _sender: &NSMenuItem) {
            let _ = self.ivars().send(());
        }

        #[unsafe(method(didWake:))]
        fn did_wake(&self, _notification: &NSNotification) {
            let _ = self.ivars().send(());
        }

        #[unsafe(method(openPermalink:))]
        fn open_permalink(&self, sender: &NSMenuItem) {
            if let Some(permalink) = sender.representedObject().and_then(text_from) {
                open_and_dismiss(&permalink);
            }
        }

        #[unsafe(method(dismissAll:))]
        fn dismiss_all(&self, _sender: &NSMenuItem) {
            dismiss_with(dismiss::all_keys);
            refresh();
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: &NSMenuItem) {
            let service = unsafe { SMAppService::mainAppService() };
            let result = match login::action(login_state()) {
                login::Action::Register => unsafe { service.registerAndReturnError() },
                login::Action::Unregister => unsafe { service.unregisterAndReturnError() },
                login::Action::OpenSettings => {
                    unsafe { SMAppService::openSystemSettingsLoginItems() };
                    Ok(())
                }
            };
            if let Err(error) = result {
                UI.with(|ui| {
                    if let Some(ui) = ui.borrow_mut().as_mut() {
                        ui.failures.push(format!(
                            "ログイン時の起動を切り替えられませんでした（{}）",
                            error.localizedDescription()
                        ));
                    }
                });
            }
            refresh();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &NSMenuItem) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker, trigger: Sender<()>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(trigger);
        unsafe { msg_send![super(this), init] }
    }
}

fn login_state() -> LoginState {
    login::state(unsafe { SMAppService::mainAppService().status() })
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

    let (trigger, _worker) = scheduler::spawn(
        FETCH_INTERVAL,
        FETCH_MIN_GAP,
        || {
            let failures = menu::failure_lines(&fetch::fetch_once());
            DispatchQueue::main().exec_async(move || on_fetched(failures));
        },
        |phase| DispatchQueue::main().exec_async(move || on_phase(phase)),
    );
    let target = MenuTarget::new(mtm, trigger);
    let observer: &AnyObject = &target;
    unsafe {
        NSWorkspace::sharedWorkspace()
            .notificationCenter()
            .addObserver_selector_name_object(
                observer,
                sel!(didWake:),
                Some(NSWorkspaceDidWakeNotification),
                None,
            );
    }
    let status_item =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = status_item.button(mtm) {
        button.setImagePosition(NSCellImagePosition::ImageLeft);
    }
    UI.with(|ui| {
        *ui.borrow_mut() = Some(Ui {
            status_item,
            target,
            failures: Vec::new(),
            phase: Phase::Idle,
            badge: String::new(),
            icons: Icons::load(),
            frame: 0,
            animation: None,
            stopping: false,
        })
    });
    refresh();

    app.run();
    Ok(())
}

fn on_phase(phase: Phase) {
    UI.with(|ui| {
        if let Some(ui) = ui.borrow_mut().as_mut() {
            ui.phase = phase;
            if phase == Phase::Fetching {
                ui.stopping = false;
                if ui.animation.is_none() {
                    ui.animation = Some(start_animation());
                }
            } else if ui.animation.is_some() {
                ui.stopping = true;
            }
        }
    });
    refresh();
}

/// 取得中の絵を ANIMATION_INTERVAL ごとに差し替えるタイマー。メニューを開いているあいだも動くよう common モードに入れる
fn start_animation() -> Retained<NSTimer> {
    let tick = RcBlock::new(|_timer: NonNull<NSTimer>| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        UI.with(|ui| {
            if let Some(ui) = ui.borrow_mut().as_mut() {
                match advance(ui.frame, ui.icons.loading.len(), ui.stopping) {
                    Some(next) => ui.frame = next,
                    None => {
                        if let Some(animation) = ui.animation.take() {
                            animation.invalidate();
                        }
                        ui.frame = 0;
                        ui.stopping = false;
                    }
                }
                update_icon(ui, mtm);
            }
        });
    });
    let timer =
        unsafe { NSTimer::timerWithTimeInterval_repeats_block(ANIMATION_INTERVAL, true, &tick) };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    timer
}

/// 次に出す絵の番号。止める途中なら、縦に戻る絵（0 番）まで回ったところで None を返して止める
fn advance(frame: usize, frames: usize, stopping: bool) -> Option<usize> {
    if frames == 0 {
        return None;
    }
    let next = (frame + 1) % frames;
    (!(stopping && next == 0)).then_some(next)
}

fn update_icon(ui: &Ui, mtm: MainThreadMarker) {
    let Some(button) = ui.status_item.button(mtm) else {
        return;
    };
    let image = match (ui.animation.is_some(), ui.icons.loading.is_empty()) {
        (true, false) => Some(&ui.icons.loading[ui.frame % ui.icons.loading.len()]),
        _ => ui.icons.idle.as_ref(),
    };
    button.setImage(image.map(|image| &**image));
    let title = match image {
        Some(_) => ui.badge.clone(),
        None => format!("@ {}", ui.badge),
    };
    button.setTitle(&NSString::from_str(title.trim_end()));
}

fn on_fetched(failures: Vec<String>) {
    UI.with(|ui| {
        if let Some(ui) = ui.borrow_mut().as_mut() {
            ui.failures = failures;
        }
    });
    refresh();
}

/// キャッシュと記録を読み直してメニューを作り直す。キャッシュから消えた分の記録はここで捨てる
fn refresh() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let store = Store::open().ok();
    let cache = store
        .as_ref()
        .and_then(|store| store.load_cache().ok().flatten());
    let users = store
        .as_ref()
        .and_then(|store| store.load_users().ok().flatten())
        .unwrap_or_default();
    let mut dismissed = store
        .as_ref()
        .and_then(|store| store.load_dismissed().ok().flatten())
        .unwrap_or_default();
    if let (Some(store), Some(cache)) = (store.as_ref(), cache.as_ref()) {
        let pruned = dismiss::prune(&dismissed, cache);
        if pruned != dismissed {
            if let Err(error) = store.save_dismissed(&pruned) {
                eprintln!("{error:#}");
            }
            dismissed = pruned;
        }
    }
    let workspaces: Vec<String> = Config::load()
        .map(|config| {
            config
                .workspace
                .into_iter()
                .map(|workspace| workspace.name)
                .collect()
        })
        .unwrap_or_default();
    UI.with(|ui| {
        if let Some(ui) = ui.borrow_mut().as_mut() {
            let model = menu::build(
                &menu::Inputs {
                    cache: cache.as_ref(),
                    users: &users,
                    failures: &ui.failures,
                    dismissed: &dismissed,
                    login: login_state(),
                    fetch: ui.phase,
                    workspaces: &workspaces,
                },
                &Local,
            );
            show(ui, &model, mtm);
        }
    });
}

fn open_and_dismiss(permalink: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(permalink)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
    dismiss_with(|cache| {
        dismiss::key_for_permalink(cache, permalink)
            .into_iter()
            .collect()
    });
    refresh();
}

/// 選んだ項目を記録に足して保存する。メインスレッドからだけ呼ぶ
fn dismiss_with(select: impl FnOnce(&Cache) -> Vec<SeenKey>) {
    let Ok(store) = Store::open() else {
        return;
    };
    let Some(cache) = store.load_cache().ok().flatten() else {
        return;
    };
    let mut dismissed = store.load_dismissed().ok().flatten().unwrap_or_default();
    dismissed.extend(select(&cache));
    if let Err(error) = store.save_dismissed(&dismiss::prune(&dismissed, &cache)) {
        eprintln!("{error:#}");
    }
}

fn show(ui: &mut Ui, model: &Menu, mtm: MainThreadMarker) {
    ui.badge = model.badge.clone();
    update_icon(ui, mtm);
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for entry in &model.entries {
        menu.addItem(&menu_item(entry, &ui.target, mtm));
    }
    ui.status_item.setMenu(Some(&menu));
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
        Entry::Header(name) => {
            let item = new_item(name, None, mtm);
            item.setEnabled(false);
            item
        }
        Entry::Message { label, permalink } => {
            let item = new_item(label, Some(sel!(openPermalink:)), mtm);
            item.setIndentationLevel(1);
            let permalink = NSString::from_str(permalink);
            let permalink: &AnyObject = &permalink;
            unsafe {
                item.setTarget(Some(target));
                item.setRepresentedObject(Some(permalink));
            }
            item
        }
        Entry::NoItems => {
            let item = new_item("なし", None, mtm);
            item.setIndentationLevel(1);
            item.setEnabled(false);
            item
        }
        Entry::DismissAll => {
            let item = new_item("すべて既読にする", Some(sel!(dismissAll:)), mtm);
            unsafe { item.setTarget(Some(target)) };
            item
        }
        Entry::Fetch(phase) => {
            let title = match phase {
                Phase::Idle => "今すぐ取得",
                Phase::Waiting => "取得待機中",
                Phase::Fetching => "取得中…",
            };
            let item = new_item(title, Some(sel!(fetchNow:)), mtm);
            item.setEnabled(*phase == Phase::Idle);
            unsafe { item.setTarget(Some(target)) };
            item
        }
        Entry::LaunchAtLogin(state) => {
            let title = match state {
                LoginState::NeedsApproval => "ログイン時に起動（システム設定で承認が必要）",
                LoginState::On | LoginState::Off => "ログイン時に起動",
            };
            let item = new_item(title, Some(sel!(toggleLogin:)), mtm);
            if *state == LoginState::On {
                item.setState(NSControlStateValueOn);
            }
            unsafe { item.setTarget(Some(target)) };
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

pub fn permalink(info: &NSDictionary) -> Option<String> {
    text_from(info.objectForKey(&NSString::from_str(PERMALINK_KEY))?)
}

/// 文字列のオブジェクトなら、その文字列を返す。空文字は無いものとして扱う
fn text_from(value: Retained<AnyObject>) -> Option<String> {
    let value = value.downcast::<NSString>().ok()?.to_string();
    (!value.is_empty()).then_some(value)
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

    #[test]
    fn reads_permalink_written_by_notify() {
        let info = user_info("https://example.slack.com/archives/C1/p1");

        assert_eq!(
            permalink(&info).as_deref(),
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
    fn text_from_menu_item_object() {
        let string: Retained<AnyObject> =
            NSString::from_str("https://example.slack.com/archives/C1/p1")
                .into_super()
                .into_super();
        assert_eq!(
            text_from(string).as_deref(),
            Some("https://example.slack.com/archives/C1/p1")
        );

        let object: Retained<AnyObject> = NSObject::new().into_super();
        assert!(text_from(object).is_none());
    }

    #[test]
    fn tests_run_outside_app_bundle() {
        assert!(!in_app_bundle());
    }

    #[test]
    fn keeps_rotating_while_fetching() {
        assert_eq!(advance(0, 12, false), Some(1));
        assert_eq!(advance(11, 12, false), Some(0));
    }

    #[test]
    fn stops_only_when_back_upright() {
        assert_eq!(advance(5, 12, true), Some(6));
        assert_eq!(advance(10, 12, true), Some(11));
        assert_eq!(advance(11, 12, true), None);
    }

    #[test]
    fn stops_without_frames() {
        assert_eq!(advance(0, 0, false), None);
    }
}
