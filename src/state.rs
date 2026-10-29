use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

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
        let home = std::env::var_os("HOME").context("HOME が設定されていません")?;
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
            return Err(error).with_context(|| format!("{} を読めませんでした", path.display()));
        }
    };
    let value = serde_json::from_str(&text)
        .with_context(|| format!("{} の内容が正しくありません", path.display()))?;
    Ok(Some(value))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let dir = path.parent().context("保存先のディレクトリがありません")?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("{} を作れませんでした", dir.display()))?;
    fs::set_permissions(dir, Permissions::from_mode(0o700))
        .with_context(|| format!("{} の権限を設定できませんでした", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .with_context(|| format!("{} を開けませんでした", tmp.display()))?;
    file.set_permissions(Permissions::from_mode(0o600))
        .with_context(|| format!("{} の権限を設定できませんでした", tmp.display()))?;
    file.write_all(&serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("{} に書けませんでした", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("{} に置き換えられませんでした", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("atme-test-{}-{name}", std::process::id()));
        Store::new(dir)
    }

    #[test]
    fn missing_files_are_none() {
        let store = temp_store("missing");
        assert!(store.load_seen().unwrap().is_none());
        assert!(store.load_cache().unwrap().is_none());
        assert!(store.load_users().unwrap().is_none());
    }

    #[test]
    fn round_trips_seen() {
        let store = temp_store("seen");
        let seen: Seen = [SeenKey {
            workspace: "home".into(),
            channel: "C1".into(),
            ts: "1793176152.520719".into(),
        }]
        .into();

        store.save_seen(&seen).unwrap();

        assert_eq!(store.load_seen().unwrap(), Some(seen));
        assert!(!store.dir.join("seen.json.tmp").exists());
    }

    #[test]
    fn files_are_owner_only() {
        let store = temp_store("permissions");
        fs::create_dir_all(&store.dir).unwrap();
        fs::set_permissions(&store.dir, Permissions::from_mode(0o755)).unwrap();

        store.save_seen(&Seen::new()).unwrap();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&store.dir), 0o700);
        assert_eq!(mode(&store.dir.join("seen.json")), 0o600);
    }
}
