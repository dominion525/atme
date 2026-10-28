use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
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

pub type Users = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    pub fetched_at: i64,
    pub items: Vec<Item>,
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
}

impl Item {
    pub fn key(&self) -> SeenKey {
        SeenKey {
            workspace: self.workspace.clone(),
            channel: self.channel_id.clone(),
            ts: self.ts.clone(),
        }
    }
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
    fs::create_dir_all(dir).with_context(|| format!("{} を作れませんでした", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)
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
}
