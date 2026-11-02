use anyhow::{bail, Context, Result};
use reqwest::blocking::Client as HttpClient;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::credentials::Token;
use crate::l10n;
use crate::read::{ChannelState, ReadSource, ThreadState};

pub const API_URL: &str = "https://slack.com/api";

impl ReadSource for Client {
    fn channel(&self, channel: &str) -> Option<ChannelState> {
        self.conversation_state(channel)
            .inspect_err(|error| eprintln!(
                    "{}",
                    l10n::format(l10n::CHANNEL_READ_STATE_FAILED, &[channel, &format!("{error:#}")])
                ))
            .ok()
    }

    fn thread(&self, channel: &str, thread_ts: &str) -> Option<ThreadState> {
        self.thread_state(channel, thread_ts)
            .inspect_err(|error| {
                eprintln!(
                    "{}",
                    l10n::format(
                        l10n::THREAD_READ_STATE_FAILED,
                        &[channel, thread_ts, &format!("{error:#}")],
                    )
                )
            })
            .ok()
    }
}

pub struct Client {
    http: HttpClient,
    token: Token,
    /// API の URL の頭（ふだんは API_URL。テストではモックサーバーに向ける）
    base: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthTest {
    pub user_id: String,
    pub team: String,
    /// ワークスペースの ID（T で始まる）。URL や名前が変わっても変わらない
    #[serde(default)]
    pub team_id: String,
    /// ワークスペースの URL（https://xxx.slack.com/）
    #[serde(default)]
    pub url: String,
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

#[derive(Deserialize)]
struct ConversationsInfoResponse {
    channel: ConversationInfo,
}

#[derive(Deserialize)]
struct ConversationInfo {
    #[serde(default)]
    is_archived: bool,
    last_read: Option<String>,
}

#[derive(Deserialize)]
struct RepliesResponse {
    messages: Vec<ReplyMessage>,
}

#[derive(Deserialize)]
struct ReplyMessage {
    #[serde(default)]
    subscribed: bool,
    last_read: Option<String>,
}

impl Client {
    pub fn new(token: Token) -> Result<Self> {
        Self::with_base(token, API_URL)
    }

    /// API の URL の頭を指定して作る
    pub fn with_base(token: Token, base: impl Into<String>) -> Result<Self> {
        let http = HttpClient::builder()
            .build()
            .with_context(|| l10n::text(l10n::HTTP_CLIENT_FAILED))?;
        Ok(Self {
            http,
            token,
            base: base.into(),
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

    pub fn conversation_state(&self, channel: &str) -> Result<ChannelState> {
        let response: ConversationsInfoResponse =
            self.call("conversations.info", &[("channel", channel)])?;
        Ok(ChannelState {
            archived: response.channel.is_archived,
            last_read: response.channel.last_read,
        })
    }

    /// スレッドの親のメッセージに付いてくる、自分の subscribe の有無と既読の位置を読む
    pub fn thread_state(&self, channel: &str, thread_ts: &str) -> Result<ThreadState> {
        let response: RepliesResponse = self.call(
            "conversations.replies",
            &[("channel", channel), ("ts", thread_ts), ("limit", "1")],
        )?;
        let parent = response
            .messages
            .into_iter()
            .next()
            .with_context(|| l10n::text(l10n::NO_THREAD_PARENT))?;
        Ok(ThreadState {
            subscribed: parent.subscribed,
            last_read: parent.last_read,
        })
    }

    fn call<T: DeserializeOwned>(&self, method: &str, params: &[(&str, &str)]) -> Result<T> {
        let response = self
            .http
            .post(format!("{}/{method}", self.base))
            .bearer_auth(self.token.as_str())
            .form(params)
            .send()
            .with_context(|| l10n::format(l10n::CALL_FAILED, &[method]))?;
        let status = response.status();
        if !status.is_success() {
            bail!(l10n::format(l10n::HTTP_STATUS, &[method, &status.to_string()]));
        }
        let body = response
            .text()
            .with_context(|| l10n::format(l10n::RESPONSE_READ_FAILED, &[method]))?;
        let envelope: Envelope = serde_json::from_str(&body)
            .with_context(|| l10n::format(l10n::RESPONSE_PARSE_FAILED, &[method]))?;
        if !envelope.ok {
            bail!(l10n::format(
                l10n::API_FAILED,
                &[method, &envelope.error.unwrap_or_default()],
            ));
        }
        serde_json::from_str(&body)
            .with_context(|| l10n::format(l10n::RESPONSE_PARSE_FAILED, &[method]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server, ServerGuard};

    const TOKEN: &str = "xoxp-test";

    fn client(server: &ServerGuard) -> Client {
        Client::with_base(Token::new(TOKEN.into()), server.url()).unwrap()
    }

    /// method への POST に、token の Authorization を付けて送ってきたら body を返す
    fn respond(server: &mut ServerGuard, method: &str, body: &str) -> mockito::Mock {
        server
            .mock("POST", format!("/{method}").as_str())
            .match_header("authorization", format!("Bearer {TOKEN}").as_str())
            .with_header("content-type", "application/json")
            .with_body(body)
            .create()
    }

    #[test]
    fn auth_test_sends_the_token_and_reads_the_workspace() {
        let mut server = Server::new();
        let mock = respond(
            &mut server,
            "auth.test",
            r#"{"ok":true,"user_id":"U1","team":"Acme","team_id":"T1","url":"https://acme.slack.com/"}"#,
        );

        let auth = client(&server).auth_test().unwrap();

        mock.assert();
        assert_eq!(auth.user_id, "U1");
        assert_eq!(auth.team, "Acme");
        assert_eq!(auth.team_id, "T1");
        assert_eq!(auth.url, "https://acme.slack.com/");
    }

    fn search_page(page: u32, pages: u32, ts: &str) -> String {
        format!(
            r#"{{"ok":true,"messages":{{"paging":{{"page":{page},"pages":{pages}}},"matches":[
                {{"channel":{{"id":"C1","name":"general"}},"user":"U2","ts":"{ts}","text":"hi","permalink":"https://acme.slack.com/archives/C1/p1"}}
            ]}}}}"#
        )
    }

    #[test]
    fn search_messages_reads_every_page() {
        let mut server = Server::new();
        let pages: Vec<_> = [("1", "1.000001"), ("2", "2.000002")]
            .iter()
            .map(|(page, ts)| {
                server
                    .mock("POST", "/search.messages")
                    .match_body(Matcher::AllOf(vec![
                        Matcher::UrlEncoded("query".into(), "to:me".into()),
                        Matcher::UrlEncoded("count".into(), "100".into()),
                        Matcher::UrlEncoded("page".into(), (*page).into()),
                    ]))
                    .with_body(search_page(page.parse().unwrap(), 2, ts))
                    .create()
            })
            .collect();

        let messages = client(&server).search_messages("to:me").unwrap();

        for page in pages {
            page.assert();
        }
        let ts: Vec<&str> = messages.iter().map(|message| message.ts.as_str()).collect();
        assert_eq!(ts, ["1.000001", "2.000002"]);
        assert_eq!(messages[0].channel.name, "general");
        assert!(!messages[0].channel.is_im);
    }

    #[test]
    fn users_info_reads_the_user() {
        let mut server = Server::new();
        respond(
            &mut server,
            "users.info",
            r#"{"ok":true,"user":{"name":"sato","profile":{"display_name":"","real_name":"Sato Taro"}}}"#,
        );

        let user = client(&server).users_info("U2").unwrap();

        assert_eq!(user.label(), "Sato Taro");
    }

    #[test]
    fn label_prefers_display_name_then_real_name_then_name() {
        let user = |display: &str, real: &str| User {
            name: "sato".into(),
            profile: Profile {
                display_name: display.into(),
                real_name: real.into(),
            },
        };

        assert_eq!(user("Taro", "Sato Taro").label(), "Taro");
        assert_eq!(user("", "Sato Taro").label(), "Sato Taro");
        assert_eq!(user("", "").label(), "sato");
    }

    #[test]
    fn conversation_state_reads_archived_and_last_read() {
        let mut server = Server::new();
        respond(
            &mut server,
            "conversations.info",
            r#"{"ok":true,"channel":{"is_archived":true,"last_read":"1.000100"}}"#,
        );

        let state = client(&server).conversation_state("C1").unwrap();

        assert!(state.archived);
        assert_eq!(state.last_read.as_deref(), Some("1.000100"));
    }

    #[test]
    fn thread_state_reads_the_parent() {
        let mut server = Server::new();
        server
            .mock("POST", "/conversations.replies")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("channel".into(), "C1".into()),
                Matcher::UrlEncoded("ts".into(), "1.000100".into()),
                Matcher::UrlEncoded("limit".into(), "1".into()),
            ]))
            .with_body(r#"{"ok":true,"messages":[{"subscribed":true,"last_read":"1.000200"}]}"#)
            .create();

        let state = client(&server).thread_state("C1", "1.000100").unwrap();

        assert!(state.subscribed);
        assert_eq!(state.last_read.as_deref(), Some("1.000200"));
    }

    #[test]
    fn thread_without_parent_is_an_error() {
        let mut server = Server::new();
        respond(&mut server, "conversations.replies", r#"{"ok":true,"messages":[]}"#);

        let error = client(&server).thread_state("C1", "1.000100").unwrap_err();

        assert_eq!(
            error.to_string(),
            "conversations.replies didn't return the thread parent"
        );
    }

    #[test]
    fn slack_error_is_reported_with_its_code() {
        let mut server = Server::new();
        respond(&mut server, "auth.test", r#"{"ok":false,"error":"invalid_auth"}"#);

        let error = client(&server).auth_test().unwrap_err();

        assert_eq!(error.to_string(), "auth.test failed: invalid_auth");
    }

    #[test]
    fn http_error_is_reported_with_its_status() {
        let mut server = Server::new();
        server.mock("POST", "/auth.test").with_status(500).create();

        let error = client(&server).auth_test().unwrap_err();

        assert_eq!(
            error.to_string(),
            "auth.test returned HTTP 500 Internal Server Error"
        );
    }

    #[test]
    fn broken_response_is_reported() {
        let mut server = Server::new();
        respond(&mut server, "auth.test", "<html>not json</html>");

        let error = client(&server).auth_test().unwrap_err();

        assert_eq!(error.to_string(), "Couldn't parse the response of auth.test");
    }

    #[test]
    fn unreachable_server_is_reported() {
        // 空いているポートを 1 つ借りてすぐ返し、何も待ち受けていないそのポートに向ける
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let client =
            Client::with_base(Token::new(TOKEN.into()), format!("http://127.0.0.1:{port}")).unwrap();

        let error = client.auth_test().unwrap_err();

        assert_eq!(error.to_string(), "Couldn't call auth.test");
    }

    #[test]
    fn read_source_treats_errors_as_unknown() {
        let mut server = Server::new();
        respond(&mut server, "conversations.info", r#"{"ok":false,"error":"channel_not_found"}"#);
        respond(&mut server, "conversations.replies", r#"{"ok":false,"error":"thread_not_found"}"#);
        let client = client(&server);

        assert!(client.channel("C1").is_none());
        assert!(client.thread("C1", "1.000100").is_none());
    }
}
