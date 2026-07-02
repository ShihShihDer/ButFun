//! 乙太方界·你送的心意，她擺了出來 v1（ROADMAP 732）。
//!
//! **核心信念**：「你的互動有後果」。至今玩家送居民的禮物（660）只化成一句記憶，
//! 世界裡不留任何痕跡；本模組讓**可展示的餽贈真的被居民擺進世界**——她把你送的
//! 木頭/火把/冰晶燈當作紀念物放在腳邊，你日後路過還看得到。記憶不只驅動聊天，
//! 也讓「改變世界佈局」這件事**第一次因玩家的善意而發生**（記憶→行為，且後果永久可見）。
//!
//! **與既有系統的分界**：這不是「居民互訪敘事分支」（gossip 694／互助蓋家 696／
//! 拌嘴 715／傳授 717／易物 723 那一整排），而是**玩家↔居民、且真的動了世界方塊**的
//! 全新後果——別的系統至今沒有任何一條會因為「玩家送禮」而放下一塊真方塊。
//!
//! **純邏輯層**：`keepsake_block`（禮物→紀念物白名單映射）、`keepsake_name`、
//! 記憶/Feed 台詞皆為確定性純函式，零 LLM、零鎖、零 IO、可單元測試。
//! 找空位 / 放方塊 / 廣播 / 持久化全在 `voxel_ws.rs`（沿用居民放置的短鎖循序慣例）。

/// Feed 事件類型字串（面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "心意留痕";

/// 紀念物台詞字元上限（泡泡/日記可讀，與其他社交台詞一致）。
const KEEPSAKE_MAX_CHARS: usize = 40;

/// 禮物→紀念物方塊映射（確定性白名單）。
///
/// 只有「擺出來好看、陳列得起來」的自然／工藝方塊才成為紀念物：木頭、石頭、木板、
/// 石磚、玻璃、火把、仙人掌、雪、冰晶、冰晶燈。工具／食物／種子／礦石不列入——
/// 那些回一般道謝即可，硬擺一塊反而突兀。回傳的 id 都落在 `Block::is_placeable`
/// 白名單內（呼叫端會 `Block::from_u8` 再放置，安全）。
///
/// 刻意「送什麼擺什麼」（同 id 對映）：讓玩家一眼認出「這就是我剛送她的東西」，
/// 紀念的分量才真實；不做花俏轉換（如種子→花），v1 先把那個「哇」的瞬間做出來。
pub fn keepsake_block(gift_item: u8) -> Option<u8> {
    match gift_item {
        5 => Some(5),   // 木頭
        3 => Some(3),   // 石頭
        8 => Some(8),   // 木板
        9 => Some(9),   // 石磚
        10 => Some(10), // 玻璃
        31 => Some(31), // 火把（插在門口，夜裡亮著）
        54 => Some(54), // 仙人掌
        55 => Some(55), // 雪
        56 => Some(56), // 冰晶
        57 => Some(57), // 冰晶燈
        _ => None,
    }
}

/// 紀念物名（面向玩家顯示，集中可 i18n）。與贈禮系統的物名保持一致措辭。
pub fn keepsake_name(block: u8) -> &'static str {
    match block {
        5 => "木頭",
        3 => "石頭",
        8 => "木板",
        9 => "石磚",
        10 => "玻璃",
        31 => "火把",
        54 => "仙人掌",
        55 => "雪",
        56 => "冰晶",
        57 => "冰晶燈",
        _ => "小禮",
    }
}

/// 截斷輔助：保留至多 [`KEEPSAKE_MAX_CHARS`] 個字元（依字元非位元組，繁中安全）。
fn clip(s: String) -> String {
    s.chars().take(KEEPSAKE_MAX_CHARS).collect()
}

/// 居民把玩家的禮物擺出來時，記進**自己**記憶的一句——點名送禮者，
/// 讓這份心意日後能被回想、被日記（650）昇華成「我把旅人送的東西擺在門口」。
/// 依 `pick` 確定性選一種措辭（零 LLM、可測），保有變化不機械。
pub fn keepsake_memory_line(giver: &str, item_name: &str, pick: usize) -> String {
    const TEMPLATES: &[&str] = &[
        "我把{giver}送的{item}擺在家門口，每次看到就想起他。",
        "{giver}送我的{item}，我好好收下、放在腳邊當作紀念。",
        "這份{item}是{giver}的心意，我把它留在身邊了。",
    ];
    let t = TEMPLATES[pick % TEMPLATES.len()];
    clip(t.replace("{giver}", giver).replace("{item}", item_name))
}

/// Feed 事件字串——讓玩家回來翻動態時，看到「我的禮物在她生活裡留下了痕跡」。
pub fn keepsake_feed_line(resident: &str, giver: &str, item_name: &str) -> String {
    clip(format!("{resident}把{giver}送的{item_name}擺了出來，當作紀念"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 白名單命中送什麼擺什麼() {
        // 可展示的自然／工藝方塊：送什麼擺什麼（同 id）。
        for &id in &[5u8, 3, 8, 9, 10, 31, 54, 55, 56, 57] {
            assert_eq!(keepsake_block(id), Some(id), "block {id} 應成為同款紀念物");
        }
    }

    #[test]
    fn 非展示類禮物不擺紀念物() {
        // 工具／食物／種子／礦石／乙太等不列入——回一般道謝即可。
        for &id in &[14u8, 18, 19, 20, 21, 22, 33, 34, 0, 99] {
            assert_eq!(keepsake_block(id), None, "block {id} 不該產生紀念物");
        }
    }

    #[test]
    fn 紀念物白名單全落在可放置範圍() {
        // 每個會被擺出的 id 都必須是玩家可放置的方塊（呼叫端 set_block 才安全）。
        use crate::voxel::Block;
        for id in 0u8..=60 {
            if let Some(keep) = keepsake_block(id) {
                let b = Block::from_u8(keep).expect("紀念物 id 應能還原成 Block");
                assert!(b.is_placeable(), "紀念物 {keep} 必須可放置");
            }
        }
    }

    #[test]
    fn 紀念物名皆非空且對得上() {
        for &id in &[5u8, 3, 8, 9, 10, 31, 54, 55, 56, 57] {
            assert!(!keepsake_name(id).is_empty());
        }
        assert_eq!(keepsake_name(255), "小禮"); // 未知 id 回退
    }

    #[test]
    fn 記憶句點名送禮者且有變化() {
        let a = keepsake_memory_line("諾亞", "火把", 0);
        let b = keepsake_memory_line("諾亞", "火把", 1);
        assert!(a.contains("諾亞") && a.contains("火把"));
        assert!(b.contains("諾亞") && b.contains("火把"));
        assert_ne!(a, b, "不同 pick 應選到不同措辭");
        assert!(a.chars().count() <= KEEPSAKE_MAX_CHARS);
    }

    #[test]
    fn 記憶句確定性同輸入同輸出() {
        assert_eq!(
            keepsake_memory_line("露娜", "冰晶燈", 7),
            keepsake_memory_line("露娜", "冰晶燈", 7)
        );
    }

    #[test]
    fn feed句含三要素且有界() {
        let f = keepsake_feed_line("露娜", "旅人阿豪", "冰晶燈");
        assert!(f.contains("露娜") && f.contains("旅人阿豪") && f.contains("冰晶燈"));
        assert!(f.chars().count() <= KEEPSAKE_MAX_CHARS);
    }

    #[test]
    fn 超長送禮者名不爆字數() {
        let long = "旅".repeat(50);
        let m = keepsake_memory_line(&long, "木頭", 0);
        assert!(m.chars().count() <= KEEPSAKE_MAX_CHARS);
    }
}
