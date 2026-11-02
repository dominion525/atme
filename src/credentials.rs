use std::fmt;

use anyhow::{Context, Result};
use security_framework::base::Error as KeychainError;
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

/// Keychain の項目（service と account の組）の読み書き。ふだんは SystemKeychain、テストでは偽物
trait Keychain {
    fn get(&self, service: &str, account: &str) -> Result<Vec<u8>, KeychainError>;
    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), KeychainError>;
    fn delete(&self, service: &str, account: &str) -> Result<(), KeychainError>;
}

/// macOS の Keychain（security-framework）
struct SystemKeychain;

impl Keychain for SystemKeychain {
    fn get(&self, service: &str, account: &str) -> Result<Vec<u8>, KeychainError> {
        get_generic_password(service, account)
    }

    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), KeychainError> {
        set_generic_password(service, account, value)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), KeychainError> {
        delete_generic_password(service, account)
    }
}

impl Token {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn load(workspace: &Workspace) -> Result<Self> {
        load(&SystemKeychain, &workspace.name)
    }

    /// name のワークスペースの token として Keychain に保存する（あれば置き換える）
    pub fn save(&self, name: &str) -> Result<()> {
        save(&SystemKeychain, self, name)
    }

    /// name のワークスペースの token を Keychain から消す。無ければ何もしない
    pub fn delete(name: &str) -> Result<()> {
        delete(&SystemKeychain, name)
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

fn load(keychain: &impl Keychain, name: &str) -> Result<Token> {
    let account = account(name);
    let value = keychain.get(SERVICE, &account).map_err(|error| {
        anyhow::anyhow!(l10n::format(
            l10n::KEYCHAIN_READ_FAILED,
            &[SERVICE, &account, &error.to_string()],
        ))
    })?;
    let value = String::from_utf8(value)
        .with_context(|| l10n::format(l10n::KEYCHAIN_NOT_UTF8, &[&account]))?;
    Ok(Token(value))
}

fn save(keychain: &impl Keychain, token: &Token, name: &str) -> Result<()> {
    let account = account(name);
    keychain
        .set(SERVICE, &account, token.0.as_bytes())
        .with_context(|| l10n::format(l10n::KEYCHAIN_WRITE_FAILED, &[SERVICE, &account]))
}

fn delete(keychain: &impl Keychain, name: &str) -> Result<()> {
    let account = account(name);
    match keychain.delete(SERVICE, &account) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(error)
            .with_context(|| l10n::format(l10n::KEYCHAIN_DELETE_FAILED, &[SERVICE, &account])),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;

    // errSecAuthFailed（Keychain の利用を断られたとき）
    const ERR_SEC_AUTH_FAILED: i32 = -25293;

    /// (service, account) から値への表を持つ偽の Keychain。fail に入れたコードで、すべての操作を失敗させる
    #[derive(Default)]
    struct FakeKeychain {
        items: RefCell<BTreeMap<(String, String), Vec<u8>>>,
        fail: Option<i32>,
    }

    impl FakeKeychain {
        fn failing(code: i32) -> Self {
            Self {
                fail: Some(code),
                ..Self::default()
            }
        }

        fn check(&self) -> Result<(), KeychainError> {
            self.fail
                .map_or(Ok(()), |code| Err(KeychainError::from_code(code)))
        }

        fn key(service: &str, account: &str) -> (String, String) {
            (service.to_owned(), account.to_owned())
        }
    }

    impl Keychain for FakeKeychain {
        fn get(&self, service: &str, account: &str) -> Result<Vec<u8>, KeychainError> {
            self.check()?;
            self.items
                .borrow()
                .get(&Self::key(service, account))
                .cloned()
                .ok_or_else(|| KeychainError::from_code(ERR_SEC_ITEM_NOT_FOUND))
        }

        fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), KeychainError> {
            self.check()?;
            self.items
                .borrow_mut()
                .insert(Self::key(service, account), value.to_vec());
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), KeychainError> {
            self.check()?;
            self.items
                .borrow_mut()
                .remove(&Self::key(service, account))
                .map(|_| ())
                .ok_or_else(|| KeychainError::from_code(ERR_SEC_ITEM_NOT_FOUND))
        }
    }

    #[test]
    fn debug_hides_the_value() {
        assert_eq!(format!("{:?}", Token("xoxp-secret".into())), "Token(***)");
    }

    #[test]
    fn saved_token_is_loaded_under_atme_and_the_workspace_name() {
        let keychain = FakeKeychain::default();

        save(&keychain, &Token::new("xoxp-home".into()), "home").unwrap();

        assert_eq!(load(&keychain, "home").unwrap().as_str(), "xoxp-home");
        // ユーザーが security コマンドで手で入れたときと同じ組で保存する
        assert!(
            keychain
                .items
                .borrow()
                .contains_key(&("atme".to_owned(), "home/token".to_owned()))
        );
    }

    #[test]
    fn saving_again_replaces_the_token() {
        let keychain = FakeKeychain::default();

        save(&keychain, &Token::new("xoxp-old".into()), "home").unwrap();
        save(&keychain, &Token::new("xoxp-new".into()), "home").unwrap();

        assert_eq!(load(&keychain, "home").unwrap().as_str(), "xoxp-new");
    }

    #[test]
    fn missing_token_names_the_item() {
        let keychain = FakeKeychain::default();

        let error = load(&keychain, "home").unwrap_err();

        assert!(
            error
                .to_string()
                .starts_with("Couldn't read service=atme account=home/token from the Keychain: ")
        );
    }

    #[test]
    fn token_that_is_not_utf8_is_an_error() {
        let keychain = FakeKeychain::default();
        keychain.set(SERVICE, "home/token", &[0xff, 0xfe]).unwrap();

        let error = load(&keychain, "home").unwrap_err();

        assert_eq!(
            error.to_string(),
            "The value of account=home/token in the Keychain isn't UTF-8"
        );
    }

    #[test]
    fn deleting_removes_only_that_workspace() {
        let keychain = FakeKeychain::default();
        save(&keychain, &Token::new("xoxp-home".into()), "home").unwrap();
        save(&keychain, &Token::new("xoxp-work".into()), "work").unwrap();

        delete(&keychain, "home").unwrap();

        assert!(load(&keychain, "home").is_err());
        assert_eq!(load(&keychain, "work").unwrap().as_str(), "xoxp-work");
    }

    #[test]
    fn deleting_a_missing_token_is_not_an_error() {
        // 手で消したあとなど、もう無いものを外すときに止めない
        assert!(delete(&FakeKeychain::default(), "home").is_ok());
    }

    #[test]
    fn other_keychain_errors_are_reported() {
        let keychain = FakeKeychain::failing(ERR_SEC_AUTH_FAILED);

        let save_error = save(&keychain, &Token::new("xoxp-home".into()), "home").unwrap_err();
        let delete_error = delete(&keychain, "home").unwrap_err();

        assert_eq!(
            save_error.to_string(),
            "Couldn't save service=atme account=home/token to the Keychain"
        );
        assert_eq!(
            delete_error.to_string(),
            "Couldn't delete service=atme account=home/token from the Keychain"
        );
    }
}
