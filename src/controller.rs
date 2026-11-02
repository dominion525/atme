//! 常駐の状態と、出来事を受けたときの判断。AppKit を使わない。
//! app.rs は出来事をここに渡し、返ってきた「するべきこと」（Effect）を AppKit で実行するだけにする
use std::path::Path;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::dismiss;
use crate::l10n;
use crate::menu::{self, FetchButton};
use crate::scheduler::Phase;
use crate::state::{Cache, Dismissed, SeenKey, Store, Users};
use crate::ui::{ShownWorkspace, Status};

/// search.messages の呼び出し回数の上限に届かないよう、続けて取得するときはこれだけ空ける
pub const FETCH_MIN_GAP: Duration = Duration::from_secs(30);

/// 出来事を受けたあとに、app.rs がするべきこと（返した順に行う）
#[derive(Debug, PartialEq)]
pub enum Effect {
    /// メニューとメニューバーの件数を作り直す
    RefreshMenu,
    /// メニューバーの絵を、今の絵の番号で描き直す
    RedrawIcon,
    /// 取得中の絵を回し始める
    StartAnimation,
    /// 取得中の絵を止める
    StopAnimation,
    /// ワークスペースのウインドウを作り直す
    ShowWorkspaces,
    /// token の入力欄を空にする
    ClearTokenField,
    /// token を別のスレッドで確かめて足す（終わったら token_checked を呼ぶ）
    CheckToken(String),
    /// 取得を頼む
    Fetch,
}

/// 常駐のあいだ持っておく状態
#[derive(Debug)]
pub struct State {
    /// メニューに出す失敗の行
    pub failures: Vec<String>,
    pub phase: Phase,
    /// 最後に取得が終わった時刻。「今すぐ取得（あと N 秒）」に使う
    pub last_fetched: Option<Instant>,
    /// 取得中の絵の番号（0 が縦の絵）
    pub frame: usize,
    /// 取得中の絵を回しているか
    pub animating: bool,
    /// 取得が終わり、縦に戻る絵まで回してから止めるところ
    pub stopping: bool,
    /// ワークスペースのウインドウの下に出す文
    pub setup_status: Status,
    /// token を確かめているあいだは、入力欄と「追加」を押せなくする
    pub adding: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            failures: Vec::new(),
            phase: Phase::Idle,
            last_fetched: None,
            frame: 0,
            animating: false,
            stopping: false,
            setup_status: Status::None,
            adding: false,
        }
    }
}

impl State {
    /// 取得が終わった。失敗の行を今回の分に置き換える
    pub fn fetched(&mut self, failures: Vec<String>) -> Vec<Effect> {
        self.failures = failures;
        vec![Effect::RefreshMenu]
    }

    /// 取得の段階が変わった
    pub fn phase_changed(&mut self, phase: Phase, now: Instant) -> Vec<Effect> {
        if self.phase == Phase::Fetching && phase != Phase::Fetching {
            self.last_fetched = Some(now);
        }
        self.phase = phase;
        let mut effects = Vec::new();
        if phase == Phase::Fetching {
            self.stopping = false;
            if !self.animating {
                self.animating = true;
                effects.push(Effect::StartAnimation);
            }
        } else if self.animating {
            self.stopping = true;
        }
        effects.push(Effect::RefreshMenu);
        effects
    }

    /// 取得中の絵の時計が進んだ。frames は絵の枚数
    pub fn tick(&mut self, frames: usize) -> Vec<Effect> {
        match advance(self.frame, frames, self.stopping) {
            Some(next) => {
                self.frame = next;
                vec![Effect::RedrawIcon]
            }
            None => {
                self.frame = 0;
                self.stopping = false;
                self.animating = false;
                vec![Effect::StopAnimation, Effect::RedrawIcon]
            }
        }
    }

    /// 今の「今すぐ取得」の状態
    pub fn fetch_button(&self, now: Instant) -> FetchButton {
        menu::fetch_button(
            self.phase,
            self.last_fetched
                .map(|finished| now.duration_since(finished)),
            FETCH_MIN_GAP,
        )
    }

    /// ログイン時の起動を切り替えられなかった。message は macOS のエラーの説明
    pub fn login_toggle_failed(&mut self, message: &str) -> Vec<Effect> {
        self.failures
            .push(l10n::format(l10n::LOGIN_TOGGLE_FAILED, &[message]));
        vec![Effect::RefreshMenu]
    }

    /// 「追加」が押された。確かめているあいだと、空の入力は受け付けない
    pub fn add_pressed(&mut self, token: &str) -> Vec<Effect> {
        if self.adding || token.trim().is_empty() {
            return Vec::new();
        }
        self.adding = true;
        self.setup_status = Status::Info(l10n::text(l10n::CHECKING));
        vec![Effect::ShowWorkspaces, Effect::CheckToken(token.to_owned())]
    }

    /// token を確かめ終わった。足せたら Slack 上の名前、足せなければエラーの文
    pub fn token_checked(&mut self, result: Result<String, String>) -> Vec<Effect> {
        self.adding = false;
        match result {
            Ok(team) => {
                self.setup_status = Status::Info(l10n::format(l10n::ADDED, &[&team]));
                vec![
                    Effect::ClearTokenField,
                    Effect::ShowWorkspaces,
                    Effect::Fetch,
                ]
            }
            Err(error) => {
                self.setup_status = Status::Error(error);
                vec![Effect::ShowWorkspaces]
            }
        }
    }

    /// ワークスペースを外し終わった（または外せなかった）
    pub fn removed(&mut self, result: Result<(), String>) -> Vec<Effect> {
        self.setup_status = match result {
            Ok(()) => Status::None,
            Err(error) => Status::Error(error),
        };
        vec![Effect::ShowWorkspaces, Effect::Fetch]
    }
}

/// メニューバーに出す絵
#[derive(Debug, PartialEq)]
pub enum Icon {
    /// 取得中の回る絵の、何枚目か
    Turning(usize),
    /// いつもの絵
    Idle,
    /// 絵が読めなかったときの、文字だけの表示
    Text,
}

/// メニューバーに出す絵と文。turning は回る絵の枚数、has_idle はいつもの絵を読めたか、badge は未読の件数（無ければ空）。
/// 絵が読めなかったときは、文字の「@」に件数を続けて、atme だと分かるようにする
pub fn icon(state: &State, turning: usize, has_idle: bool, badge: &str) -> (Icon, String) {
    let icon = if state.animating && turning > 0 {
        Icon::Turning(state.frame % turning)
    } else if has_idle {
        Icon::Idle
    } else {
        Icon::Text
    };
    let title = match icon {
        Icon::Text => format!("@ {badge}").trim_end().to_owned(),
        _ => badge.to_owned(),
    };
    (icon, title)
}

/// 次に出す絵の番号。止める途中なら、縦に戻る絵（0 番）まで回ったところで None を返して止める
fn advance(frame: usize, frames: usize, stopping: bool) -> Option<usize> {
    if frames == 0 {
        return None;
    }
    let next = (frame + 1) % frames;
    (!(stopping && next == 0)).then_some(next)
}

/// メニューを作るのに読むもの
#[derive(Default)]
pub struct MenuData {
    pub cache: Option<Cache>,
    pub users: Users,
    pub dismissed: Dismissed,
    /// 設定ファイルに書かれた順のワークスペース（設定の name）
    pub workspaces: Vec<String>,
}

/// メニューの材料を読む。キャッシュから消えた既読の記録は、ここで捨てて保存する
pub fn menu_data(store: &Store, config: &Path) -> MenuData {
    let cache = store.load_cache().ok().flatten();
    let users = store.load_users().ok().flatten().unwrap_or_default();
    let mut dismissed = store.load_dismissed().ok().flatten().unwrap_or_default();
    if let Some(cache) = &cache {
        let pruned = dismiss::prune(&dismissed, cache);
        if pruned != dismissed {
            if let Err(error) = store.save_dismissed(&pruned) {
                eprintln!("{error:#}");
            }
            dismissed = pruned;
        }
    }
    let workspaces = Config::load_from(config)
        .map(|config| {
            config
                .workspace
                .into_iter()
                .map(|workspace| workspace.name)
                .collect()
        })
        .unwrap_or_default();
    MenuData {
        cache,
        users,
        dismissed,
        workspaces,
    }
}

/// permalink の項目を、既読にした記録に足す（メニューの項目や通知から開いたとき）
pub fn dismiss_permalink(store: &Store, permalink: &str) {
    dismiss_with(store, |cache| {
        dismiss::key_for_permalink(cache, permalink)
            .into_iter()
            .collect()
    });
}

/// 今の一覧を全部、既読にした記録に足す（「すべて既読にする」）
pub fn dismiss_all(store: &Store) {
    dismiss_with(store, dismiss::all_keys);
}

fn dismiss_with(store: &Store, select: impl FnOnce(&Cache) -> Vec<SeenKey>) {
    let Some(cache) = store.load_cache().ok().flatten() else {
        return;
    };
    let mut dismissed = store.load_dismissed().ok().flatten().unwrap_or_default();
    dismissed.extend(select(&cache));
    if let Err(error) = store.save_dismissed(&dismiss::prune(&dismissed, &cache)) {
        eprintln!("{error:#}");
    }
}

/// ウインドウに並べるワークスペース。設定の name は内部の名前なので、Slack 上の名前が分かればそちらを出す
pub fn shown_workspaces(store: &Store, config: &Path) -> Vec<ShownWorkspace> {
    let names = Config::load_from(config)
        .map(|config| config.workspace)
        .unwrap_or_default();
    names
        .into_iter()
        .map(|workspace| ShownWorkspace {
            shown: shown_name(store, &workspace.name),
            name: workspace.name,
        })
        .collect()
}

/// 設定の name のワークスペースの、出す名前（Slack 上の名前が分からなければ name）
pub fn shown_name(store: &Store, name: &str) -> String {
    store
        .load_cache()
        .ok()
        .flatten()
        .and_then(|cache| cache.teams.get(name).cloned())
        .unwrap_or_else(|| name.to_owned())
}

/// 初めて起動したとき（ワークスペースが 1 つも無いとき）は、追加のウインドウを開く。壊れた設定も、開いて直してもらう
pub fn needs_first_workspace(config: &Path) -> bool {
    Config::load_from(config).map_or(true, |config| config.workspace.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::display::tests::cache;

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn icon_turns_while_fetching_and_shows_the_count() {
        let mut state = State::default();
        assert_eq!(icon(&state, 12, true, "3"), (Icon::Idle, "3".into()));
        assert_eq!(icon(&state, 12, true, ""), (Icon::Idle, String::new()));

        state.phase_changed(Phase::Fetching, now());
        state.tick(12);
        state.tick(12);
        assert_eq!(icon(&state, 12, true, "3"), (Icon::Turning(2), "3".into()));
    }

    #[test]
    fn icon_falls_back_to_text_without_images() {
        // .app の資源が欠けて絵を読めないときも、atme だと分かるように「@」を出す
        let mut state = State::default();
        assert_eq!(icon(&state, 0, false, "3"), (Icon::Text, "@ 3".into()));
        assert_eq!(icon(&state, 0, false, ""), (Icon::Text, "@".into()));

        // 回る絵が無ければ、取得中でもいつもの絵のまま
        state.phase_changed(Phase::Fetching, now());
        assert_eq!(icon(&state, 0, true, "3"), (Icon::Idle, "3".into()));
    }

    #[test]
    fn fetched_replaces_the_failure_lines() {
        let mut state = State {
            failures: vec!["old".into()],
            ..State::default()
        };

        let effects = state.fetched(vec!["new".into()]);

        assert_eq!(state.failures, ["new"]);
        assert_eq!(effects, [Effect::RefreshMenu]);
    }

    #[test]
    fn fetching_starts_the_animation_once() {
        let mut state = State::default();

        let first = state.phase_changed(Phase::Fetching, now());
        let again = state.phase_changed(Phase::Fetching, now());

        assert_eq!(first, [Effect::StartAnimation, Effect::RefreshMenu]);
        assert_eq!(again, [Effect::RefreshMenu]);
        assert!(state.animating);
    }

    #[test]
    fn finishing_a_fetch_remembers_when_and_lets_the_icon_finish_its_turn() {
        let mut state = State::default();
        let started = now();
        state.phase_changed(Phase::Fetching, started);
        let finished = started + Duration::from_secs(3);

        let effects = state.phase_changed(Phase::Idle, finished);

        assert_eq!(effects, [Effect::RefreshMenu]);
        assert_eq!(state.last_fetched, Some(finished));
        // すぐには止めず、縦の絵に戻るまで回す
        assert!(state.animating && state.stopping);
    }

    #[test]
    fn waiting_does_not_count_as_finishing() {
        let mut state = State::default();

        state.phase_changed(Phase::Waiting, now());

        assert_eq!(state.last_fetched, None);
    }

    #[test]
    fn fetching_again_while_stopping_keeps_turning() {
        let mut state = State::default();
        state.phase_changed(Phase::Fetching, now());
        state.phase_changed(Phase::Idle, now());

        let effects = state.phase_changed(Phase::Fetching, now());

        assert_eq!(effects, [Effect::RefreshMenu]);
        assert!(!state.stopping);
    }

    #[test]
    fn ticks_turn_the_icon_and_stop_upright() {
        let mut state = State::default();
        state.phase_changed(Phase::Fetching, now());
        for _ in 0..10 {
            assert_eq!(state.tick(12), [Effect::RedrawIcon]);
        }
        state.phase_changed(Phase::Idle, now());

        assert_eq!(state.tick(12), [Effect::RedrawIcon]);
        assert_eq!(state.frame, 11);
        assert_eq!(state.tick(12), [Effect::StopAnimation, Effect::RedrawIcon]);
        assert_eq!(
            (state.frame, state.animating, state.stopping),
            (0, false, false)
        );
    }

    #[test]
    fn without_frames_the_animation_stops_at_once() {
        let mut state = State::default();
        state.phase_changed(Phase::Fetching, now());

        assert_eq!(state.tick(0), [Effect::StopAnimation, Effect::RedrawIcon]);
    }

    #[test]
    fn fetch_button_waits_after_a_fetch() {
        let mut state = State::default();
        let finished = now();
        state.phase_changed(Phase::Fetching, finished);
        state.phase_changed(Phase::Idle, finished);

        assert_eq!(
            state.fetch_button(finished + Duration::from_secs(10)),
            FetchButton::Cooldown(20)
        );
        assert_eq!(
            state.fetch_button(finished + FETCH_MIN_GAP),
            FetchButton::Ready
        );
    }

    #[test]
    fn login_toggle_failure_is_shown_in_the_menu() {
        let mut state = State::default();

        let effects = state.login_toggle_failed("Operation not permitted");

        assert_eq!(
            state.failures,
            ["Couldn't change Open at Login (Operation not permitted)"]
        );
        assert_eq!(effects, [Effect::RefreshMenu]);
    }

    #[test]
    fn adding_checks_the_token_and_locks_the_input() {
        let mut state = State::default();

        let effects = state.add_pressed("xoxp-home");

        assert_eq!(
            effects,
            [
                Effect::ShowWorkspaces,
                Effect::CheckToken("xoxp-home".into())
            ]
        );
        assert!(state.adding);
        assert_eq!(state.setup_status, Status::Info("Checking…".into()));
    }

    #[test]
    fn adding_ignores_an_empty_input_and_a_second_press() {
        let mut state = State::default();
        assert!(state.add_pressed("  \n").is_empty());
        assert!(!state.adding);

        state.add_pressed("xoxp-home");
        assert!(state.add_pressed("xoxp-home").is_empty());
    }

    #[test]
    fn added_workspace_clears_the_input_and_fetches() {
        let mut state = State::default();
        state.add_pressed("xoxp-home");

        let effects = state.token_checked(Ok("Acme".into()));

        assert_eq!(
            effects,
            [
                Effect::ClearTokenField,
                Effect::ShowWorkspaces,
                Effect::Fetch
            ]
        );
        assert!(!state.adding);
        assert_eq!(state.setup_status, Status::Info("Added Acme".into()));
    }

    #[test]
    fn failed_check_keeps_the_input_and_shows_the_error() {
        let mut state = State::default();
        state.add_pressed("xoxp-home");

        let effects = state.token_checked(Err("auth.test failed: invalid_auth".into()));

        assert_eq!(effects, [Effect::ShowWorkspaces]);
        assert!(!state.adding);
        assert_eq!(
            state.setup_status,
            Status::Error("auth.test failed: invalid_auth".into())
        );
    }

    #[test]
    fn removing_clears_the_status_and_fetches() {
        let mut state = State {
            setup_status: Status::Info("Added Acme".into()),
            ..State::default()
        };

        assert_eq!(
            state.removed(Ok(())),
            [Effect::ShowWorkspaces, Effect::Fetch]
        );
        assert_eq!(state.setup_status, Status::None);

        state.removed(Err("Couldn't delete".into()));
        assert_eq!(state.setup_status, Status::Error("Couldn't delete".into()));
    }

    /// 一時的なフォルダの Store と、config を書いた設定ファイル
    fn files(config: &str) -> (tempfile::TempDir, Store, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("state"));
        let path = dir.path().join("config.toml");
        std::fs::write(&path, config).unwrap();
        (dir, store, path)
    }

    const HOME: &str = "[[workspace]]\nname = \"home\"\n";

    #[test]
    fn opening_an_item_records_it_as_dismissed() {
        let (_dir, store, _) = files(HOME);
        let cache = cache();
        store.save_cache(&cache).unwrap();

        dismiss_permalink(&store, &cache.items[1].permalink);

        assert_eq!(
            store.load_dismissed().unwrap().unwrap(),
            [cache.items[1].key()].into()
        );
    }

    #[test]
    fn opening_an_unknown_permalink_records_nothing() {
        let (_dir, store, _) = files(HOME);
        store.save_cache(&cache()).unwrap();

        dismiss_permalink(&store, "https://example.slack.com/archives/C9/p1");

        assert_eq!(store.load_dismissed().unwrap().unwrap(), Dismissed::new());
    }

    #[test]
    fn mark_all_as_read_records_every_item() {
        let (_dir, store, _) = files(HOME);
        let cache = cache();
        store.save_cache(&cache).unwrap();

        dismiss_all(&store);

        let all: Dismissed = cache.items.iter().map(|item| item.key()).collect();
        assert_eq!(store.load_dismissed().unwrap().unwrap(), all);
    }

    #[test]
    fn menu_data_forgets_records_of_items_no_longer_cached() {
        let (_dir, store, config) = files(HOME);
        let cache = cache();
        store.save_cache(&cache).unwrap();
        let gone = SeenKey {
            workspace: "home".into(),
            channel: "C9".into(),
            ts: "1.000100".into(),
        };
        let kept = cache.items[0].key();
        store.save_dismissed(&[gone, kept.clone()].into()).unwrap();

        let data = menu_data(&store, &config);

        assert_eq!(data.dismissed, [kept.clone()].into());
        assert_eq!(store.load_dismissed().unwrap().unwrap(), [kept].into());
        assert_eq!(data.workspaces, ["home"]);
        assert_eq!(data.cache.unwrap().items.len(), 2);
    }

    #[test]
    fn menu_data_without_files_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("state"));

        let data = menu_data(&store, &dir.path().join("config.toml"));

        assert!(data.cache.is_none());
        assert!(data.workspaces.is_empty());
    }

    #[test]
    fn workspaces_are_shown_by_their_slack_names() {
        let (_dir, store, config) =
            files("[[workspace]]\nname = \"home\"\n\n[[workspace]]\nname = \"new\"\n");
        store
            .save_cache(&Cache {
                teams: BTreeMap::from([("home".to_owned(), "Home 株式会社".to_owned())]),
                ..Cache::default()
            })
            .unwrap();

        let shown: Vec<(String, String)> = shown_workspaces(&store, &config)
            .into_iter()
            .map(|workspace| (workspace.name, workspace.shown))
            .collect();

        // まだ取得できていない new は、設定の name で出す
        assert_eq!(
            shown,
            [
                ("home".to_owned(), "Home 株式会社".to_owned()),
                ("new".to_owned(), "new".to_owned()),
            ]
        );
    }

    #[test]
    fn first_launch_opens_the_workspace_window() {
        let (_dir, _store, empty) = files("");
        let (_dir2, _store2, home) = files(HOME);
        let (_dir3, _store3, broken) = files("days = ");

        assert!(needs_first_workspace(&empty));
        assert!(!needs_first_workspace(&home));
        assert!(needs_first_workspace(&broken));
    }
}
