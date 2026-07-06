//! 乙太方界·居民回饋糧倉 v1（自主提案切片）。
//!
//! **真缺口**：共用糧倉 v1（`voxel_chest` 的 `nearest_food_chest`）讓餓著的居民能走去玩家存了
//! 食物的箱子借一份——但這條線至今**只有單向**：居民只會向箱子**取**，從沒有「居民也往箱子裡
//! **存**」的另一半。玩家囤糧食給居民靠，居民卻從沒為你的儲藏添過一份東西；箱子系統對居民
//! 而言，至今只是一間單方面領用的免費商店，不是真正互助的儲藏。
//!
//! **本刀補上**：居民互贈·分享採集所得（748，`voxel_share::pick_share`）已經會判斷居民手上
//! 「採到最多、有餘裕」的材料——原本只用在老朋友來訪時分給訪客。本刀把同一套「有沒有餘裕」
//! 的判斷接到另一個場景：居民閒晃途中若剛好靠近一口**你已經用過**的箱子，偶爾會順手把那份
//! 餘裕材料存進去，回饋你的基地。第一次讓「箱子」成為雙向的互助循環——玩家囤給居民借，
//! 居民也會回頭往裡添，正中 PLAN_ETHERVOX「玩家遊玩＋AI 生活交織」的精神：你的儲藏，
//! 因居民真的活在這個世界而慢慢變得更豐盛。
//!
//! **與既有系統的定位區隔**：
//! - 與 748 分享（居民↔居民、老朋友來訪觸發）：本刀是居民↔玩家、閒晃途中偶然觸發，
//!   兩者觸發時機互斥（分享用在到訪戲碼、本刀用在日常閒晃），複用同一份「挑餘料」純函式
//!   （單一事實來源，不重寫一份判斷邏輯），但各自獨立的冷卻與機率。
//! - 與共用糧倉 v1（居民向箱子取食物）：本刀是相反方向（居民往箱子存），存的是**任意**採集
//!   材料而非限定食物，兩者共用同一個 `ChestStore`，互不干擾。
//!
//! **純邏輯層**：`should_contribute` 是否該存（純函式、確定性）、`contribute_bubble` 泡泡台詞、
//! `contribute_feed_line` 動態牆播報，皆零 LLM、零鎖、零 IO。實際扣居民背包／存進箱子／持久化
//! 全在 `voxel_ws.rs`（鎖序：`res_inv` 讀→`chest` 讀決定去留在同一 tick 完成；真正轉移發生在
//! 下一輪鎖外處理，`deltas` 讀→`res_inv` 寫→`chest` 寫，各自短取即釋、不巢狀，守死鎖鐵律）。
//!
//! **成本 / 濫用防護**：句子全走固定模板，不夾帶玩家輸入；存不存純由伺服器內部（採集背包
//! 存量＋箱子已知位置＋機率）決定，玩家無法自報或催發；長冷卻（[`CONTRIBUTE_COOLDOWN_SECS`]）
//! 天然防洗版；零 migration（`voxel_chest` 既有 append-only jsonl 原封複用）、零協議破壞
//! （沿用既有 `block`/`inv_update` 廣播管線）、零新美術。

use crate::voxel_share as vshare;

/// 每個閒晃 tick 觸發存料的機率（低頻，讓「回饋」偶爾發生、有份量而非常態）。
pub const CONTRIBUTE_CHANCE_PER_TICK: f32 = 0.02;

/// 同一位居民存料的冷卻（秒）。設得長，避免同一位居民反覆繞著箱子塞東西洗版。
pub const CONTRIBUTE_COOLDOWN_SECS: f32 = 300.0;

/// 尋找「已知箱子」的搜尋半徑（方塊，XZ 平面），與既有共用糧倉尋箱半徑同量級。
pub const CONTRIBUTE_RADIUS: i32 = 16;

/// 泡泡字元上限（與其餘泡泡框上限一致，超出截斷不破框）。
pub const CONTRIBUTE_SAY_MAX_CHARS: usize = 40;

/// Feed 事件類型字串（面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "回饋糧倉";

/// 從居民的採集背包挑一份可存回箱子的餘裕材料。直接複用 748 分享既有的「有沒有餘裕」判斷
/// （單一事實來源：夠多才給、給最多的那種、每次只勻一小份），本刀只是把同一份餘料判斷
/// 接到「存進箱子」而非「分給訪客」這個新場景。
pub fn pick_contribution(bag: &std::collections::HashMap<u8, u32>) -> Option<(u8, u32)> {
    vshare::pick_share(bag)
}

/// 是否該在這個閒晃 tick 把餘裕材料存進箱子（純函式、確定性）。
/// 冷卻由呼叫端的計時欄位外層把關（比照居民教你獨門配方 v1 的慣例，不在此函式重複判斷）；
/// 這裡只把「有沒有餘料可存」＋「附近有沒有一口已知的箱子」＋「機率門檻」三道關卡收攏。
pub fn should_contribute(has_surplus: bool, near_known_chest: bool, roll: f32, chance: f32) -> bool {
    has_surplus && near_known_chest && roll < chance
}

/// 截斷輔助：保留至多 [`CONTRIBUTE_SAY_MAX_CHARS`] 個字元（依字元非位元組，繁中安全）。
fn truncate_chars(s: &str) -> String {
    s.chars().take(CONTRIBUTE_SAY_MAX_CHARS).collect()
}

/// 存料當下的泡泡台詞，依 `pick` 在幾組模板間確定性輪替。整句以字元截到上限內，永不破框。
pub fn contribute_bubble(item_name: &str, qty: u32, pick: usize) -> String {
    const T: [&str; 3] = [
        "順手把多的{i}存進箱子裡，{q}份留給大家用。",
        "這些{i}我用不完，先存{q}份進箱子吧。",
        "箱子裡的東西也該添一添——存了{q}份{i}進去。",
    ];
    let line = T[pick % T.len()]
        .replace("{i}", item_name)
        .replace("{q}", &qty.to_string());
    truncate_chars(&line)
}

/// 動態牆播報（面向玩家、不含記憶原文，純模板拼裝）。
pub fn contribute_feed_line(resident_name: &str, item_name: &str, qty: u32) -> String {
    format!("{resident_name}把{qty}份{item_name}存進了村裡的箱子")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn pick_contribution_delegates_to_share() {
        let mut bag = HashMap::new();
        bag.insert(5u8, 10u32); // 木頭，有餘裕
        assert_eq!(pick_contribution(&bag), vshare::pick_share(&bag));
        assert!(pick_contribution(&bag).is_some());
    }

    #[test]
    fn pick_contribution_none_when_no_surplus() {
        let bag = HashMap::new();
        assert_eq!(pick_contribution(&bag), None);
    }

    #[test]
    fn should_contribute_requires_all_three_gates() {
        assert!(should_contribute(true, true, 0.0, CONTRIBUTE_CHANCE_PER_TICK));
        assert!(!should_contribute(false, true, 0.0, CONTRIBUTE_CHANCE_PER_TICK));
        assert!(!should_contribute(true, false, 0.0, CONTRIBUTE_CHANCE_PER_TICK));
        assert!(!should_contribute(true, true, 1.0, CONTRIBUTE_CHANCE_PER_TICK));
    }

    #[test]
    fn contribute_bubble_contains_item_and_qty_and_fits_frame() {
        for pick in 0..3 {
            let line = contribute_bubble("木頭", 2, pick);
            assert!(line.contains('木'));
            assert!(line.contains('2'));
            assert!(line.chars().count() <= CONTRIBUTE_SAY_MAX_CHARS);
        }
    }

    #[test]
    fn contribute_bubble_rotates_deterministically() {
        let a = contribute_bubble("石頭", 2, 0);
        let b = contribute_bubble("石頭", 2, 1);
        assert_ne!(a, b);
        // 同 pick 同輸入 → 同輸出（確定性）。
        assert_eq!(a, contribute_bubble("石頭", 2, 0));
    }

    #[test]
    fn contribute_bubble_never_breaks_frame_even_with_long_item_name() {
        let long_name = "超級無敵長的材料名字測試用超級無敵長的材料名字測試用超級無敵長的材料名字測試用";
        let line = contribute_bubble(long_name, 999, 0);
        assert!(line.chars().count() <= CONTRIBUTE_SAY_MAX_CHARS);
    }

    #[test]
    fn contribute_feed_line_mentions_resident_item_and_qty() {
        let line = contribute_feed_line("露娜", "石頭", 2);
        assert!(line.contains("露娜"));
        assert!(line.contains('石'));
        assert!(line.contains('2'));
    }
}
