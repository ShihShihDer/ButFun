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

/// 方塊 id → 中文物品名（對齊 `voxel::Block` + `voxel_farm` 純物品 id）。
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
        18 => "小麥",
        19 => "麵包",
        20 => "煤礦",
        21 => "鐵礦",
        22 => "鐵錠",
        23 => "鐵磚",
        31 => "火把",
        32 => "木鎬",
        33 => "石鎬",
        34 => "鐵鎬",
        35 => "梯子",
        36 => "木斧",
        37 => "石斧",
        38 => "鐵斧",
        _ => "物品",
    }
}

/// 是否為「食物」類禮物（麵包）——居民會給特別溫暖的回應。
pub fn is_food_gift(block_id: u8) -> bool {
    block_id == 19 // BREAD_ID
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

/// 食物禮物（麵包）的居民道謝台詞——比一般禮物更歡欣。
/// 呼叫時機：`is_food_gift(item_id) == true`。
/// `pick` 同 `gift_thanks_line`：由呼叫端提供，確定性不走 random。
pub fn food_gift_thanks_line(player_name: &str, affinity: usize, pick: usize) -> String {
    if affinity == 0 || player_name.is_empty() {
        let pool: &[&str] = &[
            "哦……麵包？你親手做的嗎？謝謝你！",
            "哇，麵包！你怎麼知道我最喜歡吃麵包了！",
            "麵包耶！謝謝你帶來這麼用心的禮物。",
        ];
        pool[pick % pool.len()].to_string()
    } else if affinity <= 2 {
        let pool: &[&str] = &[
            "{name}，你帶麵包來給我！謝謝你這麼用心。",
            "{name}！自己做的麵包！聞起來好香，謝謝你。",
            "哇，{name}你烤了麵包！我好感動，謝謝。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    } else {
        let pool: &[&str] = &[
            "{name}，你知道我喜歡吃東西對吧……這塊麵包我要慢慢品嚐。",
            "每次{name}來都帶著驚喜——這次是麵包！謝謝你記得我。",
            "{name}！你親手做的麵包……我真的很珍惜和你在一起的每一刻。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    }
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
        assert_eq!(item_name_zh(20), "煤礦");
        assert_eq!(item_name_zh(21), "鐵礦");
        assert_eq!(item_name_zh(22), "鐵錠");
        assert_eq!(item_name_zh(23), "鐵磚");
        assert_eq!(item_name_zh(31), "火把");
        assert_eq!(item_name_zh(32), "木鎬");
        assert_eq!(item_name_zh(34), "鐵鎬");
        assert_eq!(item_name_zh(35), "梯子");
        assert_eq!(item_name_zh(36), "木斧");
        assert_eq!(item_name_zh(37), "石斧");
        assert_eq!(item_name_zh(38), "鐵斧");
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

    // ── 麵包 v1（ROADMAP 668）──────────────────────────────────────────────────
    #[test]
    fn item_name_wheat_and_bread() {
        assert_eq!(item_name_zh(18), "小麥");
        assert_eq!(item_name_zh(19), "麵包");
    }

    #[test]
    fn is_food_gift_only_bread() {
        assert!(is_food_gift(19));
        assert!(!is_food_gift(18)); // 小麥顆粒不算食物禮物
        assert!(!is_food_gift(5));  // 木頭非食物
        assert!(!is_food_gift(0));  // Air 非食物
    }

    #[test]
    fn food_gift_thanks_non_empty_no_placeholders() {
        // 所有好感等級、多個 pick 值，不得有未替換的 {name}/{item}。
        for affinity in [0, 1, 2, 3, 5] {
            for pick in 0..4 {
                let s = food_gift_thanks_line("旅人", affinity, pick);
                assert!(!s.is_empty(), "affinity={affinity} pick={pick} 回空");
                assert!(!s.contains("{name}"), "affinity={affinity} pick={pick} 未替換 name");
                assert!(!s.contains("{item}"), "affinity={affinity} pick={pick} 出現 item 佔位");
            }
        }
    }

    #[test]
    fn food_gift_thanks_friend_contains_name() {
        let s = food_gift_thanks_line("小星", 5, 0);
        assert!(s.contains("小星"), "友人等級應含玩家名");
    }
}
