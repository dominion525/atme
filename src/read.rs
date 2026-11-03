use std::collections::HashMap;

use crate::state::Item;

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelState {
    pub archived: bool,
    pub last_read: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadState {
    pub subscribed: bool,
    pub last_read: Option<String>,
}

/// チャンネルとスレッドの既読の位置を取ってくるもの。取れなかったときは None を返す
pub trait ReadSource {
    fn channel(&self, channel: &str) -> Option<ChannelState>;
    fn thread(&self, channel: &str, thread_ts: &str) -> Option<ThreadState>;
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Unread,
    Read,
    /// 自分が subscribe していないスレッドの返信。一覧に入れない
    Skip,
}

pub fn judge(item: &Item, channel: &ChannelState, thread: Option<&ThreadState>) -> Verdict {
    if channel.archived {
        return Verdict::Read;
    }
    if item.reply_parent().is_some() {
        return match thread {
            Some(ThreadState {
                subscribed: true,
                last_read,
            }) => compare(&item.ts, last_read.as_deref()),
            _ => Verdict::Skip,
        };
    }
    compare(&item.ts, channel.last_read.as_deref())
}

fn compare(ts: &str, last_read: Option<&str>) -> Verdict {
    match last_read {
        Some(last_read) if order(ts) <= order(last_read) => Verdict::Read,
        _ => Verdict::Unread,
    }
}

/// ts（"1793176152.520719"）を秒とマイクロ秒の組にして、大小を比べられるようにする
fn order(ts: &str) -> (i64, i64) {
    let (seconds, fraction) = ts.split_once('.').unwrap_or((ts, "0"));
    (
        seconds.parse().unwrap_or(0),
        format!("{fraction:0<6}")[..6].parse().unwrap_or(0),
    )
}

/// 未読のものだけを残す。チャンネルとスレッドの既読の位置は、それぞれ 1 度だけ取りに行く。
/// 既読の位置を取れなかったものは、消さずに未読として残す
pub fn unread_only(items: Vec<Item>, source: &impl ReadSource) -> Vec<Item> {
    let mut channels: HashMap<String, Option<ChannelState>> = HashMap::new();
    let mut threads: HashMap<(String, String), Option<ThreadState>> = HashMap::new();
    items
        .into_iter()
        .filter(|item| {
            let channel = channels
                .entry(item.channel_id.clone())
                .or_insert_with(|| source.channel(&item.channel_id));
            let Some(channel) = channel.as_ref() else {
                return true;
            };
            if channel.archived {
                return false;
            }
            let thread = match item.reply_parent() {
                Some(parent) => {
                    let thread = threads
                        .entry((item.channel_id.clone(), parent.to_owned()))
                        .or_insert_with(|| source.thread(&item.channel_id, parent));
                    let Some(thread) = thread.as_ref() else {
                        return true;
                    };
                    Some(thread)
                }
                None => None,
            };
            judge(item, channel, thread) == Verdict::Unread
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::display::tests::item;

    fn channel(last_read: Option<&str>) -> ChannelState {
        ChannelState {
            archived: false,
            last_read: last_read.map(Into::into),
        }
    }

    fn thread(subscribed: bool, last_read: Option<&str>) -> ThreadState {
        ThreadState {
            subscribed,
            last_read: last_read.map(Into::into),
        }
    }

    fn reply(channel: &str, ts: &str, parent: &str) -> Item {
        let mut item = item(channel, ts);
        item.thread_ts = Some(parent.into());
        item
    }

    #[test]
    fn main_message_is_read_up_to_last_read() {
        let state = channel(Some("100.000002"));
        assert_eq!(
            judge(&item("C1", "100.000002"), &state, None),
            Verdict::Read
        );
        assert_eq!(
            judge(&item("C1", "100.000003"), &state, None),
            Verdict::Unread
        );
        assert_eq!(
            judge(&item("C1", "100.000003"), &channel(None), None),
            Verdict::Unread
        );
    }

    #[test]
    fn thread_parent_counts_as_main_message() {
        let mut parent = item("C1", "100.000001");
        parent.thread_ts = Some("100.000001".into());

        assert_eq!(
            judge(&parent, &channel(Some("100.000001")), None),
            Verdict::Read
        );
    }

    #[test]
    fn reply_uses_thread_last_read() {
        let message = reply("C1", "100.000005", "100.000001");
        let state = channel(Some("999.000000"));

        assert_eq!(
            judge(&message, &state, Some(&thread(true, Some("100.000004")))),
            Verdict::Unread
        );
        assert_eq!(
            judge(&message, &state, Some(&thread(true, Some("100.000005")))),
            Verdict::Read
        );
    }

    #[test]
    fn reply_in_unsubscribed_thread_is_skipped() {
        let message = reply("C1", "100.000005", "100.000001");

        assert_eq!(
            judge(&message, &channel(None), Some(&thread(false, None))),
            Verdict::Skip
        );
    }

    #[test]
    fn archived_channel_is_read() {
        let state = ChannelState {
            archived: true,
            last_read: Some("0000000000.000000".into()),
        };

        assert_eq!(
            judge(&item("C1", "100.000001"), &state, None),
            Verdict::Read
        );
        assert_eq!(
            judge(&reply("C1", "100.000005", "100.000001"), &state, None),
            Verdict::Read
        );
    }

    #[test]
    fn orders_ts_numerically() {
        assert!(order("99.999999") < order("100.000000"));
        assert!(order("100.1") > order("100.000002"));
        assert_eq!(order("100"), (100, 0));
    }

    struct Fake {
        channels: HashMap<String, ChannelState>,
        threads: HashMap<(String, String), ThreadState>,
        calls: RefCell<Vec<String>>,
    }

    impl ReadSource for Fake {
        fn channel(&self, channel: &str) -> Option<ChannelState> {
            self.calls.borrow_mut().push(format!("channel {channel}"));
            self.channels.get(channel).cloned()
        }

        fn thread(&self, channel: &str, thread_ts: &str) -> Option<ThreadState> {
            self.calls
                .borrow_mut()
                .push(format!("thread {channel} {thread_ts}"));
            self.threads
                .get(&(channel.to_owned(), thread_ts.to_owned()))
                .cloned()
        }
    }

    #[test]
    fn keeps_only_unread_and_asks_each_source_once() {
        let fake = Fake {
            channels: HashMap::from([("C1".to_owned(), channel(Some("100.000002")))]),
            threads: HashMap::from([
                (
                    ("C1".to_owned(), "100.000001".to_owned()),
                    thread(true, Some("100.000003")),
                ),
                (
                    ("C1".to_owned(), "100.000010".to_owned()),
                    thread(false, None),
                ),
            ]),
            calls: RefCell::new(Vec::new()),
        };
        let items = vec![
            item("C1", "100.000002"),
            item("C1", "100.000004"),
            reply("C1", "100.000003", "100.000001"),
            reply("C1", "100.000005", "100.000001"),
            reply("C1", "100.000011", "100.000010"),
            item("C2", "100.000001"),
        ];

        let unread = unread_only(items, &fake);

        let keys: Vec<_> = unread
            .iter()
            .map(|item| (item.channel_id.as_str(), item.ts.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                ("C1", "100.000004"),
                ("C1", "100.000005"),
                ("C2", "100.000001")
            ]
        );
        assert_eq!(
            *fake.calls.borrow(),
            [
                "channel C1",
                "thread C1 100.000001",
                "thread C1 100.000010",
                "channel C2",
            ]
        );
    }

    #[test]
    fn drops_archived_channel_without_asking_threads() {
        let fake = Fake {
            channels: HashMap::from([(
                "C1".to_owned(),
                ChannelState {
                    archived: true,
                    last_read: None,
                },
            )]),
            threads: HashMap::new(),
            calls: RefCell::new(Vec::new()),
        };

        let unread = unread_only(vec![reply("C1", "100.000005", "100.000001")], &fake);

        assert!(unread.is_empty());
        assert_eq!(*fake.calls.borrow(), ["channel C1"]);
    }

    #[test]
    fn keeps_reply_when_thread_state_is_unavailable() {
        let fake = Fake {
            channels: HashMap::from([("C1".to_owned(), channel(None))]),
            threads: HashMap::new(),
            calls: RefCell::new(Vec::new()),
        };

        let unread = unread_only(vec![reply("C1", "100.000005", "100.000001")], &fake);

        assert_eq!(unread.len(), 1);
    }
}
