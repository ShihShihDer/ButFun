//! 新手見面禮·故鄉的起手禮（ROADMAP 444）的純邏輯。
//!
//! 全新玩家**第一次登入**時，故鄉送上一份一次性「見面禮」：一把鎬子（採礦更快）、
//! 一束木材（第一份合成素材）、外加一小撮迎新乙太——讓剛落腳的新人不必空著手摸索半天，
//! 一進場就能踏進「採集→合成→變強」的核心循環。冪等性（只領一次）由 `welcome_kit_store`
//! 的 claim 把關；本模組只負責「**禮包內容**」與「**把禮包套進一個背包**」這兩件純邏輯，
//! 確定性、可測、零 IO。
//!
//! 經濟紀律：禮包**一次性、固定、極小**。物品都是世界本來就採得到的東西（賣給商人也只是
//! 從商人金庫提領、非憑空鑄幣），迎新乙太比照連日歸鄉（`visit_streak`，每日 3）量級的小額，
//! 對既有閉合乙太迴圈擾動可忽略。

use crate::inventory::{Inventory, ItemKind};

/// 見面禮固定內容（物品）：採礦工具打底 + 第一份合成素材。
/// 順序穩定（供前端逐項顯示時不跳動）；數值小而克制，是「起步推一把」不是「躺贏」。
pub const KIT_ITEMS: &[(ItemKind, u32)] = &[
    // 一把鎬子：身上有它採礦更快（`tools` 倍率），新人省去「先攢素材合一把」的冷啟動。
    (ItemKind::Pickaxe, 1),
    // 一束木材：最基礎的合成素材，讓新人馬上有東西可合、可賣、可堆。
    (ItemKind::Wood, 5),
];

/// 見面禮附帶的迎新乙太（小額一次性，比照連日歸鄉日獎量級）。
/// 給新人「第一桶乙太」的一角，能立刻去商人那買第一包種子／擴一格地。
pub const KIT_ETHER: u32 = 20;

/// 一筆被授予的物品（item 的 wire 字串 + 數量），供協議單播給前端逐項顯示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedItem {
    pub item: ItemKind,
    pub qty: u32,
}

/// 把見面禮套進玩家背包：逐項加入，回傳**實際加入**的每項（背包夾上限後可能少於名目量；
/// 全新玩家背包是空的，正常情況下會全額加入）。`qty == 0` 或實際沒加進去（已滿）的項目
/// 不列入回傳，前端就不會顯示「+0」。乙太不在此處理（由呼叫端在玩家身上加 [`KIT_ETHER`]，
/// 隨既有持久化自然存檔），保持本函式只動背包、好測。
pub fn apply_kit(inv: &mut Inventory) -> Vec<GrantedItem> {
    let mut granted = Vec::new();
    for &(item, qty) in KIT_ITEMS {
        let added = inv.add(item, qty);
        if added > 0 {
            granted.push(GrantedItem { item, qty: added });
        }
    }
    granted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_kit_grants_all_items_to_empty_inventory() {
        let mut inv = Inventory::new();
        let granted = apply_kit(&mut inv);
        // 兩項禮物都全額加入空背包。
        assert_eq!(granted.len(), KIT_ITEMS.len());
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
        assert_eq!(inv.count(ItemKind::Wood), 5);
        // 回傳的數量與背包實際一致。
        for g in &granted {
            assert_eq!(inv.count(g.item), g.qty);
        }
    }

    #[test]
    fn apply_kit_is_additive_over_existing_items() {
        // 已有東西的背包（理論上見面禮只給空背包的新人，但純函式仍應乾淨疊加）。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 2);
        apply_kit(&mut inv);
        assert_eq!(inv.count(ItemKind::Wood), 2 + 5);
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn kit_amounts_stay_small_and_economy_safe() {
        // 經濟紀律守門：見面禮固定極小，別在無意間被改大成躺贏禮包。
        assert!(KIT_ETHER <= 30, "迎新乙太應維持小額");
        let total_items: u32 = KIT_ITEMS.iter().map(|&(_, q)| q).sum();
        assert!(total_items <= 12, "禮包物品總量應維持克制");
    }
}
