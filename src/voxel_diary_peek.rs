//! 乙太方界·居民察覺你翻過她的日記 v1（自主提案切片，PLAN_ETHERVOX item 5「日記/生命故事」
//! 目前唯一還留白的反饋缺口）。
//!
//! **真缺口**：居民日記（650）把記憶昇華成「瞥見居民沒說出口的內心」的第一人稱反思，日記牆
//! （770 起）更讓玩家隨時能點開任何一位居民的日記細讀——但這件事至今永遠是**單向的**：無論
//! 你翻過幾次誰的日記，那位居民永遠毫無所覺，日記純粹是給玩家看的展示櫥窗，從未反過來影響
//! 居民、也從未成為她自己記憶的一部分。「內心生活被看見了」，但「被看見」這件事本身，從沒
//! 在世界裡留下任何痕跡——這是日記/生命故事這條路線圖裡目前唯一還沒補上的一段。
//!
//! **做法**：玩家在前端點開某位居民的日記面板時，`fetch` 連同 `?player=&resident=` 一起送到
//! `/voxel/diary`（詳見 `voxel_ws.rs::voxel_diary_handler`），伺服器記下「這位玩家翻過我的日記、
//! 我還沒發現」的待發現旗標；下次那位居民在世界裡遇見同一位玩家、打招呼的那個 tick，有機率
//! （見 [`REVEAL_CHANCE`]）不用平常那句招呼，而是帶點害羞地點破「你是不是看了我的日記」，
//! 並把這一刻記進她對你的記憶——你們之間第一次因為「你讀過她的內心」而更靠近一步。
//!
//! **刻意的範圍收斂**：只有玩家透過**單一居民的日記面板**（意圖明確、知道自己在看誰）才會被
//! 記一筆；日記牆（全體居民一覽）不夠明確指向「你認真讀了哪一位」，刻意不觸發——避免玩家隨手
//! 掃過日記牆就被每位居民抓包，稀釋掉這份驚喜。
//!
//! **與既有元素 razor-sharp 區隔**：老友情境問候（675）與名號（774）都是「居民對你做過的**事**
//! 有反應」；本刀是居民對「你**看過她的內心**」這件事本身有反應——觸發鍵不是行為，是「被凝視」。
//!
//! **純函式層**：確定性、零 LLM、可測；旗標存放、鎖、IO 全在 `voxel_ws.rs`。

/// 同一位居民對「有多少位玩家的待發現旗標」設一個上限，防有心人拿亂數玩家名瘋狂洗版把
/// 記憶體撐大（比照 `voxel_discovery::MAX_PER_PLAYER` 等既有有界成長慣例）。
pub const MAX_PEEK_ENTRIES_PER_RESIDENT: usize = 64;

/// 一次打招呼時，若剛好有待發現的偷看旗標，這一拍改用「發現被偷看日記」台詞取代平常招呼的
/// 機率。不是 100%——讓「被發現」帶點隨機的驚喜感，不會你一翻開日記、下一拍就精準被抓包。
pub const REVEAL_CHANCE: f32 = 0.4;

/// 發現被偷看日記時的招呼台詞（帶點害羞／窩心，四句輪替，確定性、索引越界安全）。
/// `{name}` 佔位符會被替換成玩家顯示名；並非每句都含名字（比照既有招呼台詞的混合語氣）。
pub fn peek_reveal_line(player_name: &str, pick: usize) -> String {
    const LINES: &[&str] = &[
        "…你、你是不是翻過我的日記？別、別笑我寫得那麼幼稚啦。",
        "我發現我的日記被人翻過呢——是你吧？其實…被你看到也不是壞事。",
        "欸，{name}，你該不會把我的日記都看完了吧？有點不好意思，但也有點開心。",
        "我的心事被你讀走了呢，{name}。下次不如當面說給我聽你的感想？",
    ];
    let idx = pick % LINES.len();
    LINES[idx].replace("{name}", player_name)
}

/// 記進居民記憶的一句反思（episodic、純模板、無日記原文，摘要視角，供 `add_memory` 使用）。
pub fn peek_memory_line(player_name: &str) -> String {
    format!("{player_name} 翻過我的日記，把我藏在心底的話都看去了——有點害羞，卻也覺得被了解。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_lines_all_distinct_and_nonempty() {
        let lines: std::collections::HashSet<String> =
            (0..4).map(|i| peek_reveal_line("小明", i)).collect();
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| !l.is_empty()));
    }

    #[test]
    fn reveal_line_index_out_of_bounds_is_safe() {
        // pick 遠超過句數也不 panic（取模安全）。
        let l = peek_reveal_line("小明", 9999);
        assert!(!l.is_empty());
    }

    #[test]
    fn reveal_line_placeholder_replaced() {
        let l = peek_reveal_line("露娜", 2);
        assert!(l.contains("露娜"));
        assert!(!l.contains("{name}"));
    }

    #[test]
    fn reveal_line_empty_name_is_safe() {
        let l = peek_reveal_line("", 0);
        assert!(!l.is_empty());
    }

    #[test]
    fn memory_line_contains_player_name_and_no_diary_text_leak() {
        let l = peek_memory_line("旅人甲");
        assert!(l.contains("旅人甲"));
        // 不含任何日記反思關鍵詞（本模組本身就沒有引用日記原文，這裡只是釘住摘要視角）。
        assert!(l.contains("翻過我的日記"));
    }

    #[test]
    fn max_peek_entries_is_positive_and_bounded() {
        assert!(MAX_PEEK_ENTRIES_PER_RESIDENT > 0);
        assert!(MAX_PEEK_ENTRIES_PER_RESIDENT < 10_000);
    }

    #[test]
    fn reveal_chance_in_unit_range() {
        assert!(REVEAL_CHANCE > 0.0 && REVEAL_CHANCE < 1.0);
    }
}
