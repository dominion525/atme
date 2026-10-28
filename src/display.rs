use std::collections::BTreeMap;

use crate::state::Item;

pub fn channel_label(item: &Item) -> String {
    if item.is_im {
        format!(
            "DM {}",
            item.peer_name.as_deref().unwrap_or(&item.channel_name)
        )
    } else if item.is_mpim {
        "グループ DM".into()
    } else {
        format!("#{}", item.channel_name)
    }
}

pub fn summary(text: &str, names: &BTreeMap<String, String>, max_chars: usize) -> String {
    let text = render(text, names)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        "（本文なし）".into()
    } else if text.chars().count() > max_chars {
        text.chars().take(max_chars).chain(['…']).collect()
    } else {
        text
    }
}

fn render(text: &str, names: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        out.push_str(&render_token(&after[..end], names));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn render_token(token: &str, names: &BTreeMap<String, String>) -> String {
    let (target, label) = match token.split_once('|') {
        Some((target, label)) => (target, Some(label)),
        None => (token, None),
    };
    if let Some(id) = target.strip_prefix('@') {
        let name = names.get(id).map(String::as_str).or(label).unwrap_or(id);
        return format!("@{name}");
    }
    if let Some(id) = target.strip_prefix('#') {
        return format!("#{}", label.unwrap_or(id));
    }
    if let Some(special) = target.strip_prefix('!') {
        return match (special, label) {
            ("here" | "channel" | "everyone", _) => format!("@{special}"),
            (_, Some(label)) => label.to_owned(),
            (_, None) => format!("@{}", special.split('^').next().unwrap_or(special)),
        };
    }
    label.unwrap_or(target).to_owned()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn item(channel: &str, ts: &str) -> Item {
        Item {
            workspace: "home".into(),
            channel_id: channel.into(),
            channel_name: "general".into(),
            is_im: false,
            is_mpim: false,
            peer_name: None,
            user: Some("U2".into()),
            user_name: Some("相手".into()),
            ts: ts.into(),
            text: "本文".into(),
            permalink: format!("https://example.slack.com/archives/{channel}/p{ts}"),
        }
    }

    #[test]
    fn renders_slack_markup() {
        let names = BTreeMap::from([("U1".to_owned(), "自分".to_owned())]);
        assert_eq!(
            render(
                "<@U1> <@U9> <!here> <!channel> <!subteam^S1|@dev> <#C1|general> <https://a.example|リンク> <https://b.example> a &lt;b&gt; &amp; c",
                &names
            ),
            "@自分 @U9 @here @channel @dev #general リンク https://b.example a <b> & c"
        );
        assert_eq!(render("閉じない <@U1", &names), "閉じない <@U1");
    }

    #[test]
    fn summary_is_one_line_and_truncated() {
        let names = BTreeMap::new();
        assert_eq!(
            summary("一行目\n\n二行目  三", &names, 10),
            "一行目 二行目 三"
        );
        assert_eq!(summary("あいうえお", &names, 3), "あいう…");
        assert_eq!(summary("あいう", &names, 3), "あいう");
        assert_eq!(summary("", &names, 3), "（本文なし）");
    }

    #[test]
    fn labels_channels() {
        let mut dm = item("D1", "1.0");
        dm.is_im = true;
        dm.channel_name = "U2".into();
        assert_eq!(channel_label(&dm), "DM U2");
        dm.peer_name = Some("相手".into());
        assert_eq!(channel_label(&dm), "DM 相手");

        let mut group = item("G1", "1.0");
        group.is_mpim = true;
        assert_eq!(channel_label(&group), "グループ DM");

        assert_eq!(channel_label(&item("C1", "1.0")), "#general");
    }
}
