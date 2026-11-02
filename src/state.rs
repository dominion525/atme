use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::l10n;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SeenKey {
    pub workspace: String,
    pub channel: String,
    pub ts: String,
}

pub type Seen = BTreeSet<SeenKey>;

/// atme から開いたもの・「すべて既読にする」で消したもの
pub type Dismissed = BTreeSet<SeenKey>;

pub type Users = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    pub fetched_at: i64,
    pub items: Vec<Item>,
    /// 設定の name から、Slack 上のワークスペース名への対応
    #[serde(default)]
    pub teams: BTreeMap<String, String>,
    /// 設定の name から、Slack のワークスペースの ID（team_id）への対応。追加のときに同じワークスペースを見分けるのに使う
    #[serde(default)]
    pub team_ids: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub workspace: String,
    pub channel_id: String,
    pub channel_name: String,
    pub is_im: bool,
    pub is_mpim: bool,
    pub peer_name: Option<String>,
    pub user: Option<String>,
    pub user_name: Option<String>,
    pub ts: String,
    pub text: String,
    pub permalink: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
}

impl Item {
    /// スレッドの返信なら、スレッドの親の ts を返す（親のメッセージ自身は返信ではない）
    pub fn reply_parent(&self) -> Option<&str> {
        self.thread_ts
            .as_deref()
            .filter(|parent| *parent != self.ts)
    }

    pub fn key(&self) -> SeenKey {
        SeenKey {
            workspace: self.workspace.clone(),
            channel: self.channel_id.clone(),
            ts: self.ts.clone(),
        }
    }
}

pub fn ts_seconds(ts: &str) -> i64 {
    ts.split('.')
        .next()
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(0)
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open() -> Result<Self> {
        let home = std::env::var_os("HOME").with_context(|| l10n::text(l10n::HOME_NOT_SET))?;
        Ok(Self::new(PathBuf::from(home).join(".local/state/atme")))
    }

    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn load_seen(&self) -> Result<Option<Seen>> {
        read_json(&self.dir.join("seen.json"))
    }

    pub fn save_seen(&self, seen: &Seen) -> Result<()> {
        write_json(&self.dir.join("seen.json"), seen)
    }

    pub fn load_cache(&self) -> Result<Option<Cache>> {
        read_json(&self.dir.join("cache.json"))
    }

    pub fn save_cache(&self, cache: &Cache) -> Result<()> {
        write_json(&self.dir.join("cache.json"), cache)
    }

    pub fn load_users(&self) -> Result<Option<Users>> {
        read_json(&self.dir.join("users.json"))
    }

    pub fn save_users(&self, users: &Users) -> Result<()> {
        write_json(&self.dir.join("users.json"), users)
    }

    pub fn load_dismissed(&self) -> Result<Option<Dismissed>> {
        read_json(&self.dir.join("dismissed.json"))
    }

    pub fn save_dismissed(&self, dismissed: &Dismissed) -> Result<()> {
        write_json(&self.dir.join("dismissed.json"), dismissed)
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| l10n::format(l10n::READ_FAILED, &[&path.display().to_string()]));
        }
    };
    let value = serde_json::from_str(&text)
        .with_context(|| l10n::format(l10n::INVALID_CONTENTS, &[&path.display().to_string()]))?;
    Ok(Some(value))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let dir = path.parent().with_context(|| l10n::text(l10n::NO_SAVE_DIR))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| l10n::format(l10n::CREATE_FAILED, &[&dir.display().to_string()]))?;
    fs::set_permissions(dir, Permissions::from_mode(0o700))
        .with_context(|| l10n::format(l10n::PERMISSION_FAILED, &[&dir.display().to_string()]))?;
    let tmp = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .with_context(|| l10n::format(l10n::OPEN_FAILED, &[&tmp.display().to_string()]))?;
    file.set_permissions(Permissions::from_mode(0o600))
        .with_context(|| l10n::format(l10n::PERMISSION_FAILED, &[&tmp.display().to_string()]))?;
    file.write_all(&serde_json::to_vec_pretty(value)?)
        .with_context(|| l10n::format(l10n::WRITE_FAILED, &[&tmp.display().to_string()]))?;
    fs::rename(&tmp, path).with_context(|| l10n::format(l10n::REPLACE_FAILED, &[&path.display().to_string()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一時的なフォルダの中の、まだ無い状態のフォルダを使う Store。フォルダはテストが終わると消える
    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("state"));
        (dir, store)
    }

    fn key(ts: &str) -> SeenKey {
        SeenKey {
            workspace: "home".into(),
            channel: "C1".into(),
            ts: ts.into(),
        }
    }

    fn item() -> Item {
        Item {
            workspace: "home".into(),
            channel_id: "C1".into(),
            channel_name: "general".into(),
            is_im: false,
            is_mpim: false,
            peer_name: None,
            user: Some("U1".into()),
            user_name: Some("佐藤".into()),
            ts: "1793176152.520719".into(),
            text: "本文".into(),
            permalink: "https://example.slack.com/archives/C1/p1793176152520719".into(),
            thread_ts: Some("1793176000.000100".into()),
        }
    }

    #[test]
    fn missing_files_are_none() {
        let (_dir, store) = temp_store();
        assert!(store.load_seen().unwrap().is_none());
        assert!(store.load_cache().unwrap().is_none());
        assert!(store.load_users().unwrap().is_none());
        assert!(store.load_dismissed().unwrap().is_none());
    }

    #[test]
    fn round_trips_seen() {
        let (_dir, store) = temp_store();
        let seen: Seen = [key("1793176152.520719")].into();

        store.save_seen(&seen).unwrap();

        assert_eq!(store.load_seen().unwrap(), Some(seen));
        assert!(!store.dir.join("seen.json.tmp").exists());
    }

    #[test]
    fn round_trips_cache() {
        let (_dir, store) = temp_store();
        let cache = Cache {
            fetched_at: 1793181900,
            items: vec![item()],
            teams: BTreeMap::from([("home".to_owned(), "Home".to_owned())]),
            team_ids: BTreeMap::from([("home".to_owned(), "T1".to_owned())]),
        };

        store.save_cache(&cache).unwrap();
        let loaded = store.load_cache().unwrap().unwrap();

        assert_eq!(loaded.fetched_at, cache.fetched_at);
        assert_eq!(loaded.items, cache.items);
        assert_eq!(loaded.teams, cache.teams);
        assert_eq!(loaded.team_ids, cache.team_ids);
    }

    #[test]
    fn reads_a_cache_written_by_an_older_version() {
        // teams・team_ids・thread_ts が無かった頃のキャッシュも読める
        let (_dir, store) = temp_store();
        fs::create_dir_all(&store.dir).unwrap();
        fs::write(
            store.dir.join("cache.json"),
            r#"{"fetched_at":1,"items":[{"workspace":"home","channel_id":"C1","channel_name":"general","is_im":false,"is_mpim":false,"peer_name":null,"user":null,"user_name":null,"ts":"1.000100","text":"","permalink":"https://example.slack.com/"}]}"#,
        )
        .unwrap();

        let cache = store.load_cache().unwrap().unwrap();

        assert!(cache.teams.is_empty());
        assert!(cache.team_ids.is_empty());
        assert_eq!(cache.items[0].thread_ts, None);
    }

    #[test]
    fn round_trips_users_and_dismissed() {
        let (_dir, store) = temp_store();
        let users: Users = BTreeMap::from([(
            "home".to_owned(),
            BTreeMap::from([("U1".to_owned(), "佐藤".to_owned())]),
        )]);
        let dismissed: Dismissed = [key("1.000100"), key("2.000200")].into();

        store.save_users(&users).unwrap();
        store.save_dismissed(&dismissed).unwrap();

        assert_eq!(store.load_users().unwrap(), Some(users));
        assert_eq!(store.load_dismissed().unwrap(), Some(dismissed));
    }

    #[test]
    fn broken_file_names_the_path() {
        let (_dir, store) = temp_store();
        fs::create_dir_all(&store.dir).unwrap();
        let path = store.dir.join("cache.json");
        fs::write(&path, "{ not json").unwrap();

        let error = store.load_cache().unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("The contents of {} are invalid", path.display())
        );
    }

    #[test]
    fn files_are_owner_only() {
        let (_dir, store) = temp_store();
        fs::create_dir_all(&store.dir).unwrap();
        fs::set_permissions(&store.dir, Permissions::from_mode(0o755)).unwrap();

        store.save_seen(&Seen::new()).unwrap();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&store.dir), 0o700);
        assert_eq!(mode(&store.dir.join("seen.json")), 0o600);
    }
}
