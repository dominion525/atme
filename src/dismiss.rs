use crate::state::{Cache, Dismissed, Item, SeenKey};

/// permalink が一致する項目を探す。permalink はメッセージごとに違う
pub fn key_for_permalink(cache: &Cache, permalink: &str) -> Option<SeenKey> {
    cache
        .items
        .iter()
        .find(|item| item.permalink == permalink)
        .map(Item::key)
}

pub fn all_keys(cache: &Cache) -> Vec<SeenKey> {
    cache.items.iter().map(Item::key).collect()
}

/// キャッシュに残っている項目の分だけを残す。Slack で既読になって一覧から消えたものの記録は要らない
pub fn prune(dismissed: &Dismissed, cache: &Cache) -> Dismissed {
    cache
        .items
        .iter()
        .map(Item::key)
        .filter(|key| dismissed.contains(key))
        .collect()
}

pub fn visible<'a>(cache: &'a Cache, dismissed: &Dismissed) -> Vec<&'a Item> {
    cache
        .items
        .iter()
        .filter(|item| !dismissed.contains(&item.key()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::cache;

    #[test]
    fn finds_key_by_permalink() {
        let cache = cache();

        assert_eq!(
            key_for_permalink(&cache, &cache.items[1].permalink),
            Some(cache.items[1].key())
        );
        assert_eq!(key_for_permalink(&cache, "https://example.com/other"), None);
    }

    #[test]
    fn hides_dismissed_items() {
        let cache = cache();
        let dismissed: Dismissed = [cache.items[0].key()].into();

        let shown = visible(&cache, &dismissed);

        assert_eq!(shown, [&cache.items[1]]);
    }

    #[test]
    fn dismissing_all_hides_everything() {
        let cache = cache();
        let dismissed: Dismissed = all_keys(&cache).into_iter().collect();

        assert!(visible(&cache, &dismissed).is_empty());
    }

    #[test]
    fn prunes_keys_no_longer_cached() {
        let cache = cache();
        let gone = SeenKey {
            workspace: "home".into(),
            channel: "C9".into(),
            ts: "1.0".into(),
        };
        let dismissed: Dismissed = [cache.items[0].key(), gone].into();

        assert_eq!(prune(&dismissed, &cache), [cache.items[0].key()].into());
    }
}
