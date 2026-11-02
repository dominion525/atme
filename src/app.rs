//! 常駐のアプリ。AppKit の出来事を受けて controller に渡し、返ってきた「するべきこと」を AppKit で行うだけにする。
//! 判断は controller に、部品の形は ui に置く
use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use block2::{DynBlock, RcBlock};
use chrono::Local;
use dispatch2::DispatchQueue;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, ProtocolObject};
use objc2::{AnyThread, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSButton, NSCellImagePosition, NSImage, NSMenu, NSMenuDelegate, NSMenuItem,
    NSModalResponse, NSSecureTextField, NSStatusBar, NSStatusItem,
    NSUserInterfaceItemIdentification, NSVariableStatusItemLength, NSWindow, NSWorkspace,
    NSWorkspaceDidWakeNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSBundle, NSDictionary, NSError, NSNotification, NSObject, NSObjectProtocol,
    NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer, NSURL, ns_string,
};
use objc2_service_management::SMAppService;
use objc2_user_notifications::{
    UNAuthorizationOptions, UNNotificationResponse, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

use crate::config;
use crate::controller::{self, Effect, FETCH_MIN_GAP};
use crate::fetch;
use crate::l10n;
use crate::login::{self, LoginState};
use crate::manifest;
use crate::menu;
use crate::notify::PERMALINK_KEY;
use crate::scheduler;
use crate::setup;
use crate::state::Store;
use crate::ui;

const FETCH_INTERVAL: Duration = Duration::from_secs(300);
// 取得中の絵を差し替える間隔（秒）。12 枚で 1 周およそ 0.6 秒
const ANIMATION_INTERVAL: f64 = 0.05;

/// 常駐のあいだ持っておく AppKit の部品と、controller の状態
struct Ui {
    state: controller::State,
    status_item: Retained<NSStatusItem>,
    target: Retained<MenuTarget>,
    menu: Retained<NSMenu>,
    /// 取得を頼む先（scheduler）
    trigger: Sender<()>,
    badge: String,
    icons: Icons,
    animation: Option<Retained<NSTimer>>,
    /// Sparkle の SPUStandardUpdaterController。読み込めなかったときは None
    updater: Option<Retained<AnyObject>>,
    /// ワークスペースの追加・削除のウインドウ。1 度開いたら閉じても持っておく
    window: Option<Retained<NSWindow>>,
    token_field: Option<Retained<NSSecureTextField>>,
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
    // メインスレッドだけが触る。取得のスレッドからは DispatchQueue::main 経由で渡す
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

/// controller の状態を更新し、返ってきた「するべきこと」を行う。UI を借りたまま行わないよう、更新と実行を分ける
fn handle(update: impl FnOnce(&mut controller::State) -> Vec<Effect>) {
    let effects = UI.with(|ui| {
        ui.borrow_mut()
            .as_mut()
            .map(|ui| update(&mut ui.state))
            .unwrap_or_default()
    });
    for effect in effects {
        apply(effect);
    }
}

fn apply(effect: Effect) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    match effect {
        Effect::RefreshMenu => refresh(),
        Effect::RedrawIcon => UI.with(|ui| {
            if let Some(ui) = ui.borrow().as_ref() {
                update_icon(ui, mtm);
            }
        }),
        Effect::StartAnimation => {
            let timer = start_animation();
            UI.with(|ui| {
                if let Some(ui) = ui.borrow_mut().as_mut() {
                    ui.animation = Some(timer);
                }
            });
        }
        Effect::StopAnimation => UI.with(|ui| {
            if let Some(animation) = ui.borrow_mut().as_mut().and_then(|ui| ui.animation.take()) {
                animation.invalidate();
            }
        }),
        Effect::ShowWorkspaces => show_workspaces(),
        Effect::ClearTokenField => UI.with(|ui| {
            if let Some(field) = ui.borrow().as_ref().and_then(|ui| ui.token_field.as_ref()) {
                field.setStringValue(ns_string!(""));
            }
        }),
        // auth.test は通信するので、メインスレッドを止めないよう別のスレッドで確かめる
        Effect::CheckToken(token) => {
            std::thread::spawn(move || {
                let result = setup::add(&token).map_err(|error| format!("{error:#}"));
                DispatchQueue::main()
                    .exec_async(move || handle(|state| state.token_checked(result)));
            });
        }
        Effect::Fetch => UI.with(|ui| {
            if let Some(ui) = ui.borrow().as_ref() {
                let _ = ui.trigger.send(());
            }
        }),
    }
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
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(fetchNow:))]
        fn fetch_now(&self, _sender: &NSMenuItem) {
            apply(Effect::Fetch);
        }

        #[unsafe(method(didWake:))]
        fn did_wake(&self, _notification: &NSNotification) {
            apply(Effect::Fetch);
        }

        #[unsafe(method(openPermalink:))]
        fn open_permalink(&self, sender: &NSMenuItem) {
            if let Some(permalink) = sender.representedObject().and_then(text_from) {
                open_and_dismiss(&permalink);
            }
        }

        #[unsafe(method(dismissAll:))]
        fn dismiss_all(&self, _sender: &NSMenuItem) {
            if let Ok(store) = Store::open() {
                controller::dismiss_all(&store);
            }
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
            match result {
                Ok(()) => refresh(),
                Err(error) => {
                    let message = error.localizedDescription().to_string();
                    handle(|state| state.login_toggle_failed(&message));
                }
            }
        }

        #[unsafe(method(openWorkspaces:))]
        fn open_workspaces(&self, _sender: &NSMenuItem) {
            open_workspaces();
        }

        #[unsafe(method(addWorkspace:))]
        fn add_workspace(&self, _sender: &AnyObject) {
            let token = UI.with(|ui| {
                ui.borrow()
                    .as_ref()
                    .and_then(|ui| ui.token_field.as_ref())
                    .map(|field| field.stringValue().to_string())
            });
            if let Some(token) = token {
                handle(|state| state.add_pressed(&token));
            }
        }

        #[unsafe(method(removeWorkspace:))]
        fn remove_workspace(&self, sender: &NSButton) {
            if let Some(name) = sender.identifier().map(|name| name.to_string()) {
                confirm_removal(name);
            }
        }

        #[unsafe(method(createSlackApp:))]
        fn create_slack_app(&self, _sender: &AnyObject) {
            open_url(manifest::create_app_url().as_str());
        }

        #[unsafe(method(closeWorkspaces:))]
        fn close_workspaces(&self, _sender: &AnyObject) {
            let Some(window) = UI.with(|ui| ui.borrow().as_ref().and_then(|ui| ui.window.clone()))
            else {
                return;
            };
            match window.attachedSheet() {
                // 確認のシートが出ているときは、シートを「キャンセル」（最初のボタン）で閉じる
                Some(sheet) => window.endSheet_returnCode(&sheet, ui::REMOVAL_CANCEL),
                None if window.isKeyWindow() => window.performClose(None),
                None => {}
            }
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &NSMenuItem) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }

    unsafe impl NSObjectProtocol for MenuTarget {}

    unsafe impl NSMenuDelegate for MenuTarget {
        // 開く直前に作り直し、「あと N 秒」などをその時点の値にする
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, _menu: &NSMenu) {
            refresh();
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn login_state() -> LoginState {
    login::state(unsafe { SMAppService::mainAppService().status() })
}

pub fn in_app_bundle() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// atme.app の Contents/Frameworks から Sparkle を読み込み、自動アップデートを始める。
/// ビルド時にはリンクしないので、Sparkle の無いところ（テストや .app の外）でも atme は動く
fn start_updater() -> Option<Retained<AnyObject>> {
    let frameworks = NSBundle::mainBundle().privateFrameworksPath()?;
    let path = NSString::from_str(&format!("{frameworks}/Sparkle.framework"));
    let bundle = NSBundle::bundleWithPath(&path)?;
    if !unsafe { bundle.load() } {
        eprintln!("{}", l10n::text(l10n::SPARKLE_LOAD_FAILED));
        return None;
    }
    let class = AnyClass::get(c"SPUStandardUpdaterController")?;
    let allocated: Allocated<AnyObject> = unsafe { msg_send![class, alloc] };
    let no_delegate: Option<&AnyObject> = None;
    unsafe {
        msg_send![
            allocated,
            initWithStartingUpdater: true,
            updaterDelegate: no_delegate,
            userDriverDelegate: no_delegate
        ]
    }
}

pub fn run() -> Result<()> {
    if !in_app_bundle() {
        bail!(l10n::text(l10n::OUTSIDE_APP_RESIDENT));
    }
    let mtm = MainThreadMarker::new().with_context(|| l10n::text(l10n::NOT_MAIN_THREAD))?;
    let app = NSApplication::sharedApplication(mtm);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = NotificationDelegate::new();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    request_authorization(&center);

    // ATME_NO_FETCH があるときは取得しない。README の画像を、見本のキャッシュだけで撮るため
    let (trigger, _worker) = if std::env::var_os("ATME_NO_FETCH").is_some() {
        let (trigger, _ignored) = std::sync::mpsc::channel();
        (trigger, None)
    } else {
        let (trigger, worker) = scheduler::spawn(
            FETCH_INTERVAL,
            FETCH_MIN_GAP,
            || {
                let failures = menu::failure_lines(&fetch::fetch_once());
                DispatchQueue::main().exec_async(move || handle(|state| state.fetched(failures)));
            },
            |phase| {
                DispatchQueue::main()
                    .exec_async(move || handle(|state| state.phase_changed(phase, Instant::now())))
            },
        );
        (trigger, Some(worker))
    };
    let target = MenuTarget::new(mtm);
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
    // メニューは 1 つだけ作り、作り直すときは中の項目を入れ替える
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    status_item.setMenu(Some(&menu));
    app.setMainMenu(Some(&ui::main_menu(observer, mtm)));
    UI.with(|ui| {
        *ui.borrow_mut() = Some(Ui {
            state: controller::State::default(),
            status_item,
            target,
            menu,
            trigger,
            badge: String::new(),
            icons: Icons::load(),
            animation: None,
            updater: start_updater(),
            window: None,
            token_field: None,
        })
    });
    refresh();
    if config::path().map_or(true, |path| controller::needs_first_workspace(&path)) {
        open_workspaces();
    }

    app.run();
    Ok(())
}

/// 取得中の絵を ANIMATION_INTERVAL ごとに進めるタイマー。メニューを開いているあいだも動くよう common モードに入れる
fn start_animation() -> Retained<NSTimer> {
    let tick = RcBlock::new(|_timer: NonNull<NSTimer>| {
        let frames = UI.with(|ui| ui.borrow().as_ref().map_or(0, |ui| ui.icons.loading.len()));
        handle(|state| state.tick(frames));
    });
    let timer =
        unsafe { NSTimer::timerWithTimeInterval_repeats_block(ANIMATION_INTERVAL, true, &tick) };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    timer
}

fn update_icon(ui: &Ui, mtm: MainThreadMarker) {
    let Some(button) = ui.status_item.button(mtm) else {
        return;
    };
    let (icon, title) = controller::icon(
        &ui.state,
        ui.icons.loading.len(),
        ui.icons.idle.is_some(),
        &ui.badge,
    );
    let image = match icon {
        controller::Icon::Turning(frame) => Some(&ui.icons.loading[frame]),
        controller::Icon::Idle => ui.icons.idle.as_ref(),
        controller::Icon::Text => None,
    };
    button.setImage(image.map(|image| &**image));
    button.setTitle(&NSString::from_str(&title));
}

/// キャッシュと記録を読み直して、メニューとメニューバーの件数を作り直す
fn refresh() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = match (Store::open(), config::path()) {
        (Ok(store), Ok(path)) => controller::menu_data(&store, &path),
        _ => controller::MenuData::default(),
    };
    let login = login_state();
    UI.with(|ui| {
        let mut ui = ui.borrow_mut();
        let Some(ui) = ui.as_mut() else {
            return;
        };
        let model = menu::build(
            &menu::Inputs {
                cache: data.cache.as_ref(),
                users: &data.users,
                failures: &ui.state.failures,
                dismissed: &data.dismissed,
                login,
                updates: ui.updater.is_some(),
                fetch: ui.state.fetch_button(Instant::now()),
                workspaces: &data.workspaces,
            },
            &Local,
        );
        ui.badge = model.badge.clone();
        update_icon(ui, mtm);
        ui.menu.removeAllItems();
        let target: &AnyObject = &ui.target;
        for entry in &model.entries {
            ui.menu
                .addItem(&ui::menu_item(entry, target, ui.updater.as_deref(), mtm));
        }
    });
}

fn open_url(url: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

/// メッセージを Slack で開き、atme の一覧から外す
fn open_and_dismiss(permalink: &str) {
    open_url(permalink);
    if let Ok(store) = Store::open() {
        controller::dismiss_permalink(&store, permalink);
    }
    refresh();
}

/// 「削除」を押したときに、ワークスペースのウインドウに確認のシートを出し、「削除」を選んだときだけ外す
fn confirm_removal(name: String) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some((window, target)) = UI.with(|ui| {
        ui.borrow()
            .as_ref()
            .and_then(|ui| Some((ui.window.clone()?, ui.target.clone())))
    }) else {
        return;
    };
    let shown = Store::open().map_or_else(
        |_| name.clone(),
        |store| controller::shown_name(&store, &name),
    );
    let alert = ui::removal_alert(&shown, &target, mtm);
    let handler = RcBlock::new(move |response: NSModalResponse| {
        if ui::removal_confirmed(response) {
            let result = setup::remove(&name).map_err(|error| format!("{error:#}"));
            handle(|state| state.removed(result));
        }
    });
    alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
}

/// ワークスペースのウインドウを開いて前に出す
fn open_workspaces() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    show_workspaces();
    UI.with(|ui| {
        if let Some(window) = ui.borrow().as_ref().and_then(|ui| ui.window.clone()) {
            window.makeKeyAndOrderFront(None);
        }
    });
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}

/// ワークスペースのウインドウの中身を、今の設定と状態から作り直す。まだ開いていなければ作る
fn show_workspaces() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let workspaces = match (Store::open(), config::path()) {
        (Ok(store), Ok(path)) => controller::shown_workspaces(&store, &path),
        _ => Vec::new(),
    };
    UI.with(|ui| {
        let mut ui = ui.borrow_mut();
        let Some(ui) = ui.as_mut() else {
            return;
        };
        let field = ui
            .token_field
            .get_or_insert_with(|| ui::token_field(mtm))
            .clone();
        let target: &AnyObject = &ui.target;
        let (content, height) = ui::workspace_content(
            &workspaces,
            &ui.state.setup_status,
            ui.state.adding,
            &field,
            target,
            mtm,
        );
        let window = ui
            .window
            .get_or_insert_with(|| ui::workspace_window(height, mtm))
            .clone();
        window.setContentView(Some(&content));
        window.setContentSize(NSSize::new(ui::WINDOW_WIDTH, height));
        window.center();
        window.makeFirstResponder(Some(&field));
    });
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
                "{}",
                l10n::format(
                    l10n::NOTIFICATION_REQUEST_FAILED,
                    &[&error.localizedDescription().to_string()],
                )
            );
        } else if !granted.as_bool() {
            eprintln!("{}", l10n::text(l10n::NOTIFICATIONS_NOT_ALLOWED_SETTINGS));
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
}
