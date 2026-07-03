//! 居民互贈·分享採集所得 v1（ROADMAP 748）——老朋友到訪時，主人把自己採集背包裡
//! 真的採到最多、且有餘裕的那種材料，勻一小份給訪客帶回去。
//!
//! **與 723「以物易物」的關鍵區別**：723 是象徵性的（基於「特長分類」的抽象物名、
//! 對稱交換、**不動任何實際背包**，純 Feed+記憶）。本切片是乙太方界第一道**真實物資
//! 流動**——分享的是主人 `res_inv` 裡真的有的東西，且真的把材料從主人背包**移到**訪客
//! 背包（零和守恆、不憑空生料），呼應 728 回禮 v2「反映真實勞動」的精神，但這次流動
//! 發生在居民↔居民之間，餵的是訪客自己的發明／建造計畫——朋友第一次在物質上互相
//! 幫襯彼此的目標，小社會的內部經濟有了第一道真實的血流（PLAN_ETHERVOX item 4）。
//!
//! **成本紀律**：零 LLM（機率＋確定性選句）、零持久化／零 migration（`res_inv` 本就是
//! 純記憶體採集背包，重啟歸零）、零新美術（Feed 沿用既有列，新 kind 是 additive 字串
//! 舊前端安全落回）、FPS 零影響（掛既有到訪時機、稀少、無新尋路）。

use std::collections::HashMap;

use crate::voxel_bonds::BondTier;

/// 觸發分享的機率（老朋友到訪、且這次沒演別齣戲時）。稀少有感，鏡像既有「一訪一戲」的低頻。
pub const SHARE_CHANCE: f32 = 0.12;

/// 主人得握有這麼多某種材料才算「有餘裕」可分享——低於此不掏，
/// 免得打亂自己正在湊料的發明／建造計畫（`voxel_invent` 也吃 `res_inv`）。
pub const SHARE_MIN_STOCK: u32 = 6;

/// 一次最多分享這麼多份（只勻一點點；守恆轉移、不掏空主人）。
pub const SHARE_CAP: u32 = 2;

/// Feed 事件類型字串（面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "居民分享";

/// 台詞字元上限（泡泡／日記可讀，與其他社交台詞一致）。
pub const SHARE_MAX_CHARS: usize = 40;

/// 是否該在這次到訪觸發分享（純函式、確定性）。
/// 只在老朋友、且這次到訪沒觸發互助蓋家／拌嘴／傳授／易物時才可能（同一訪只演一齣戲，
/// 鏡像 `voxel_resident_trade::should_resident_trade` 的優先序慣例）。
pub fn should_share(tier: BondTier, other_scene: bool, roll: f32) -> bool {
    tier == BondTier::Friend && !other_scene && roll < SHARE_CHANCE
}

/// 從主人的採集背包挑一份可分享的材料（純函式、可測）。
///
/// 規則（確定性、可重現）：
/// - 忽略 Air（0）與數量 `< SHARE_MIN_STOCK`（有餘裕才給）；
/// - 在合格者中挑數量最多者（她最投入、最不缺的那種）；同量時 `block_id` 小者優先（穩定排序）；
/// - 分享份數 `= min(SHARE_CAP, 餘量)`（實務上恆為 `SHARE_CAP`，`min` 僅作安全夾值）。
///
/// 無任何合格材料 → `None`（呼叫端當作這次沒得分享，安靜跳過）。
pub fn pick_share(bag: &HashMap<u8, u32>) -> Option<(u8, u32)> {
    bag.iter()
        .filter(|&(&b, &q)| b != 0 && q >= SHARE_MIN_STOCK)
        .max_by(|&(ba, qa), &(bb, qb)| qa.cmp(qb).then_with(|| bb.cmp(ba)))
        .map(|(&b, &q)| (b, SHARE_CAP.min(q)))
}

/// 截斷輔助：保留至多 `SHARE_MAX_CHARS` 個字元（依字元非位元組，繁中安全）。
fn truncate_chars(s: &str) -> String {
    s.chars().take(SHARE_MAX_CHARS).collect()
}

/// 主人分享時說的台詞（確定性、零 LLM，帶訪客名＋物名，≤40 字剛好在泡泡框內）。
pub fn share_say_line(visitor_name: &str, item_name: &str, pick: usize) -> String {
    let pool: &[&str] = &[
        "{v}，這些{i}你帶回去用吧！",
        "{v}，我採多了{i}，分你一點～",
        "{v}，難得你來，這{i}拿去！",
        "{v}，這{i}留給你，別跟我客氣。",
    ];
    truncate_chars(&pool[pick % pool.len()].replace("{v}", visitor_name).replace("{i}", item_name))
}

/// 主人記憶：我把親手採到的東西分給了來訪的老朋友（讓日記端能昇華成一則慷慨的記憶）。
pub fn share_memory_line_host(visitor_name: &str, item_name: &str) -> String {
    truncate_chars(&format!("把我採到的{item_name}分了些給來看我的{visitor_name}"))
}

/// 訪客記憶：老朋友把他採到的東西分給了我（記在訪客名下，日後可被回想／日記化）。
pub fn share_memory_line_visitor(host_name: &str, item_name: &str) -> String {
    truncate_chars(&format!("{host_name}把親手採到的{item_name}分了些給我，好暖心"))
}

/// Feed 動態文案（第三人稱、附在 actor＝主人名後面），讓離線玩家回來翻動態也知道
/// 居民彼此在照應。主人名由呼叫端作為 actor 傳入 `append_feed`，這裡不重複帶。
pub fn share_feed_line(visitor_name: &str, item_name: &str, qty: u32) -> String {
    // Feed 摘要不進泡泡框，長度寬鬆，但仍保守截一下防極端物名。
    format!("把採到的{item_name}分了 {qty} 份給老朋友{visitor_name}")
        .chars()
        .take(SHARE_MAX_CHARS + 10)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bag(pairs: &[(u8, u32)]) -> HashMap<u8, u32> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn should_share_only_friend_no_other_scene() {
        // 老朋友、沒別齣戲、機率過門檻 → 觸發。
        assert!(should_share(BondTier::Friend, false, 0.0));
        // 有別齣戲 → 不觸發（一訪一戲）。
        assert!(!should_share(BondTier::Friend, true, 0.0));
        // 非老朋友 → 不觸發。
        assert!(!should_share(BondTier::Acquaintance, false, 0.0));
        assert!(!should_share(BondTier::Stranger, false, 0.0));
        // 機率沒過門檻 → 不觸發。
        assert!(!should_share(BondTier::Friend, false, SHARE_CHANCE + 0.01));
    }

    #[test]
    fn pick_share_takes_most_abundant_surplus() {
        // 木頭(5) 8 份最多且過門檻 → 挑木頭、勻 SHARE_CAP 份。
        let b = bag(&[(5, 8), (3, 6), (14, 2)]);
        assert_eq!(pick_share(&b), Some((5, SHARE_CAP)));
    }

    #[test]
    fn pick_share_none_when_no_surplus() {
        // 全部低於門檻 → 沒餘裕可分享。
        let b = bag(&[(5, SHARE_MIN_STOCK - 1), (3, 1)]);
        assert_eq!(pick_share(&b), None);
        // 空背包 → None。
        assert_eq!(pick_share(&HashMap::new()), None);
    }

    #[test]
    fn pick_share_ignores_air_and_ties_break_low_id() {
        // Air(0) 再多也不算數。
        let b = bag(&[(0, 100), (5, 6), (3, 6)]);
        // 5 與 3 同量 6，穩定排序取 block_id 小者（3）。
        assert_eq!(pick_share(&b), Some((3, SHARE_CAP)));
    }

    #[test]
    fn pick_share_qty_never_exceeds_stock() {
        // 餘量恰等於門檻時，分享份數不超過 SHARE_CAP，也不超過餘量。
        let b = bag(&[(5, SHARE_MIN_STOCK)]);
        let (_, qty) = pick_share(&b).unwrap();
        assert!(qty <= SHARE_CAP);
        assert!(qty <= SHARE_MIN_STOCK);
        assert!(qty >= 1);
    }

    #[test]
    fn say_line_non_empty_within_bubble_has_names() {
        for pick in 0..6 {
            let line = share_say_line("露娜", "木頭", pick);
            assert!(!line.is_empty());
            assert!(line.chars().count() <= SHARE_MAX_CHARS, "台詞不得破泡泡框");
            assert!(line.contains("露娜"), "台詞應含訪客名");
            assert!(line.contains("木頭"), "台詞應含物名");
        }
    }

    #[test]
    fn say_line_varies_by_pick() {
        let a = share_say_line("諾娃", "石頭", 0);
        let b = share_say_line("諾娃", "石頭", 1);
        assert_ne!(a, b, "不同 pick 應輪替不同措辭");
    }

    #[test]
    fn memory_lines_carry_both_names_and_item() {
        let h = share_memory_line_host("露娜", "木頭");
        assert!(h.contains("露娜") && h.contains("木頭"));
        assert!(h.chars().count() <= SHARE_MAX_CHARS);
        let v = share_memory_line_visitor("諾娃", "玻璃");
        assert!(v.contains("諾娃") && v.contains("玻璃"));
        assert!(v.chars().count() <= SHARE_MAX_CHARS);
    }

    #[test]
    fn feed_line_contains_item_qty_and_visitor() {
        let f = share_feed_line("露娜", "木頭", 2);
        assert!(f.contains("木頭"));
        assert!(f.contains("露娜"));
        assert!(f.contains('2'));
    }
}
