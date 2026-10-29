use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

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
            .with_context(|| format!("{} を読めませんでした", path.display()))?;
        Self::parse(&text).with_context(|| format!("{} の内容が正しくありません", path.display()))
    }

    fn parse(text: &str) -> Result<Self> {
        let config: Config = toml::from_str(text)?;
        if config.workspace.is_empty() {
            bail!("[[workspace]] が 1 つもありません");
        }
        let mut names = HashSet::new();
        for workspace in &config.workspace {
            if !names.insert(workspace.name.as_str()) {
                bail!("ワークスペース名 {} が重複しています", workspace.name);
            }
        }
        Ok(config)
    }
}

pub fn path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME が設定されていません")?;
    Ok(PathBuf::from(home).join(".config/atme/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

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

        assert!(error.to_string().contains("重複"));
    }
}
