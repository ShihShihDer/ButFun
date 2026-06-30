//! 乙太方界·居民贈禮 v1——玩家把採來的材料化作一份心意送給居民（ROADMAP 660）。
//!
//! 純邏輯（模板文字、記憶摘要、觸及半徑），無 WS / 鎖 / IO 細節。
//! 由 `voxel_ws.rs` 包進鎖後呼叫；確定性、可測、零 LLM。

/// 贈禮觸及範圍（方塊距離，水平 XZ 平面）。
/// 比挖/放方塊的 REACH（6.0）稍短：需要走近才能遞東西。
pub const GIFT_REACH: f32 = 5.0;

/// 贈禮加入記憶的筆數（代表禮物重量高於一次對話）。
/// 每次送禮新增 2 筆記憶 → 好感度 +2（對話只 +1）。
pub const GIFT_MEMORY_COUNT: usize = 2;

/// 方塊 id → 中文物品名（對齊 `voxel::Block` + `voxel_farm::SEEDS_ID`）。
/// 未知 id 回 "物品" 保守降級。
pub fn item_name_zh(block_id: u8) -> &'static str {
    match block_id {
        1 => "草",
        2 => "泥土",
        3 => "石頭",
        4 => "沙",
        5 => "木頭",
        6 => "葉片",
        7 => "水",
        8 => "木板",
        9 => "石磚",
        10 => "玻璃",
        11 => "農田土",
        12 => "幼苗",
        13 => "成熟小麥",
        14 => "種子",
        _ => "物品",
    }
}

/// 居民道謝台詞（依好感等級選不同句，零 LLM，確定性）。
///
/// - `affinity` 0   → 陌生人：稍微驚訝、客氣致謝
/// - `affinity` 1–2 → 相識：帶玩家名字的親切道謝
/// - `affinity` 3+  → 友人：帶名字、更溫暖、有「一直照顧我」的感受
///
/// `pick` 由呼叫端提供（unix 秒 % 句池長度），在同等級句池內輪替確保確定性（不走 random）。
/// `player_name` 空字串 = 訪客模式，回陌生人句池。
pub fn gift_thanks_line(
    item_name: &str,
    player_name: &str,
    affinity: usize,
    pick: usize,
) -> String {
    if affinity == 0 || player_name.is_empty() {
        let pool: &[&str] = &[
            "哦？送我{item}？謝謝你的心意！",
            "這……{item}？我收下了，感謝你。",
            "謝謝！你送我{item}，我很高興。",
        ];
        pool[pick % pool.len()].replace("{item}", item_name)
    } else if affinity <= 2 {
        let pool: &[&str] = &[
            "{name}，謝謝你帶來{item}！",
            "{name}！這份{item}我很喜歡，謝謝你。",
            "啊，{name}，你送我{item}～我好開心。",
        ];
        pool[pick % pool.len()]
            .replace("{name}", player_name)
            .replace("{item}", item_name)
    } else {
        let pool: &[&str] = &[
            "{name}，你總是這樣照顧我……這份{item}我會好好珍藏。",
            "能有{name}這樣的朋友，我很幸運。謝謝這份{item}。",
            "{name}！每次你來都帶著心意……這份{item}讓我很感動。",
        ];
        pool[pick % pool.len()]
            .replace("{name}", player_name)
            .replace("{item}", item_name)
    }
}

/// 被居民存進記憶的第一筆摘要（「事件」層：記錄送禮這件事）。
pub fn gift_memory_event(player: &str, item_name: &str) -> String {
    format!("收到了{player}送來的{item_name}，心裡暖暖的")
}

/// 被居民存進記憶的第二筆摘要（「感受」層：代表更深的印象，讓好感度多加一層）。
pub fn gift_memory_feeling(player: &str, item_name: &str) -> String {
    format!("{player}送我{item_name}——這個人很體貼")
}

// ── 測試 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_name_known_ids() {
        assert_eq!(item_name_zh(2), "泥土");
        assert_eq!(item_name_zh(3), "石頭");
        assert_eq!(item_name_zh(5), "木頭");
        assert_eq!(item_name_zh(8), "木板");
        assert_eq!(item_name_zh(14), "種子");
    }

    #[test]
    fn item_name_unknown_fallback() {
        assert_eq!(item_name_zh(200), "物品");
        assert_eq!(item_name_zh(0), "物品"); // Air 不送
    }

    #[test]
    fn gift_thanks_stranger_no_name() {
        // affinity=0 或 player_name 空字串 → 陌生人句池，不含玩家名，非空
        let s = gift_thanks_line("木頭", "", 0, 0);
        assert!(!s.is_empty());
        assert!(s.contains("木頭"));
        assert!(!s.contains("{item}"));
    }

    #[test]
    fn gift_thanks_stranger_with_zero_affinity() {
        let s = gift_thanks_line("石頭", "旅人", 0, 1);
        assert!(!s.is_empty());
        assert!(s.contains("石頭"));
        assert!(!s.contains("{item}"));
        assert!(!s.contains("{name}"));
    }

    #[test]
    fn gift_thanks_acquaintance() {
        let s = gift_thanks_line("木板", "小明", 2, 0);
        assert!(s.contains("小明"));
        assert!(s.contains("木板"));
        assert!(!s.contains("{name}"));
        assert!(!s.contains("{item}"));
    }

    #[test]
    fn gift_thanks_friend() {
        let s = gift_thanks_line("玻璃", "阿星", 5, 2);
        assert!(s.contains("阿星"));
        assert!(s.contains("玻璃"));
        assert!(!s.contains("{name}"));
        assert!(!s.contains("{item}"));
    }

    #[test]
    fn gift_thanks_pick_wraps_and_non_empty() {
        // 句池長度 3；pick 超界 → 取模，永遠回非空
        for pick in 0..10 {
            let s = gift_thanks_line("種子", "旅人", 0, pick);
            assert!(!s.is_empty(), "pick={pick} 回空字串");
        }
    }

    #[test]
    fn gift_memory_event_contains_player_and_item() {
        let s = gift_memory_event("小美", "木頭");
        assert!(s.contains("小美"));
        assert!(s.contains("木頭"));
    }

    #[test]
    fn gift_memory_feeling_contains_player_and_item() {
        let s = gift_memory_feeling("阿宏", "玻璃");
        assert!(s.contains("阿宏"));
        assert!(s.contains("玻璃"));
    }

    #[test]
    fn constants_sane() {
        assert!(GIFT_REACH > 0.0);
        assert_eq!(GIFT_MEMORY_COUNT, 2);
    }
}
