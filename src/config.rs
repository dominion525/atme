use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

use crate::l10n;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_days")]
    pub days: u32,
    #[serde(default)]
    pub workspace: Vec<Workspace>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    /// Keychain の account（`<name>/token`）や記録のキーに使う名前
    pub name: String,
}

fn default_days() -> u32 {
    7
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = path()?;
        let text = std::fs::read_to_string(&path)
            .with_context(|| l10n::format(l10n::READ_FAILED, &[&path.display().to_string()]))?;
        Self::parse(&text).with_context(|| l10n::format(l10n::INVALID_CONTENTS, &[&path.display().to_string()]))
    }

    fn parse(text: &str) -> Result<Self> {
        let config: Config = toml::from_str(text)?;
        if config.workspace.is_empty() {
            bail!(l10n::text(l10n::NO_WORKSPACE));
        }
        let mut names = HashSet::new();
        for workspace in &config.workspace {
            if !names.insert(workspace.name.as_str()) {
                bail!(l10n::format(l10n::DUPLICATE_WORKSPACE, &[&workspace.name]));
            }
        }
        Ok(config)
    }
}

/// 設定ファイルに name のワークスペースを足す。ファイルが無ければ作る
pub fn add_workspace(name: &str) -> Result<()> {
    edit(|text| with_workspace(text, name))
}

/// 設定ファイルから name のワークスペースを外す
pub fn remove_workspace(name: &str) -> Result<()> {
    edit(|text| without_workspace(text, name))
}

/// 設定ファイルを読み、change で書き換えて保存する。ユーザーが書いたコメントや並びは残す
fn edit(change: impl FnOnce(&str) -> Result<String>) -> Result<()> {
    let path = path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error)
                .with_context(|| l10n::format(l10n::READ_FAILED, &[&path.display().to_string()]));
        }
    };
    let changed = change(&text)
        .with_context(|| l10n::format(l10n::INVALID_CONTENTS, &[&path.display().to_string()]))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| l10n::format(l10n::CREATE_FAILED, &[&dir.display().to_string()]))?;
    }
    std::fs::write(&path, changed)
        .with_context(|| l10n::format(l10n::WRITE_FAILED, &[&path.display().to_string()]))
}

fn with_workspace(text: &str, name: &str) -> Result<String> {
    let mut doc: DocumentMut = text.parse()?;
    let tables = doc
        .entry("workspace")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .with_context(|| l10n::text(l10n::NO_WORKSPACE))?;
    if tables.iter().any(|table| table_name(table) == Some(name)) {
        bail!(l10n::format(l10n::ALREADY_ADDED, &[name]));
    }
    let mut table = Table::new();
    table["name"] = value(name);
    tables.push(table);
    Ok(doc.to_string())
}

fn without_workspace(text: &str, name: &str) -> Result<String> {
    let mut doc: DocumentMut = text.parse()?;
    if let Some(tables) = doc
        .get_mut("workspace")
        .and_then(Item::as_array_of_tables_mut)
    {
        tables.retain(|table| table_name(table) != Some(name));
    }
    Ok(doc.to_string())
}

fn table_name(table: &Table) -> Option<&str> {
    table.get("name").and_then(Item::as_str)
}

pub fn path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").with_context(|| l10n::text(l10n::HOME_NOT_SET))?;
    Ok(PathBuf::from(home).join(".config/atme/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_a_workspace_keeping_comments() {
        let text = "# 自分の設定\ndays = 3\n\n[[workspace]]\nname = \"home\"\n";

        let changed = with_workspace(text, "work").unwrap();

        assert_eq!(
            changed,
            "# 自分の設定\ndays = 3\n\n[[workspace]]\nname = \"home\"\n\n[[workspace]]\nname = \"work\"\n"
        );
        assert_eq!(Config::parse(&changed).unwrap().workspace.len(), 2);
    }

    #[test]
    fn adds_to_an_empty_file() {
        assert_eq!(
            with_workspace("", "home").unwrap(),
            "[[workspace]]\nname = \"home\"\n"
        );
    }

    #[test]
    fn refuses_to_add_twice() {
        let error = with_workspace("[[workspace]]\nname = \"home\"\n", "home").unwrap_err();

        assert_eq!(error.to_string(), "home has already been added");
    }

    #[test]
    fn removes_a_workspace() {
        let text = "days = 3\n\n[[workspace]]\nname = \"home\"\n\n[[workspace]]\nname = \"work\"\n";

        assert_eq!(
            without_workspace(text, "home").unwrap(),
            "days = 3\n\n[[workspace]]\nname = \"work\"\n"
        );
    }

    #[test]
    fn parses_workspaces() {
        let config = Config::parse(
            r#"
            days = 3

            [[workspace]]
            name = "home"

            [[workspace]]
            name = "work"
            "#,
        )
        .unwrap();

        assert_eq!(config.days, 3);
        assert_eq!(config.workspace.len(), 2);
        assert_eq!(config.workspace[0].name, "home");
        assert_eq!(config.workspace[1].name, "work");
    }

    #[test]
    fn days_defaults_to_7() {
        let config = Config::parse(
            r#"
            [[workspace]]
            name = "home"
            "#,
        )
        .unwrap();

        assert_eq!(config.days, 7);
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(Config::parse("dayz = 3").is_err());
        // token を設定に書かせない。session 方式で使っていた domain と auth も、もう受け付けない
        for key in [
            r#"token = "xoxp-""#,
            r#"domain = "home-team""#,
            r#"auth = "token""#,
        ] {
            let text = format!("[[workspace]]\nname = \"home\"\n{key}\n");
            assert!(Config::parse(&text).is_err(), "{key}");
        }
    }

    #[test]
    fn rejects_no_workspace() {
        assert!(Config::parse("days = 7").is_err());
    }

    #[test]
    fn rejects_duplicate_names() {
        let error = Config::parse(
            r#"
            [[workspace]]
            name = "home"

            [[workspace]]
            name = "home"
            "#,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "Workspace name home is duplicated");
    }
}
