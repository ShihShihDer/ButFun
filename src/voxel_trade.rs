//! 乙太方界·居民以物易物 v1——玩家向居民請求交易，居民提出以物換物條件（ROADMAP 670）。
//!
//! 純邏輯（交易提案生成、居民台詞、記憶摘要），無 WS / 鎖 / IO 細節。
//! 由 `voxel_ws.rs` 包進鎖後呼叫；確定性、可測、零 LLM。
//!
//! **交易流程**：
//! 1. 玩家點「⇌ 交易」→ TradeRequest → 伺服器生成提案 → 回 `trade_offer`。
//! 2. 玩家在 30 秒內點「接受」→ TradeAccept → 伺服器執行扣/給背包 → 回 `trade_done`。
//!
//! **各居民特長**（依 resident_id 字元和 % 4 決定，永遠確定性）：
//! - slot 0 → 種子 ↔ 木頭（農業，對應露娜）
//! - slot 1 → 石頭 ↔ 木頭（建築，對應諾娃）
//! - slot 2 → 木頭 ↔ 沙子（探索，對應賽勒）
//! - slot 3 → 玻璃 ↔ 石頭（煉製，對應奧瑞）
//!
//! **好感度影響比例**：
//! - 0（陌生）：玩家給 2 得 1（略不划算，反映居民不信任）
//! - 1–2（相識）：1:1 公平
//! - 3+（友人）：玩家給 1 得 2（划算，居民優待朋友）

/// 交易觸及範圍（方塊距離，水平 XZ 平面，與 GIFT_REACH 一致）。
pub const TRADE_REACH: f32 = 5.0;

/// 交易提案有效時間（秒）：玩家 30 秒內未接受則伺服器自動作廢。
pub const TRADE_OFFER_TTL: u64 = 30;

/// 居民的交易提案：居民提供的物品 ↔ 玩家需要提供的物品。
#[derive(Clone, Debug)]
pub struct TradeOffer {
    /// 居民提供的物品 id。
    pub offer_item: u8,
    /// 居民提供的數量。
    pub offer_count: u32,
    /// 居民想要的物品 id（玩家需給出）。
    pub want_item: u8,
    /// 居民想要的數量（玩家需給出）。
    pub want_count: u32,
}

/// 依 resident_id 決定交易特長 slot（0..4，永遠確定性）。
pub fn resident_trade_slot(resident_id: &str) -> usize {
    let sum: u64 = resident_id.bytes().map(|b| b as u64).sum();
    (sum % 4) as usize
}

/// 根據居民 ID 與玩家好感度生成交易提案（確定性純函式）。
pub fn make_offer(resident_id: &str, affinity: usize) -> TradeOffer {
    let slot = resident_trade_slot(resident_id);
    // (offer_item, want_item)：居民提供 / 玩家給出
    let (offer_item, want_item): (u8, u8) = match slot {
        0 => (14, 5),  // 種子 ↔ 木頭
        1 => (3, 5),   // 石頭 ↔ 木頭
        2 => (5, 4),   // 木頭 ↔ 沙子
        _ => (10, 3),  // 玻璃 ↔ 石頭
    };
    let (offer_count, want_count): (u32, u32) = if affinity == 0 {
        (1, 2) // 陌生人：玩家給 2 得 1
    } else if affinity <= 2 {
        (1, 1) // 相識：公平 1:1
    } else {
        (2, 1) // 友人：玩家給 1 得 2
    };
    TradeOffer { offer_item, offer_count, want_item, want_count }
}

/// 方塊 / 物品 id → 中文名（對齊 voxel_gift::item_name_zh，獨立維護讓模組自給自足）。
pub fn item_name_zh(block_id: u8) -> &'static str {
    match block_id {
        1 => "草",
        2 => "泥土",
        3 => "石頭",
        4 => "沙子",
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
        _ => "物品",
    }
}

/// 居民提出交易時冒出的台詞（確定性純函式，零 LLM）。
/// 依 offer_count vs want_count 選不同語氣（公平 / 划算 / 不划算）。
pub fn offer_say_line(offer: &TradeOffer) -> String {
    let oname = item_name_zh(offer.offer_item);
    let wname = item_name_zh(offer.want_item);
    if offer.offer_count == offer.want_count {
        format!("我這兒有{}，要不要換你的{}？1:1 公平！", oname, wname)
    } else if offer.offer_count > offer.want_count {
        // 居民提供更多（友人優待）：強調划算
        format!("給你{}個{}，換你{}個{}——你是我的朋友，划得來！",
            offer.offer_count, oname, offer.want_count, wname)
    } else {
        // 居民提供更少（陌生人不信任）：坦白說明條件
        format!("你給我{}個{}，我給你{}個{}，怎麼樣？",
            offer.want_count, wname, offer.offer_count, oname)
    }
}

/// 交易成功後居民說的話（確定性純函式）。
pub fn done_say_line(player_name: &str, got_name: &str) -> String {
    if player_name.is_empty() {
        format!("成交！{}給你了。", got_name)
    } else {
        format!("{}，成交！{}給你了，謝謝你。", player_name, got_name)
    }
}

/// 寫進居民記憶的摘要（1 筆，確定性純函式）。
pub fn trade_memory(player_name: &str, gave_name: &str, got_name: &str) -> String {
    format!("和{}以物易物：我給了{}，換來了{}，感覺不錯", player_name, gave_name, got_name)
}

// ── 測試 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_trade_slot_in_range() {
        for id in ["luna", "nova", "sailer", "auri", "", "abc123", "居民露娜"] {
            let slot = resident_trade_slot(id);
            assert!(slot < 4, "slot={slot} 超出 0..4 (id={id})");
        }
    }

    #[test]
    fn resident_trade_slot_deterministic() {
        let s1 = resident_trade_slot("居民露娜-001");
        let s2 = resident_trade_slot("居民露娜-001");
        assert_eq!(s1, s2, "相同 id 應得相同 slot");
    }

    #[test]
    fn make_offer_stranger_unfavorable() {
        // 陌生人（affinity=0）：玩家給更多才能得到居民的物品
        for id in ["luna", "nova", "sailer", "auri"] {
            let offer = make_offer(id, 0);
            assert!(offer.want_count > offer.offer_count,
                "陌生人交易應不划算 id={id}");
        }
    }

    #[test]
    fn make_offer_acquaintance_fair() {
        // 相識（1–2）：1:1 公平
        for id in ["luna", "nova", "sailer", "auri"] {
            for affinity in [1usize, 2] {
                let offer = make_offer(id, affinity);
                assert_eq!(offer.offer_count, offer.want_count,
                    "相識應 1:1 id={id} affinity={affinity}");
            }
        }
    }

    #[test]
    fn make_offer_friend_favorable() {
        // 友人（3+）：玩家給更少、得到更多
        for id in ["luna", "nova", "sailer", "auri"] {
            for affinity in [3usize, 5, 10] {
                let offer = make_offer(id, affinity);
                assert!(offer.offer_count > offer.want_count,
                    "友人交易應划算 id={id} affinity={affinity}");
            }
        }
    }

    #[test]
    fn make_offer_items_nonzero_and_different() {
        let offer = make_offer("test-resident", 1);
        assert!(offer.offer_item > 0, "offer_item 應非 0（不是 Air）");
        assert!(offer.want_item > 0, "want_item 應非 0（不是 Air）");
        assert_ne!(offer.offer_item, offer.want_item,
            "提供與需求物品不應相同（同一物品沒意義）");
    }

    #[test]
    fn make_offer_counts_positive() {
        for affinity in [0usize, 1, 2, 3, 10] {
            let offer = make_offer("resident-x", affinity);
            assert!(offer.offer_count > 0, "affinity={affinity} offer_count 應>0");
            assert!(offer.want_count > 0, "affinity={affinity} want_count 應>0");
        }
    }

    #[test]
    fn offer_say_line_non_empty_no_braces() {
        for affinity in [0, 1, 2, 3] {
            let offer = make_offer("resident-y", affinity);
            let s = offer_say_line(&offer);
            assert!(!s.is_empty(), "affinity={affinity} 台詞不得空");
            assert!(!s.contains('{'), "affinity={affinity} 台詞含未替換佔位");
        }
    }

    #[test]
    fn offer_say_line_fair_contains_item_names() {
        // 公平 1:1：台詞應提到兩種物品名
        let offer = TradeOffer { offer_item: 5, offer_count: 1, want_item: 4, want_count: 1 };
        let s = offer_say_line(&offer);
        assert!(s.contains("木頭"), "公平台詞應含 offer_item 名（木頭）");
        assert!(s.contains("沙子"), "公平台詞應含 want_item 名（沙子）");
    }

    #[test]
    fn done_say_line_non_empty() {
        assert!(!done_say_line("旅人", "木頭").is_empty());
        assert!(!done_say_line("", "石頭").is_empty());
    }

    #[test]
    fn done_say_line_with_name_contains_name() {
        let s = done_say_line("小明", "玻璃");
        assert!(s.contains("小明"), "含玩家名的成交台詞應包含玩家名");
    }

    #[test]
    fn trade_memory_contains_all_parts() {
        let s = trade_memory("小美", "種子", "木頭");
        assert!(s.contains("小美"), "記憶應含玩家名");
        assert!(s.contains("種子"), "記憶應含給出物品名");
        assert!(s.contains("木頭"), "記憶應含換來物品名");
    }

    #[test]
    fn item_name_zh_known_ids() {
        assert_eq!(item_name_zh(3), "石頭");
        assert_eq!(item_name_zh(4), "沙子");
        assert_eq!(item_name_zh(5), "木頭");
        assert_eq!(item_name_zh(10), "玻璃");
        assert_eq!(item_name_zh(14), "種子");
    }

    #[test]
    fn item_name_zh_unknown_fallback() {
        assert_eq!(item_name_zh(200), "物品");
        assert_eq!(item_name_zh(0), "物品"); // Air 不交易
    }

    #[test]
    fn constants_sane() {
        assert!(TRADE_REACH > 0.0, "TRADE_REACH 應大於 0");
        assert!(TRADE_OFFER_TTL > 0, "TRADE_OFFER_TTL 應大於 0");
    }
}
