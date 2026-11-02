//! 取得やワークスペースの追加・削除が触る外の世界。ふだんは Env::system()、テストでは差し替える
use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Local};

use crate::config::{self, Workspace};
use crate::credentials::Token;
use crate::notify;
use crate::slack;
use crate::state::{Item, Store, Users};

pub struct Env {
    /// 設定ファイルの置き場所
    pub config: PathBuf,
    pub store: Store,
    /// Slack の API の URL の頭
    pub api: String,
    /// ワークスペースの token を読む
    pub token: Box<dyn Fn(&Workspace) -> Result<Token>>,
    /// name のワークスペースの token として保存する
    pub save_token: Box<dyn Fn(&Token, &str) -> Result<()>>,
    /// name のワークスペースの token を消す
    pub delete_token: Box<dyn Fn(&str) -> Result<()>>,
    /// 新着を通知する
    pub notify: Box<dyn Fn(&[Item], &Users, &BTreeMap<String, String>)>,
    /// さかのぼる日付と、通知済みの記録を捨てる境目の基準にする時刻
    pub now: DateTime<Local>,
}

impl Env {
    /// ふだんの外の世界（HOME の下のファイル、Slack、Keychain、macOS の通知、今の時刻）
    pub fn system() -> Result<Self> {
        Ok(Self {
            config: config::path()?,
            store: Store::open()?,
            api: slack::API_URL.to_owned(),
            token: Box::new(Token::load),
            save_token: Box::new(|token, name| token.save(name)),
            delete_token: Box::new(Token::delete),
            notify: Box::new(notify::notify),
            now: Local::now(),
        })
    }
}

/// テスト用の偽の外の世界。一時的なフォルダに設定と状態を置き、token はメモリの表に出し入れし、通知は記録するだけ
#[cfg(test)]
pub mod fake {
    use std::cell::RefCell;
    use std::rc::Rc;

    use chrono::TimeZone;

    use super::*;

    pub struct World {
        pub env: Env,
        /// ワークスペースの name から token への表（Keychain の代わり）
        pub tokens: Rc<RefCell<BTreeMap<String, String>>>,
        /// 通知した項目の ts
        pub notified: Rc<RefCell<Vec<String>>>,
        /// 通知した項目の題（ワークスペース名 / 場所）。通知に渡されたワークスペース名の対応で作る
        pub titles: Rc<RefCell<Vec<String>>>,
        _dir: tempfile::TempDir,
    }

    /// config を書いた設定ファイルと、空の状態のフォルダで、api の Slack に向ける
    pub fn world(config: &str, api: String) -> World {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, config).unwrap();
        let tokens = Rc::new(RefCell::new(BTreeMap::<String, String>::new()));
        let notified = Rc::new(RefCell::new(Vec::new()));
        let titles = Rc::new(RefCell::new(Vec::new()));
        let record_titles = Rc::clone(&titles);
        let (read, save, delete, record) = (
            Rc::clone(&tokens),
            Rc::clone(&tokens),
            Rc::clone(&tokens),
            Rc::clone(&notified),
        );
        let env = Env {
            config: config_path,
            store: Store::new(dir.path().join("state")),
            api,
            token: Box::new(move |workspace| match read.borrow().get(&workspace.name) {
                Some(token) => Ok(Token::new(token.clone())),
                None => anyhow::bail!("no token for {}", workspace.name),
            }),
            save_token: Box::new(move |token, name| {
                save.borrow_mut().insert(name.to_owned(), token.as_str().to_owned());
                Ok(())
            }),
            delete_token: Box::new(move |name| {
                delete.borrow_mut().remove(name);
                Ok(())
            }),
            notify: Box::new(move |items, _users, teams| {
                record
                    .borrow_mut()
                    .extend(items.iter().map(|item| item.ts.clone()));
                record_titles
                    .borrow_mut()
                    .extend(items.iter().map(|item| crate::display::location(item, teams)));
            }),
            // 2026-11-01 12:00。さかのぼる日付は 7 日前の 2026-10-25
            now: Local.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap(),
        };
        World {
            env,
            tokens,
            notified,
            titles,
            _dir: dir,
        }
    }
}
