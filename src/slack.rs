use anyhow::{bail, Context, Result};
use reqwest::blocking::Client as HttpClient;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::credentials::Token;
use crate::l10n;
use crate::read::{ChannelState, ReadSource, ThreadState};

const API_URL: &str = "https://slack.com/api";

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
}

#[derive(Debug, Deserialize)]
pub struct AuthTest {
    pub user_id: String,
    pub team: String,
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
        let http = HttpClient::builder()
            .build()
            .with_context(|| l10n::text(l10n::HTTP_CLIENT_FAILED))?;
        Ok(Self { http, token })
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
            .post(format!("{API_URL}/{method}"))
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
