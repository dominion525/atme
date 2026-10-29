use std::fmt;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::Workspace;

const SERVICE: &str = "atme";

/// Slack の user token（xoxp-）。表示するときは値を伏せる
pub struct Token(String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(***)")
    }
}

impl Token {
    pub fn load(workspace: &Workspace) -> Result<Self> {
        Ok(Self(read(&format!("{}/token", workspace.name))?))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn read(account: &str) -> Result<String> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"])
        .output()
        .context("security コマンドを実行できませんでした")?;
    if !output.status.success() {
        bail!(
            "Keychain から service={SERVICE} account={account} を読めませんでした: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let value = String::from_utf8(output.stdout)
        .with_context(|| format!("Keychain の account={account} の値が UTF-8 ではありません"))?;
    Ok(value.trim_end_matches('\n').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_value() {
        assert_eq!(format!("{:?}", Token("xoxp-secret".into())), "Token(***)");
    }
}
