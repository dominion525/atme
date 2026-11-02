use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use block2::{DynBlock, RcBlock};
use chrono::Local;
use dispatch2::DispatchQueue;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, ProtocolObject};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSApplication, NSButton,
    NSCellImagePosition, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSModalResponse,
    NSSecureTextField, NSStatusBar, NSStatusItem, NSUserInterfaceItemIdentification,
    NSVariableStatusItemLength, NSWindow, NSWorkspace, NSWorkspaceDidWakeNotification,
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
use crate::l10n;
use crate::login::{self, LoginState};
use crate::manifest;
use crate::menu::{self, Menu};
use crate::notify::PERMALINK_KEY;
use crate::scheduler::{self, Phase};
use crate::setup;
use crate::state::{Cache, SeenKey, Store};

const FETCH_INTERVAL: Duration = Duration::from_secs(300);
// 取得中の絵を差し替える間隔（秒）。12 枚で 1 周およそ 0.6 秒
const ANIMATION_INTERVAL: f64 = 0.05;
// search.messages の呼び出し回数の上限に届かないよう、続けて取得するときはこれだけ空ける
const FETCH_MIN_GAP: Duration = Duration::from_secs(30);

struct Ui {
    status_item: Retained<NSStatusItem>,
    target: Retained<MenuTarget>,
    menu: Retained<NSMenu>,
    failures: Vec<String>,
    phase: Phase,
    /// 最後に取得が終わった時刻。「今すぐ取得（あと N 秒）」に使う
    last_fetched: Option<Instant>,
    badge: String,
    icons: Icons,
    frame: usize,
    animation: Option<Retained<NSTimer>>,
    /// 取得が終わり、縦に戻る絵まで回してから止めるところ
    stopping: bool,
    /// Sparkle の SPUStandardUpdaterController。読み込めなかったときは None
    updater: Option<Retained<AnyObject>>,
    /// ワークスペースの追加・削除のウインドウ。1 度開いたら閉じても持っておく
    window: Option<Retained<NSWindow>>,
    token_field: Option<Retained<NSSecureTextField>>,
    /// ウインドウの下に出す、確認中・追加できた・失敗の文
    setup_status: crate::ui::Status,
    /// token を確かめているあいだは「追加」を押せなくする
    adding: bool,
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
                        ui.failures.push(l10n::format(
                            l10n::LOGIN_TOGGLE_FAILED,
                            &[&error.localizedDescription().to_string()],
                        ));
                    }
                });
            }
            refresh();
        }

        #[unsafe(method(openWorkspaces:))]
        fn open_workspaces(&self, _sender: &NSMenuItem) {
            open_workspaces();
        }

        #[unsafe(method(addWorkspace:))]
        fn add_workspace(&self, _sender: &AnyObject) {
            let token = UI.with(|ui| {
                let mut ui = ui.borrow_mut();
                let ui = ui.as_mut()?;
                let token = ui.token_field.as_ref()?.stringValue().to_string();
                if ui.adding || token.trim().is_empty() {
                    return None;
                }
                ui.adding = true;
                ui.setup_status = crate::ui::Status::Info(l10n::text(l10n::CHECKING));
                Some(token)
            });
            let Some(token) = token else {
                return;
            };
            show_workspaces();
            // auth.test は通信するので、メインスレッドを止めないよう別のスレッドで確かめる
            let trigger = self.ivars().clone();
            std::thread::spawn(move || {
                let result = setup::add(&token).map_err(|error| format!("{error:#}"));
                DispatchQueue::main().exec_async(move || {
                    let added = result.is_ok();
                    UI.with(|ui| {
                        if let Some(ui) = ui.borrow_mut().as_mut() {
                            ui.adding = false;
                            ui.setup_status = match &result {
                                Ok(team) => crate::ui::Status::Info(l10n::format(l10n::ADDED, &[team])),
                                Err(error) => crate::ui::Status::Error(error.clone()),
                            };
                            if let (true, Some(field)) = (added, &ui.token_field) {
                                field.setStringValue(ns_string!(""));
                            }
                        }
                    });
                    show_workspaces();
                    if added {
                        let _ = trigger.send(());
                    }
                });
            });
        }

        #[unsafe(method(removeWorkspace:))]
        fn remove_workspace(&self, sender: &NSButton) {
            let Some(name) = sender.identifier().map(|name| name.to_string()) else {
                return;
            };
            confirm_removal(name, self.ivars().clone());
        }

        #[unsafe(method(createSlackApp:))]
        fn create_slack_app(&self, _sender: &AnyObject) {
            let url = manifest::create_app_url();
            if let Some(url) = NSURL::URLWithString(&NSString::from_str(url.as_str())) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }

        #[unsafe(method(closeWorkspaces:))]
        fn close_workspaces(&self, _sender: &AnyObject) {
            let Some(window) = UI.with(|ui| ui.borrow().as_ref().and_then(|ui| ui.window.clone()))
            else {
                return;
            };
            match window.attachedSheet() {
                // 確認のシートが出ているときは、シートを「キャンセル」（最初のボタン）で閉じる
                Some(sheet) => window.endSheet_returnCode(&sheet, NSAlertFirstButtonReturn),
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
                DispatchQueue::main().exec_async(move || on_fetched(failures));
            },
            |phase| DispatchQueue::main().exec_async(move || on_phase(phase)),
        );
        (trigger, Some(worker))
    };
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
    // メニューは 1 つだけ作り、作り直すときは中の項目を入れ替える
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    status_item.setMenu(Some(&menu));
    let menu_target: &AnyObject = &target;
    app.setMainMenu(Some(&crate::ui::main_menu(menu_target, mtm)));
    UI.with(|ui| {
        *ui.borrow_mut() = Some(Ui {
            status_item,
            target,
            menu,
            failures: Vec::new(),
            phase: Phase::Idle,
            last_fetched: None,
            badge: String::new(),
            icons: Icons::load(),
            frame: 0,
            animation: None,
            stopping: false,
            updater: start_updater(),
            window: None,
            token_field: None,
            setup_status: crate::ui::Status::None,
            adding: false,
        })
    });
    refresh();
    // 初めて起動したとき（ワークスペースが 1 つも無いとき）は、追加のウインドウを開く
    if Config::load().map_or(true, |config| config.workspace.is_empty()) {
        open_workspaces();
    }

    app.run();
    Ok(())
}

fn on_phase(phase: Phase) {
    UI.with(|ui| {
        if let Some(ui) = ui.borrow_mut().as_mut() {
            if ui.phase == Phase::Fetching && phase != Phase::Fetching {
                ui.last_fetched = Some(Instant::now());
            }
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
                    updates: ui.updater.is_some(),
                    fetch: menu::fetch_button(
                        ui.phase,
                        ui.last_fetched.map(|finished| finished.elapsed()),
                        FETCH_MIN_GAP,
                    ),
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
    ui.menu.removeAllItems();
    let target: &AnyObject = &ui.target;
    for entry in &model.entries {
        ui.menu
            .addItem(&crate::ui::menu_item(entry, target, ui.updater.as_deref(), mtm));
    }
}


/// 「削除」を押したときに、ワークスペースのウインドウに確認のシートを出す。
/// 「削除」を選んだときだけ外す。既定のボタン（Return）は「キャンセル」。⌘W と Esc もキャンセルになる（closeWorkspaces:）
fn confirm_removal(name: String, trigger: Sender<()>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = UI.with(|ui| ui.borrow().as_ref().and_then(|ui| ui.window.clone())) else {
        return;
    };
    let shown = Store::open()
        .ok()
        .and_then(|store| store.load_cache().ok().flatten())
        .and_then(|cache| cache.teams.get(&name).cloned())
        .unwrap_or_else(|| name.clone());
    let Some(target) = UI.with(|ui| ui.borrow().as_ref().map(|ui| ui.target.clone())) else {
        return;
    };
    let alert = crate::ui::removal_alert(&shown, &target, mtm);
    let handler = RcBlock::new(move |response: NSModalResponse| {
        if response == NSAlertSecondButtonReturn {
            remove_now(&name, &trigger);
        }
    });
    alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
}

/// 設定ファイルと Keychain から外し、ウインドウを作り直して取得し直す
fn remove_now(name: &str, trigger: &Sender<()>) {
    let status = match setup::remove(name) {
        Ok(()) => crate::ui::Status::None,
        Err(error) => crate::ui::Status::Error(format!("{error:#}")),
    };
    UI.with(|ui| {
        if let Some(ui) = ui.borrow_mut().as_mut() {
            ui.setup_status = status;
        }
    });
    show_workspaces();
    let _ = trigger.send(());
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

/// ワークスペースのウインドウの中身を、今の設定から作り直す。まだ開いていなければ作る
fn show_workspaces() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let teams = Store::open()
        .ok()
        .and_then(|store| store.load_cache().ok().flatten())
        .map(|cache| cache.teams)
        .unwrap_or_default();
    // 設定の name は内部の名前なので、Slack 上の名前が分かればそちらだけを出す
    let workspaces: Vec<crate::ui::ShownWorkspace> = Config::load()
        .map(|config| {
            config
                .workspace
                .into_iter()
                .map(|workspace| crate::ui::ShownWorkspace {
                    shown: teams.get(&workspace.name).unwrap_or(&workspace.name).clone(),
                    name: workspace.name,
                })
                .collect()
        })
        .unwrap_or_default();
    UI.with(|ui| {
        let mut ui = ui.borrow_mut();
        let Some(ui) = ui.as_mut() else {
            return;
        };
        let field = ui
            .token_field
            .get_or_insert_with(|| crate::ui::token_field(mtm))
            .clone();
        let target: &AnyObject = &ui.target;
        let (content, height) = crate::ui::workspace_content(
            &workspaces,
            &ui.setup_status,
            ui.adding,
            &field,
            target,
            mtm,
        );
        let window = ui
            .window
            .get_or_insert_with(|| crate::ui::workspace_window(height, mtm))
            .clone();
        window.setContentView(Some(&content));
        window.setContentSize(NSSize::new(crate::ui::WINDOW_WIDTH, height));
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
