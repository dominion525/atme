use std::fmt;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::Workspace;
use crate::l10n;

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
        .with_context(|| l10n::text(l10n::SECURITY_FAILED))?;
    if !output.status.success() {
        bail!(l10n::format(
            l10n::KEYCHAIN_READ_FAILED,
            &[SERVICE, account, String::from_utf8_lossy(&output.stderr).trim()],
        ));
    }
    let value = String::from_utf8(output.stdout)
        .with_context(|| l10n::format(l10n::KEYCHAIN_NOT_UTF8, &[account]))?;
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
