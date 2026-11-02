//! メニューやウインドウの部品を作る。常駐の状態や HOME の下のファイルは読まず、入力を引数で受けて部品を返す。
//! app.rs が状態を集めてここを呼ぶ。tests/appkit.rs は、ここで作った部品を直に確かめる
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{sel, MainThreadOnly, Message};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle, NSModalResponse, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSColor,
    NSControlStateValueOn, NSMenu, NSMenuItem, NSSecureTextField, NSTextField,
    NSUserInterfaceItemIdentification, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use crate::l10n;
use crate::login::LoginState;
use crate::menu::{Entry, FetchButton};

/// ワークスペースのウインドウの下に出す文。失敗は赤で出す
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    None,
    Info(String),
    Error(String),
}

/// メニューの 1 項目。ボタンの押し先は target、「アップデートを確認…」だけは updater（Sparkle）
pub fn menu_item(
    entry: &Entry,
    target: &AnyObject,
    updater: Option<&AnyObject>,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
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
            let item = new_item(&l10n::text(l10n::NONE), None, mtm);
            item.setIndentationLevel(1);
            item.setEnabled(false);
            item
        }
        Entry::DismissAll => {
            let item = new_item(
                &l10n::text(l10n::MARK_ALL_AS_READ),
                Some(sel!(dismissAll:)),
                mtm,
            );
            unsafe { item.setTarget(Some(target)) };
            item
        }
        Entry::Fetch(button) => {
            let title = match button {
                FetchButton::Ready => l10n::text(l10n::FETCH_NOW),
                FetchButton::Cooldown(seconds) => {
                    l10n::format(l10n::FETCH_NOW_IN, &[&seconds.to_string()])
                }
                FetchButton::Waiting => l10n::text(l10n::WAITING_TO_FETCH),
                FetchButton::Fetching => l10n::text(l10n::FETCHING),
            };
            let item = new_item(&title, Some(sel!(fetchNow:)), mtm);
            item.setEnabled(*button == FetchButton::Ready);
            unsafe { item.setTarget(Some(target)) };
            item
        }
        Entry::Workspaces => {
            let item = new_item(
                &l10n::text(l10n::WORKSPACES),
                Some(sel!(openWorkspaces:)),
                mtm,
            );
            unsafe { item.setTarget(Some(target)) };
            item
        }
        Entry::LaunchAtLogin(state) => {
            let title = match state {
                LoginState::NeedsApproval => l10n::text(l10n::OPEN_AT_LOGIN_NEEDS_APPROVAL),
                LoginState::On | LoginState::Off => l10n::text(l10n::OPEN_AT_LOGIN),
            };
            let item = new_item(&title, Some(sel!(toggleLogin:)), mtm);
            if *state == LoginState::On {
                item.setState(NSControlStateValueOn);
            }
            unsafe { item.setTarget(Some(target)) };
            item
        }
        Entry::CheckForUpdates => {
            // SPUStandardUpdaterController の checkForUpdates: を直接呼ばせる
            let item = new_item(
                &l10n::text(l10n::CHECK_FOR_UPDATES),
                Some(sel!(checkForUpdates:)),
                mtm,
            );
            if let Some(updater) = updater {
                unsafe { item.setTarget(Some(updater)) };
            }
            item
        }
        Entry::Quit => {
            let item = new_item(&l10n::text(l10n::QUIT), Some(sel!(quit:)), mtm);
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

/// アプリのメインメニュー。Dock に出ないアプリではメインメニューは画面に出ないが、
/// これがあると入力欄で ⌘V（貼り付け）などのキー操作が効く
pub fn main_menu(target: &AnyObject, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let main = NSMenu::new(mtm);
    // ⌘W で、ワークスペースのウインドウ（確認のシートが出ていればシート）を閉じる。
    // ⌘ の付かない Esc はメインメニューでは処理されないので、ウインドウとシートに置いたボタンで受ける（escape_button）
    let window = NSMenu::new(mtm);
    let close = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Close"),
            Some(sel!(closeWorkspaces:)),
            ns_string!("w"),
        )
    };
    unsafe { close.setTarget(Some(target)) };
    window.addItem(&close);
    let window_item = NSMenuItem::new(mtm);
    window_item.setSubmenu(Some(&window));
    main.addItem(&window_item);
    let edit = NSMenu::new(mtm);
    for (title, action, key) in [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        edit.addItem(&item);
    }
    let edit_item = NSMenuItem::new(mtm);
    edit_item.setSubmenu(Some(&edit));
    main.addItem(&edit_item);
    main
}

/// 確認のシートを ⌘W や Esc で閉じるときの応答（「キャンセル」のボタンと同じ）
pub const REMOVAL_CANCEL: NSModalResponse = NSAlertFirstButtonReturn;

/// 確認のシートの応答が「削除」のボタンのものか。removal_alert のボタンの並び（キャンセル、削除）と対にする
pub fn removal_confirmed(response: NSModalResponse) -> bool {
    response == NSAlertSecondButtonReturn
}

/// 「削除」を押したときに出す確認のシート。最初のボタンが「キャンセル」、2 番目が「削除」。
/// 消すのは取り返しがつかないので、既定のボタン（Return）は「キャンセル」にする。Esc は escape_button で受ける
pub fn removal_alert(shown: &str, target: &AnyObject, mtm: MainThreadMarker) -> Retained<NSAlert> {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str(&l10n::format(
        l10n::REMOVE_CONFIRM,
        &[shown],
    )));
    alert.setInformativeText(&NSString::from_str(&l10n::text(
        l10n::REMOVE_CONFIRM_DETAIL,
    )));
    let cancel = alert.addButtonWithTitle(&NSString::from_str(&l10n::text(l10n::CANCEL)));
    cancel.setKeyEquivalent(ns_string!("\r"));
    let remove = alert.addButtonWithTitle(&NSString::from_str(&l10n::text(l10n::REMOVE)));
    remove.setKeyEquivalent(ns_string!(""));
    remove.setHasDestructiveAction(true);
    alert.setAccessoryView(Some(&escape_button(target, mtm)));
    alert
}

// ワークスペースのウインドウの幅と余白。部品は座標で置く（Auto Layout の fittingSize では幅が足りず、はみ出したため）
pub const WINDOW_WIDTH: f64 = 480.0;
pub const MARGIN: f64 = 20.0;
pub const GAP: f64 = 10.0;

/// ウインドウに並べるワークスペース。name は設定の名前、shown は出す名前（Slack 上の名前が分かればそれ）
pub struct ShownWorkspace {
    pub name: String,
    pub shown: String,
}

/// ワークスペースのウインドウの中身と、その高さ。field は作り直しても入力が残るよう、呼ぶ側が持っておく
pub fn workspace_content(
    workspaces: &[ShownWorkspace],
    status: &Status,
    adding: bool,
    field: &NSSecureTextField,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, f64) {
    let text_width = WINDOW_WIDTH - MARGIN * 2.0;
    let mut rows = Vec::new();
    if workspaces.is_empty() {
        rows.push(text_row(
            wrapping_label(&l10n::text(l10n::NO_WORKSPACES), mtm),
            text_width,
        ));
    }
    for workspace in workspaces {
        let remove = button(
            &l10n::text(l10n::REMOVE),
            sel!(removeWorkspace:),
            target,
            mtm,
        );
        remove.setIdentifier(Some(&NSString::from_str(&workspace.name)));
        rows.push(button_row(&label(&workspace.shown, mtm), remove));
    }
    // 一覧と、足す手順のあいだの区切り
    rows.push(separator_row(mtm));
    // 先に Slack App を作り、そのあとで出てくる token を貼る、という順番が分かるよう、手順に番号を振って上から並べる
    let create = button(
        &l10n::text(l10n::CREATE_SLACK_APP),
        sel!(createSlackApp:),
        target,
        mtm,
    );
    rows.push(button_row(
        &wrapping_label(&l10n::text(l10n::STEP_CREATE), mtm),
        create,
    ));
    rows.push(text_row(
        wrapping_label(&l10n::text(l10n::STEP_INSTALL), mtm),
        text_width,
    ));
    rows.push(text_row(
        wrapping_label(&l10n::text(l10n::STEP_PASTE), mtm),
        text_width,
    ));
    let add = button(&l10n::text(l10n::ADD), sel!(addWorkspace:), target, mtm);
    // 確かめているあいだは、入力欄も「追加」も触れないようにする
    field.setEnabled(!adding);
    add.setEnabled(!adding);
    add.setKeyEquivalent(ns_string!("\r"));
    rows.push(button_row(field, add));
    let status = match status {
        Status::None => wrapping_label("", mtm),
        Status::Info(text) => wrapping_label(text, mtm),
        Status::Error(text) => {
            let error = wrapping_label(text, mtm);
            error.setTextColor(Some(&NSColor::systemRedColor()));
            error
        }
    };
    rows.push(text_row(status, text_width));

    let sizes: Vec<(f64, Option<NSSize>)> = rows
        .iter()
        .map(|row| (row.left_height, row.right.as_ref().map(|(_, size)| *size)))
        .collect();
    let (frames, height) = layout(&sizes, WINDOW_WIDTH);
    let content = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WINDOW_WIDTH, height)),
    );
    content.addSubview(&escape_button(target, mtm));
    for (row, (left, right)) in rows.iter().zip(frames) {
        row.left.setFrame(left);
        content.addSubview(&row.left);
        if let (Some((view, _)), Some(frame)) = (&row.right, right) {
            view.setFrame(frame);
            content.addSubview(view);
        }
    }
    (content, height)
}

/// token の入力欄。xoxp-… を薄い字で出す
pub fn token_field(mtm: MainThreadMarker) -> Retained<NSSecureTextField> {
    let field = NSSecureTextField::new(mtm);
    field.setPlaceholderString(Some(ns_string!("xoxp-…")));
    field
}

/// ワークスペースのウインドウ（中身は workspace_content で入れる）
pub fn workspace_window(height: f64, mtm: MainThreadMarker) -> Retained<NSWindow> {
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WINDOW_WIDTH, height)),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(&l10n::text(l10n::WORKSPACES_TITLE)));
    window
}

/// ウインドウの 1 行。左の部品は残りの幅いっぱい、右の部品（ボタン）は右端に置く
struct Row {
    left: Retained<NSView>,
    left_height: f64,
    right: Option<(Retained<NSView>, NSSize)>,
}

/// 行を上から順に置いたときの、各行の左と右の部品の位置と、全体の高さ。座標は AppKit と同じく左下が原点
pub fn layout(rows: &[(f64, Option<NSSize>)], width: f64) -> (Vec<(NSRect, Option<NSRect>)>, f64) {
    let heights: Vec<f64> = rows
        .iter()
        .map(|(left, right)| right.map_or(*left, |size| left.max(size.height)))
        .collect();
    let height =
        MARGIN * 2.0 + heights.iter().sum::<f64>() + GAP * heights.len().saturating_sub(1) as f64;
    let mut top = MARGIN;
    let frames = rows
        .iter()
        .zip(&heights)
        .map(|((left, right), row)| {
            // 行の高さの中で、上下の真ん中にそろえる
            let bottom = height - top - row;
            let right_width = right.map_or(0.0, |size| size.width + GAP);
            let left_frame = NSRect::new(
                NSPoint::new(MARGIN, bottom + (row - left) / 2.0),
                NSSize::new(width - MARGIN * 2.0 - right_width, *left),
            );
            let right_frame = right.map(|size| {
                NSRect::new(
                    NSPoint::new(
                        width - MARGIN - size.width,
                        bottom + (row - size.height) / 2.0,
                    ),
                    size,
                )
            });
            top += row + GAP;
            (left_frame, right_frame)
        })
        .collect();
    (frames, height)
}

/// 幅いっぱいの区切りの線の行
fn separator_row(mtm: MainThreadMarker) -> Row {
    let line = NSBox::new(mtm);
    line.setBoxType(NSBoxType::Separator);
    Row {
        left: as_view(&line),
        left_height: 1.0,
        right: None,
    }
}

/// 幅いっぱいの文の行。長い文は折り返し、その高さにする
fn text_row(text: Retained<NSTextField>, width: f64) -> Row {
    text.setPreferredMaxLayoutWidth(width);
    let height = text.fittingSize().height.max(17.0);
    Row {
        left: as_view(&text),
        left_height: height,
        right: None,
    }
}

/// 左に部品、右端にボタンを置く行。左の高さは部品に合わせる
fn button_row(left: &NSView, right: Retained<NSButton>) -> Row {
    right.sizeToFit();
    let right_size = right.frame().size;
    let left_height = if left.downcast_ref::<NSSecureTextField>().is_some() {
        // 入力欄は文字が無いと高さが決まらないので、標準の高さにする
        22.0
    } else {
        // 文はボタンの左の幅で折り返し、その高さにする
        if let Some(text) = left.downcast_ref::<NSTextField>() {
            text.setPreferredMaxLayoutWidth(WINDOW_WIDTH - MARGIN * 2.0 - right_size.width - GAP);
        }
        left.fittingSize().height
    };
    Row {
        left: left.retain(),
        left_height,
        right: Some((as_view(&right), right_size)),
    }
}

fn as_view(view: &NSView) -> Retained<NSView> {
    view.retain()
}

fn button(
    title: &str,
    action: Sel,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    }
}

/// Esc で closeWorkspaces: を呼ぶ、見えないボタン（大きさ 0・透明）。
/// ⌘ の付かないキーはメニューではなくウインドウの中のボタンが受けるので、ウインドウとシートに 1 つずつ置く
pub fn escape_button(target: &AnyObject, mtm: MainThreadMarker) -> Retained<NSButton> {
    let escape = button("", sel!(closeWorkspaces:), target, mtm);
    escape.setKeyEquivalent(ns_string!("\u{1b}"));
    escape.setTransparent(true);
    escape.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)));
    escape
}

fn label(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setSelectable(true);
    label
}

/// 幅に収まらないときに折り返す文。labelWithString の部品は 1 行用で、折り返し幅を決めても折り返さない
fn wrapping_label(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setSelectable(true);
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    fn max_x(rect: NSRect) -> f64 {
        rect.origin.x + rect.size.width
    }

    #[test]
    fn layout_keeps_every_part_inside_the_window() {
        let button = NSSize::new(80.0, 32.0);
        let (frames, height) = layout(
            &[
                (17.0, None),
                (17.0, Some(button)),
                (22.0, Some(button)),
                (40.0, None),
            ],
            WINDOW_WIDTH,
        );

        for (left, right) in &frames {
            assert!(left.origin.x >= MARGIN && left.origin.y >= MARGIN);
            assert!(max_x(*left) <= WINDOW_WIDTH - MARGIN);
            if let Some(right) = right {
                assert_eq!(max_x(*right), WINDOW_WIDTH - MARGIN);
                assert_eq!(max_x(*left) + GAP, right.origin.x);
            }
        }
        // 32 + 32 は行の中でいちばん高い部品（ボタン）の高さ
        assert_eq!(height, MARGIN * 2.0 + 17.0 + 32.0 + 32.0 + 40.0 + GAP * 3.0);
    }

    #[test]
    fn layout_puts_rows_from_the_top() {
        let (frames, height) = layout(&[(17.0, None), (20.0, None)], WINDOW_WIDTH);

        assert_eq!(frames[0].0.origin.y, height - MARGIN - 17.0);
        assert_eq!(frames[1].0.origin.y, height - MARGIN - 17.0 - GAP - 20.0);
        assert_eq!(frames[1].0.origin.y, MARGIN);
    }
}
