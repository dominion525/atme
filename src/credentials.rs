use std::fmt;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::{Auth, Workspace};

const SERVICE: &str = "atme";

pub enum Credentials {
    Token(String),
    Session { token: String, cookie: String },
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credentials::Token(_) => f.write_str("Token(***)"),
            Credentials::Session { .. } => f.write_str("Session { token: ***, cookie: *** }"),
        }
    }
}

impl Credentials {
    pub fn load(workspace: &Workspace) -> Result<Self> {
        let token = read(&format!("{}/token", workspace.name))?;
        match workspace.auth {
            Auth::Token => Ok(Credentials::Token(token)),
            Auth::Session => {
                let cookie = read(&format!("{}/cookie", workspace.name))?;
                Ok(Credentials::Session { token, cookie })
            }
        }
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
