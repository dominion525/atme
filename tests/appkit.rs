//! メニューやウインドウの部品を、メインスレッドで直に作って確かめる（Cargo.toml の harness = false）。
//! 文言は .app の外なので英語（l10n のキーそのもの）になる
use std::panic::{catch_unwind, AssertUnwindSafe};

use atme::login::LoginState;
use atme::menu::{Entry, FetchButton};
use atme::ui::{self, ShownWorkspace, Status};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::sel;
use objc2_app_kit::{
    NSAlertFirstButtonReturn, NSApplication, NSButton, NSColor, NSControlStateValueOff, NSControlStateValueOn,
    NSEventModifierFlags, NSMenu, NSMenuItem, NSTextField, NSUserInterfaceItemIdentification,
    NSView, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSString};

fn main() {
    let mtm = MainThreadMarker::new().expect("テストはメインスレッドで動かす");
    NSApplication::sharedApplication(mtm);
    let tests: &[(&str, fn(MainThreadMarker))] = &[
        ("message_opens_its_permalink", message_opens_its_permalink),
        ("headers_and_info_cannot_be_chosen", headers_and_info_cannot_be_chosen),
        ("fetch_button_follows_its_state", fetch_button_follows_its_state),
        ("launch_at_login_shows_its_state", launch_at_login_shows_its_state),
        ("check_for_updates_goes_to_sparkle", check_for_updates_goes_to_sparkle),
        ("other_items_go_to_the_target", other_items_go_to_the_target),
        ("main_menu_has_paste_and_close", main_menu_has_paste_and_close),
        ("removal_alert_defaults_to_cancel", removal_alert_defaults_to_cancel),
        ("only_the_remove_button_removes", only_the_remove_button_removes),
        ("window_without_workspaces", window_without_workspaces),
        ("window_lists_workspaces_with_remove", window_lists_workspaces_with_remove),
        ("window_locks_input_while_adding", window_locks_input_while_adding),
        ("window_shows_errors_in_red", window_shows_errors_in_red),
        ("window_has_the_steps_and_escape", window_has_the_steps_and_escape),
        ("window_title_and_close_button", window_title_and_close_button),
    ];
    let mut failed = Vec::new();
    for (name, test) in tests {
        match catch_unwind(AssertUnwindSafe(|| test(mtm))) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(*name);
            }
        }
    }
    println!(
        "\ntest result: {}. {} passed; {} failed",
        if failed.is_empty() { "ok" } else { "FAILED" },
        tests.len() - failed.len(),
        failed.len()
    );
    if !failed.is_empty() {
        std::process::exit(1);
    }
}

/// ボタンや項目の押し先にする、何もしないオブジェクト
fn target() -> Retained<NSObject> {
    NSObject::new()
}

fn is(object: Option<Retained<AnyObject>>, expected: &AnyObject) -> bool {
    object.is_some_and(|object| std::ptr::eq(&*object, expected))
}

fn item(entry: Entry, target: &AnyObject, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    ui::menu_item(&entry, target, None, mtm)
}

fn action(item: &NSMenuItem) -> Option<Sel> {
    item.action()
}

fn message_opens_its_permalink(mtm: MainThreadMarker) {
    let target = target();
    let item = item(
        Entry::Message {
            label: "#general  Sato: hi".into(),
            permalink: "https://acme.slack.com/archives/C1/p1".into(),
        },
        &target,
        mtm,
    );

    assert_eq!(item.title().to_string(), "#general  Sato: hi");
    assert_eq!(action(&item), Some(sel!(openPermalink:)));
    assert!(is(item.target(), &target));
    assert_eq!(item.indentationLevel(), 1);
    let permalink = item.representedObject().unwrap().downcast::<NSString>().unwrap();
    assert_eq!(permalink.to_string(), "https://acme.slack.com/archives/C1/p1");
}

fn headers_and_info_cannot_be_chosen(mtm: MainThreadMarker) {
    let target = target();
    for entry in [
        Entry::Header("Acme".into()),
        Entry::Info("Fetched 11/01 12:00".into()),
        Entry::NoItems,
    ] {
        assert!(!item(entry, &target, mtm).isEnabled());
    }
    assert_eq!(item(Entry::NoItems, &target, mtm).title().to_string(), "None");
    assert!(item(Entry::Separator, &target, mtm).isSeparatorItem());
}

fn fetch_button_follows_its_state(mtm: MainThreadMarker) {
    let target = target();
    let cases = [
        (FetchButton::Ready, "Fetch Now", true),
        (FetchButton::Cooldown(12), "Fetch Now (in 12s)", false),
        (FetchButton::Waiting, "Waiting to Fetch", false),
        (FetchButton::Fetching, "Fetching…", false),
    ];
    for (button, title, enabled) in cases {
        let item = item(Entry::Fetch(button), &target, mtm);
        assert_eq!(item.title().to_string(), title);
        assert_eq!(item.isEnabled(), enabled, "{title}");
        assert_eq!(action(&item), Some(sel!(fetchNow:)));
    }
}

fn launch_at_login_shows_its_state(mtm: MainThreadMarker) {
    let target = target();
    let on = item(Entry::LaunchAtLogin(LoginState::On), &target, mtm);
    let off = item(Entry::LaunchAtLogin(LoginState::Off), &target, mtm);
    let approval = item(Entry::LaunchAtLogin(LoginState::NeedsApproval), &target, mtm);

    assert_eq!(on.state(), NSControlStateValueOn);
    assert_eq!(off.state(), NSControlStateValueOff);
    assert_eq!(
        approval.title().to_string(),
        "Open at Login (needs approval in System Settings)"
    );
    assert_eq!(action(&on), Some(sel!(toggleLogin:)));
}

fn check_for_updates_goes_to_sparkle(mtm: MainThreadMarker) {
    let target = target();
    let updater = NSObject::new();
    let with = ui::menu_item(&Entry::CheckForUpdates, &target, Some(&updater), mtm);
    let without = ui::menu_item(&Entry::CheckForUpdates, &target, None, mtm);

    assert_eq!(with.title().to_string(), "Check for Updates…");
    assert_eq!(action(&with), Some(sel!(checkForUpdates:)));
    assert!(is(with.target(), &updater));
    assert!(without.target().is_none());
}

fn other_items_go_to_the_target(mtm: MainThreadMarker) {
    let target = target();
    let cases = [
        (Entry::DismissAll, "Mark All as Read", sel!(dismissAll:)),
        (Entry::Workspaces, "Workspaces…", sel!(openWorkspaces:)),
        (Entry::Quit, "Quit", sel!(quit:)),
    ];
    for (entry, title, selector) in cases {
        let item = item(entry, &target, mtm);
        assert_eq!(item.title().to_string(), title);
        assert_eq!(action(&item), Some(selector));
        assert!(is(item.target(), &target));
    }
}

/// メニューの中から、action が selector の項目を探す（下の階層も見る）
fn find(menu: &NSMenu, selector: Sel) -> Option<Retained<NSMenuItem>> {
    menu.itemArray().iter().find_map(|item| {
        if action(&item) == Some(selector) {
            return Some(item);
        }
        item.submenu().and_then(|submenu| find(&submenu, selector))
    })
}

fn main_menu_has_paste_and_close(mtm: MainThreadMarker) {
    let target = target();
    let menu = ui::main_menu(&target, mtm);

    let paste = find(&menu, sel!(paste:)).expect("貼り付けの項目が無い");
    assert_eq!(paste.keyEquivalent().to_string(), "v");
    assert!(paste.keyEquivalentModifierMask().contains(NSEventModifierFlags::Command));
    let close = find(&menu, sel!(closeWorkspaces:)).expect("閉じる項目が無い");
    assert_eq!(close.keyEquivalent().to_string(), "w");
    assert!(is(close.target(), &target));
    for selector in [sel!(copy:), sel!(cut:), sel!(selectAll:), sel!(undo:)] {
        assert!(find(&menu, selector).is_some());
    }
}

fn removal_alert_defaults_to_cancel(mtm: MainThreadMarker) {
    let target = target();
    let alert = ui::removal_alert("Acme", &target, mtm);

    assert_eq!(alert.messageText().to_string(), "Remove Acme?");
    assert!(alert.informativeText().to_string().contains("Keychain"));
    let buttons = alert.buttons();
    let titles: Vec<String> = buttons.iter().map(|button| button.title().to_string()).collect();
    assert_eq!(titles, ["Cancel", "Remove"]);
    // 既定のボタン（Return）は「キャンセル」。「削除」は破壊的な操作として出し、Return では押されない
    assert_eq!(buttons.objectAtIndex(0).keyEquivalent().to_string(), "\r");
    assert_eq!(buttons.objectAtIndex(1).keyEquivalent().to_string(), "");
    assert!(buttons.objectAtIndex(1).hasDestructiveAction());
    let escape = alert
        .accessoryView()
        .and_then(|view| view.downcast::<NSButton>().ok())
        .expect("Esc のボタンが無い");
    assert_escape(&escape, &target);
}

/// シートの実際のボタンから、押したときの応答を読み直して、消すのは「削除」のボタンだけで、閉じるときの応答は「キャンセル」と同じであることを確かめる
fn only_the_remove_button_removes(mtm: MainThreadMarker) {
    let target = target();
    let alert = ui::removal_alert("Acme", &target, mtm);
    let buttons = alert.buttons();

    for (index, button) in buttons.iter().enumerate() {
        // NSAlert は、i 番目に足したボタンの応答を NSAlertFirstButtonReturn + i にする
        let response = NSAlertFirstButtonReturn + index as isize;
        let title = button.title().to_string();
        assert_eq!(ui::removal_confirmed(response), title == "Remove", "{title}");
    }
    let cancel = (ui::REMOVAL_CANCEL - NSAlertFirstButtonReturn) as usize;
    assert_eq!(buttons.objectAtIndex(cancel).title().to_string(), "Cancel");
    assert!(!ui::removal_confirmed(ui::REMOVAL_CANCEL));
}

fn assert_escape(button: &NSButton, target: &AnyObject) {
    assert_eq!(button.keyEquivalent().to_string(), "\u{1b}");
    assert!(button.isTransparent());
    assert_eq!(button.action(), Some(sel!(closeWorkspaces:)));
    assert!(is(button.target(), target));
}

struct Window {
    content: Retained<NSView>,
    height: f64,
}

fn window(
    workspaces: &[(&str, &str)],
    status: Status,
    adding: bool,
    field: &objc2_app_kit::NSSecureTextField,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Window {
    let workspaces: Vec<ShownWorkspace> = workspaces
        .iter()
        .map(|(name, shown)| ShownWorkspace {
            name: (*name).into(),
            shown: (*shown).into(),
        })
        .collect();
    let (content, height) = ui::workspace_content(&workspaces, &status, adding, field, target, mtm);
    Window { content, height }
}

impl Window {
    fn views(&self) -> Vec<Retained<NSView>> {
        self.content.subviews().iter().collect()
    }

    fn buttons(&self) -> Vec<Retained<NSButton>> {
        self.views()
            .into_iter()
            .filter_map(|view| view.downcast::<NSButton>().ok())
            .collect()
    }

    fn button(&self, title: &str) -> Retained<NSButton> {
        self.buttons()
            .into_iter()
            .find(|button| button.title().to_string() == title)
            .unwrap_or_else(|| panic!("「{title}」のボタンが無い"))
    }

    fn texts(&self) -> Vec<Retained<NSTextField>> {
        self.views()
            .into_iter()
            .filter_map(|view| view.downcast::<NSTextField>().ok())
            .collect()
    }

    fn text(&self, value: &str) -> Retained<NSTextField> {
        self.texts()
            .into_iter()
            .find(|text| text.stringValue().to_string() == value)
            .unwrap_or_else(|| panic!("「{value}」の文が無い"))
    }

    /// どの部品も、左右の余白を残してウインドウの幅に収まり、高さの中にある
    fn assert_fits(&self) {
        assert_eq!(self.content.frame().size.height, self.height);
        for view in self.views() {
            let frame = view.frame();
            if frame.size.width == 0.0 {
                // Esc の見えないボタン
                continue;
            }
            assert!(frame.origin.x >= ui::MARGIN, "{frame:?}");
            assert!(frame.origin.x + frame.size.width <= ui::WINDOW_WIDTH - ui::MARGIN, "{frame:?}");
            assert!(frame.origin.y >= 0.0 && frame.origin.y + frame.size.height <= self.height, "{frame:?}");
        }
    }
}

fn window_without_workspaces(mtm: MainThreadMarker) {
    let target = target();
    let field = ui::token_field(mtm);
    let window = window(&[], Status::None, false, &field, &target, mtm);

    window.text("No workspaces");
    assert!(window.buttons().iter().all(|button| button.title().to_string() != "Remove"));
    window.assert_fits();
    assert_eq!(field.placeholderString().unwrap().to_string(), "xoxp-…");
}

fn window_lists_workspaces_with_remove(mtm: MainThreadMarker) {
    let target = target();
    let field = ui::token_field(mtm);
    let window = window(
        &[("acme", "Acme Inc."), ("home", "home")],
        Status::None,
        false,
        &field,
        &target,
        mtm,
    );

    window.text("Acme Inc.");
    window.text("home");
    let removes: Vec<String> = window
        .buttons()
        .into_iter()
        .filter(|button| button.title().to_string() == "Remove")
        .map(|button| {
            assert_eq!(button.action(), Some(sel!(removeWorkspace:)));
            button.identifier().unwrap().to_string()
        })
        .collect();
    assert_eq!(removes, ["acme", "home"]);
    window.assert_fits();
}

fn window_locks_input_while_adding(mtm: MainThreadMarker) {
    let target = target();
    let field = ui::token_field(mtm);

    let adding = window(&[], Status::Info("Checking…".into()), true, &field, &target, mtm);
    assert!(!field.isEnabled());
    assert!(!adding.button("Add").isEnabled());

    let idle = window(&[], Status::None, false, &field, &target, mtm);
    assert!(field.isEnabled());
    let add = idle.button("Add");
    assert!(add.isEnabled());
    assert_eq!(add.keyEquivalent().to_string(), "\r");
    assert_eq!(add.action(), Some(sel!(addWorkspace:)));
}

fn window_shows_errors_in_red(mtm: MainThreadMarker) {
    let target = target();
    let field = ui::token_field(mtm);
    let red = NSColor::systemRedColor();

    let error = window(&[], Status::Error("auth.test failed: invalid_auth".into()), false, &field, &target, mtm);
    let text = error.text("auth.test failed: invalid_auth");
    assert!(text.textColor().is_some_and(|color| color.isEqual(Some(&red))));

    let info = window(&[], Status::Info("Added Acme".into()), false, &field, &target, mtm);
    let text = info.text("Added Acme");
    assert!(!text.textColor().is_some_and(|color| color.isEqual(Some(&red))));

    // 長いエラーでも折り返して収まる
    let long = window(&[], Status::Error("x ".repeat(200)), false, &field, &target, mtm);
    long.assert_fits();
}

fn window_has_the_steps_and_escape(mtm: MainThreadMarker) {
    let target = target();
    let field = ui::token_field(mtm);
    let window = window(&[], Status::None, false, &field, &target, mtm);

    window.text("1. Create a Slack App for each workspace");
    window.text("2. Click Install to Workspace and copy the token");
    window.text("3. Paste the copied token and add it");
    let create = window.button("Create Slack App…");
    assert_eq!(create.action(), Some(sel!(createSlackApp:)));
    let escape = window
        .buttons()
        .into_iter()
        .find(|button| button.keyEquivalent().to_string() == "\u{1b}")
        .expect("Esc のボタンが無い");
    assert_escape(&escape, &target);
}

fn window_title_and_close_button(mtm: MainThreadMarker) {
    let window = ui::workspace_window(300.0, mtm);

    assert_eq!(window.title().to_string(), "Workspaces");
    assert!(window.styleMask().contains(NSWindowStyleMask::Closable));
    assert!(!window.isReleasedWhenClosed());
}
