use anyhow::{bail, Context, Result};
use reqwest::blocking::Client as HttpClient;
use reqwest::header::COOKIE;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::config::Workspace;
use crate::credentials::Credentials;

pub struct Client {
    http: HttpClient,
    base_url: String,
    credentials: Credentials,
}

#[derive(Debug, Deserialize)]
pub struct AuthTest {
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub channel: Channel,
    pub user: Option<String>,
    pub username: Option<String>,
    pub ts: String,
    pub text: String,
    pub permalink: String,
}

#[derive(Debug, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub is_im: bool,
    #[serde(default)]
    pub is_mpim: bool,
}

#[derive(Debug, Deserialize)]
pub struct User {
    pub name: String,
    #[serde(default)]
    pub profile: Profile,
}

#[derive(Debug, Default, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub real_name: String,
}

impl User {
    pub fn label(&self) -> &str {
        [
            &self.profile.display_name,
            &self.profile.real_name,
            &self.name,
        ]
        .into_iter()
        .find(|s| !s.is_empty())
        .map_or("", |s| s.as_str())
    }
}

#[derive(Deserialize)]
struct Envelope {
    ok: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct SearchResponse {
    messages: SearchMessages,
}

#[derive(Deserialize)]
struct SearchMessages {
    matches: Vec<Message>,
    paging: Paging,
}

#[derive(Deserialize)]
struct Paging {
    pages: u32,
}

#[derive(Deserialize)]
struct UsersInfoResponse {
    user: User,
}

impl Client {
    pub fn new(workspace: &Workspace, credentials: Credentials) -> Result<Self> {
        let base_url = match credentials {
            Credentials::Token(_) => "https://slack.com/api".to_owned(),
            Credentials::Session { .. } => format!("https://{}.slack.com/api", workspace.domain),
        };
        let http = HttpClient::builder()
            .build()
            .context("HTTP クライアントを作れませんでした")?;
        Ok(Self {
            http,
            base_url,
            credentials,
        })
    }

    pub fn auth_test(&self) -> Result<AuthTest> {
        self.call("auth.test", &[])
    }

    pub fn search_messages(&self, query: &str) -> Result<Vec<Message>> {
        let mut messages = Vec::new();
        let mut page = 1;
        loop {
            let page_param = page.to_string();
            let response: SearchResponse = self.call(
                "search.messages",
                &[("query", query), ("count", "100"), ("page", &page_param)],
            )?;
            messages.extend(response.messages.matches);
            if page >= response.messages.paging.pages {
                break;
            }
            page += 1;
        }
        Ok(messages)
    }

    pub fn users_info(&self, user: &str) -> Result<User> {
        let response: UsersInfoResponse = self.call("users.info", &[("user", user)])?;
        Ok(response.user)
    }

    fn call<T: DeserializeOwned>(&self, method: &str, params: &[(&str, &str)]) -> Result<T> {
        let url = format!("{}/{method}", self.base_url);
        let request = match &self.credentials {
            Credentials::Token(token) => self.http.post(&url).bearer_auth(token).form(params),
            Credentials::Session { token, cookie } => {
                let mut form = vec![("token", token.as_str())];
                form.extend_from_slice(params);
                self.http
                    .post(&url)
                    .header(COOKIE, format!("d={cookie}"))
                    .form(&form)
            }
        };
        let response = request
            .send()
            .with_context(|| format!("{method} を呼び出せませんでした"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("{method} が HTTP {status} を返しました");
        }
        let body = response
            .text()
            .with_context(|| format!("{method} の応答を読めませんでした"))?;
        let envelope: Envelope = serde_json::from_str(&body)
            .with_context(|| format!("{method} の応答を解釈できませんでした"))?;
        if !envelope.ok {
            bail!(
                "{method} が失敗しました: {}",
                envelope.error.unwrap_or_default()
            );
        }
        serde_json::from_str(&body)
            .with_context(|| format!("{method} の応答を解釈できませんでした"))
    }
}
