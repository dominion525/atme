use std::fmt;

use anyhow::{Context, Result};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

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
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn load(workspace: &Workspace) -> Result<Self> {
        Ok(Self(read(&account(&workspace.name))?))
    }

    /// name のワークスペースの token として Keychain に保存する（あれば置き換える）
    pub fn save(&self, name: &str) -> Result<()> {
        let account = account(name);
        set_generic_password(SERVICE, &account, self.0.as_bytes()).with_context(|| {
            l10n::format(l10n::KEYCHAIN_WRITE_FAILED, &[SERVICE, &account])
        })
    }

    /// name のワークスペースの token を Keychain から消す。無ければ何もしない
    pub fn delete(name: &str) -> Result<()> {
        let account = account(name);
        match delete_generic_password(SERVICE, &account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(error) => Err(error).with_context(|| {
                l10n::format(l10n::KEYCHAIN_DELETE_FAILED, &[SERVICE, &account])
            }),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// errSecItemNotFound
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

fn account(name: &str) -> String {
    format!("{name}/token")
}

fn read(account: &str) -> Result<String> {
    let value = get_generic_password(SERVICE, account).map_err(|error| {
        anyhow::anyhow!(l10n::format(
            l10n::KEYCHAIN_READ_FAILED,
            &[SERVICE, account, &error.to_string()],
        ))
    })?;
    String::from_utf8(value).with_context(|| l10n::format(l10n::KEYCHAIN_NOT_UTF8, &[account]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_value() {
        assert_eq!(format!("{:?}", Token("xoxp-secret".into())), "Token(***)");
    }

    #[test]
    fn account_is_name_and_token() {
        assert_eq!(account("home"), "home/token");
    }
}
