//! 乙太方界·久別重逢摘要 v1——玩家重新連線時，把離線期間發生的重要事件（蓋家完工、
//! 居民情誼升級、心願成真、技能傳授）彙整成一句「你不在的這段時間…」私訊，第一次讓
//! 玩家感受到「世界在我不在時真的繼續活著」，而不是回來後一片死寂、只能靠自己重新找話題。
//!
//! 設計要點：
//! - **白名單過濾**：只挑「值得播報」的事件種類，濾掉高頻噪音（採集/整地/閒聊/新心願/
//!   念頭種下/睡覺/脫困/跑腿交付），避免摘要落落長、稀釋掉真正重要的事。
//! - **離線間隔閘**：太短（如短暫斷線重連）不顯示，避免疲勞轟炸。
//! - **純記憶體 last_seen**（比照 pending_trades／clique 慣例）：重啟後首次連線不顯示
//!   （沒有基準點），之後正常累積、零 migration。

use crate::voxel_feed::FeedEvent;

/// 離線多久以上才值得跳出摘要（秒）——避免短暫斷線重連狂發訊息。
pub const WELCOME_BACK_MIN_GAP_SECS: u64 = 180;
/// 摘要最多列幾件事，避免落落長。
pub const MAX_DIGEST_ITEMS: usize = 3;

/// 值得寫進「久別重逢」摘要的 Feed 事件種類白名單。
const NOTABLE_KINDS: &[&str] =
    &["蓋家完工", "蓋家擴建完工", "居民情誼", "技能傳授", "心願成真", "居民想念"];

/// 這個 Feed 事件種類值不值得放進摘要。
pub fn is_notable(kind: &str) -> bool {
    NOTABLE_KINDS.contains(&kind)
}

/// 是否該顯示久別重逢摘要：從沒見過這個名字（`None`）→ 不顯示（避免首次登入就跳訊息、
/// 也避免伺服器剛重啟時人人都跳一次）；間隔太短 → 不顯示；否則 → 顯示。
pub fn should_show_welcome(last_seen: Option<u64>, now: u64) -> bool {
    match last_seen {
        None => false,
        Some(last) => now.saturating_sub(last) >= WELCOME_BACK_MIN_GAP_SECS,
    }
}

/// 把一則 Feed 事件格式化成一句摘要行（面向玩家字串，留 i18n 空間）。
/// 「居民情誼」「技能傳授」的 `detail` 已是完整句子，直接使用；其餘種類補上主角名字。
fn digest_line(ev: &FeedEvent) -> String {
    match ev.kind.as_str() {
        "蓋家完工" => format!("{}蓋好了{}", ev.resident, ev.detail),
        "蓋家擴建完工" => format!("{}把{}擴建了", ev.resident, ev.detail),
        "居民情誼" | "技能傳授" => ev.detail.clone(),
        "心願成真" => format!("{}{}", ev.resident, ev.detail),
        _ => format!("{}{}", ev.resident, ev.detail),
    }
}

/// 從最近事件（假定已依時間新到舊排序，如 `voxel_feed::load_recent_feed`）中挑出
/// `since` 之後、屬於白名單的事件，取最新的 `MAX_DIGEST_ITEMS` 筆格式化成摘要行。
pub fn summarize_events(events: &[FeedEvent], since: u64) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.ts_secs > since && is_notable(&e.kind))
        .take(MAX_DIGEST_ITEMS)
        .map(digest_line)
        .collect()
}

/// 把摘要行組成一則私訊；沒有值得說的事就回 `None`（不顯示空摘要）。
pub fn format_welcome_message(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    Some(format!("🌙 你不在的這段時間：{}", lines.join("；")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts: u64, kind: &str, resident: &str, detail: &str) -> FeedEvent {
        FeedEvent {
            ts_secs: ts,
            kind: kind.to_string(),
            resident: resident.to_string(),
            detail: detail.to_string(),
        }
    }

    // is_notable

    #[test]
    fn notable_kinds_true() {
        for k in ["蓋家完工", "蓋家擴建完工", "居民情誼", "技能傳授", "心願成真"] {
            assert!(is_notable(k), "{k} 應為白名單");
        }
    }

    #[test]
    fn noisy_kinds_false() {
        for k in ["採集", "整地", "鄰里閒聊", "新心願", "念頭種下", "睡覺", "脫困", "跑腿交付", ""] {
            assert!(!is_notable(k), "{k} 不應為白名單");
        }
    }

    // should_show_welcome

    #[test]
    fn no_prior_visit_never_shows() {
        assert!(!should_show_welcome(None, 10_000));
    }

    #[test]
    fn gap_too_short_does_not_show() {
        assert!(!should_show_welcome(Some(1000), 1000 + WELCOME_BACK_MIN_GAP_SECS - 1));
    }

    #[test]
    fn gap_exactly_threshold_shows() {
        assert!(should_show_welcome(Some(1000), 1000 + WELCOME_BACK_MIN_GAP_SECS));
    }

    #[test]
    fn gap_long_shows() {
        assert!(should_show_welcome(Some(1000), 1000 + 86_400));
    }

    // digest_line via summarize_events

    #[test]
    fn build_complete_line_reads_naturally() {
        let lines = summarize_events(&[ev(500, "蓋家完工", "露娜", "小屋")], 0);
        assert_eq!(lines, vec!["露娜蓋好了小屋".to_string()]);
    }

    #[test]
    fn build_expansion_line_reads_naturally() {
        let lines = summarize_events(&[ev(500, "蓋家擴建完工", "諾娃", "小屋")], 0);
        assert_eq!(lines, vec!["諾娃把小屋擴建了".to_string()]);
    }

    #[test]
    fn bond_and_teach_lines_use_detail_verbatim() {
        let bond = summarize_events(&[ev(500, "居民情誼", "露娜", "🤝 露娜 和 諾娃 成了老朋友！")], 0);
        assert_eq!(bond, vec!["🤝 露娜 和 諾娃 成了老朋友！".to_string()]);
        let teach = summarize_events(&[ev(500, "技能傳授", "露娜", "露娜把自己發明的「燒玻璃」教給了諾娃！")], 0);
        assert_eq!(teach, vec!["露娜把自己發明的「燒玻璃」教給了諾娃！".to_string()]);
    }

    #[test]
    fn wish_come_true_line_prefixes_resident() {
        let lines = summarize_events(&[ev(500, "心願成真", "諾娃", "因為旅人的話，蓋好了小屋")], 0);
        assert_eq!(lines, vec!["諾娃因為旅人的話，蓋好了小屋".to_string()]);
    }

    #[test]
    fn filters_out_noisy_and_stale_events() {
        let events = vec![
            ev(500, "採集", "露娜", "採集了木頭"),
            ev(200, "蓋家完工", "露娜", "小屋"), // 早於 since，該被濾掉
            ev(600, "蓋家完工", "諾娃", "水井"),
        ];
        let lines = summarize_events(&events, 300);
        assert_eq!(lines, vec!["諾娃蓋好了水井".to_string()]);
    }

    #[test]
    fn caps_at_max_digest_items_keeps_newest_first() {
        let events = vec![
            ev(400, "蓋家完工", "露娜", "小屋"),
            ev(300, "蓋家完工", "諾娃", "水井"),
            ev(200, "技能傳授", "賽勒", "教了奧瑞一招"),
            ev(100, "心願成真", "奧瑞", "因為旅人的話，蓋好了瞭望台"),
        ];
        let lines = summarize_events(&events, 0);
        assert_eq!(lines.len(), MAX_DIGEST_ITEMS);
        assert_eq!(lines[0], "露娜蓋好了小屋".to_string());
        assert_eq!(lines[1], "諾娃蓋好了水井".to_string());
    }

    #[test]
    fn no_notable_events_returns_empty() {
        let events = vec![ev(500, "採集", "露娜", "採集了木頭")];
        assert!(summarize_events(&events, 0).is_empty());
    }

    // format_welcome_message

    #[test]
    fn empty_lines_yield_none() {
        assert_eq!(format_welcome_message(&[]), None);
    }

    #[test]
    fn non_empty_lines_join_with_prefix() {
        let msg = format_welcome_message(&["露娜蓋好了小屋".to_string(), "諾娃蓋好了水井".to_string()]);
        assert_eq!(msg, Some("🌙 你不在的這段時間：露娜蓋好了小屋；諾娃蓋好了水井".to_string()));
    }
}
